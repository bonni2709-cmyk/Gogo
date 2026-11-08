//! Capability-protected loopback media gateway. Native players never receive CDN URLs.
use crate::{MediaStream, PreparedVideo, SearchClient};
use anyhow::{bail, Result};
use axum::{body::Body, extract::{Path, State}, http::{header, HeaderMap, Method, StatusCode}, response::Response, routing::get, Router};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::Semaphore};
use tokio_util::sync::CancellationToken;

/// Drop closes the listener and cancels active requests. Intentionally no Debug.
pub struct PlaybackSession {
    address: SocketAddr,
    capability: String,
    metadata: PreparedVideo,
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl PlaybackSession {
    pub async fn start(client: SearchClient, stream: MediaStream) -> Result<Self> {
        let mut random = [0u8; 32];
        getrandom::getrandom(&mut random).map_err(|_| anyhow::anyhow!("secure random generator unavailable"))?;
        let capability: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let cancel = CancellationToken::new();
        let state = Arc::new(Gateway {
            client, metadata: stream.metadata.clone(), stream, address,
            capability: capability.clone(), cancel: cancel.clone(), slots: Arc::new(Semaphore::new(4)),
        });
        let metadata = state.metadata.clone();
        let app = Router::new().route("/{token}/media", get(media)).with_state(state);
        let task = tokio::spawn(async move { let _ = axum::serve(listener, app).await; });
        Ok(Self { address, capability, metadata, cancel, task })
    }

    pub fn metadata(&self) -> &PreparedVideo { &self.metadata }

    /// Only this local endpoint may be passed to the native playback engine.
    pub fn local_url(&self) -> String { format!("http://{}/{}/media", self.address, self.capability) }
}
impl Drop for PlaybackSession {
    fn drop(&mut self) { self.cancel.cancel(); self.task.abort(); }
}

struct Gateway {
    client: SearchClient,
    stream: MediaStream,
    metadata: PreparedVideo,
    address: SocketAddr,
    capability: String,
    cancel: CancellationToken,
    slots: Arc<Semaphore>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ByteRange {
    From { start: u64, end: Option<u64> },
    Suffix(u64),
}

fn decimal(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) { return None; }
    value.parse().ok()
}

fn parse_range(range: &str) -> Option<ByteRange> {
    if range.len() > 80 { return None; }
    let (start, end) = range.strip_prefix("bytes=")?.split_once('-')?;
    if start.is_empty() { return decimal(end).filter(|n| *n > 0).map(ByteRange::Suffix); }
    let start = decimal(start)?;
    let end = if end.is_empty() { None } else { Some(decimal(end)?) };
    if end.is_some_and(|end| end < start) { return None; }
    Some(ByteRange::From { start, end })
}

impl ByteRange {
    fn resolve(self, total: u64) -> Option<(u64, u64)> {
        let last = total.checked_sub(1)?;
        match self {
            Self::Suffix(length) => Some((total.saturating_sub(length), last)),
            Self::From { start, end } if start < total => Some((start, end.unwrap_or(last).min(last))),
            _ => None,
        }
    }
}

fn response_headers(status: StatusCode, headers: &HeaderMap, requested: Option<ByteRange>) -> Result<(HeaderMap, u64)> {
    let expected_status = if requested.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
    if status != expected_status { bail!("unexpected CDN status"); }
    if headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).and_then(|s| s.split(';').next()) != Some("video/mp4") { bail!("CDN is not MP4"); }
    if headers.get(header::CONTENT_ENCODING).is_some_and(|v| v.as_bytes() != b"identity") { bail!("encoded media response is not supported"); }
    let length = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(decimal).filter(|n| *n > 0).ok_or_else(|| anyhow::anyhow!("CDN content length missing"))?;
    let range_count = headers.get_all(header::CONTENT_RANGE).iter().count();
    if let Some(requested) = requested {
        if range_count != 1 { bail!("ambiguous CDN range"); }
        let range = headers.get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("bytes ")).ok_or_else(|| anyhow::anyhow!("invalid CDN range"))?;
        let (interval, total) = range.split_once('/').ok_or_else(|| anyhow::anyhow!("invalid CDN range"))?;
        let (start, end) = interval.split_once('-').ok_or_else(|| anyhow::anyhow!("invalid CDN range"))?;
        let numbers = (decimal(start), decimal(end), decimal(total));
        let (Some(start), Some(end), Some(total)) = numbers else { bail!("invalid CDN range numbers"); };
        if requested.resolve(total) != Some((start, end)) || end.checked_sub(start).and_then(|n| n.checked_add(1)) != Some(length) { bail!("CDN returned a different or inconsistent byte range"); }
    } else if range_count != 0 {
        bail!("unexpected partial range on a full response");
    }
    let mut output = HeaderMap::new();
    for name in [header::CONTENT_TYPE, header::CONTENT_LENGTH, header::CONTENT_RANGE] {
        if let Some(value) = headers.get(&name) { output.insert(name, value.clone()); }
    }
    output.insert(header::ACCEPT_RANGES, "bytes".parse()?);
    output.insert(header::CACHE_CONTROL, "no-store".parse()?);
    output.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse()?);
    Ok((output, length))
}

async fn media(State(state): State<Arc<Gateway>>, Path(token): Path<String>, method: Method, headers: HeaderMap) -> Response {
    // Host check prevents DNS rebinding to this listener. No CORS permission is emitted.
    if token != state.capability || headers.get(header::HOST).and_then(|h| h.to_str().ok()) != Some(state.address.to_string().as_str()) { return empty(StatusCode::NOT_FOUND); }
    let Ok(permit) = state.slots.clone().try_acquire_owned() else { return empty(StatusCode::TOO_MANY_REQUESTS); };
    let range = match headers.get(header::RANGE) {
        Some(value) => match value.to_str() { Ok(value) if parse_range(value).is_some() => Some(value), _ => return empty(StatusCode::RANGE_NOT_SATISFIABLE) },
        None => None,
    };
    let upstream = tokio::select! {
        _ = state.cancel.cancelled() => return empty(StatusCode::GONE),
        result = state.client.open_media(&state.stream, range, method == Method::HEAD) => result,
    };
    let Ok(upstream) = upstream else { return empty(StatusCode::BAD_GATEWAY); };
    let status = upstream.status();
    let Ok((output, length)) = response_headers(status, upstream.headers(), range.and_then(parse_range)) else { return empty(StatusCode::BAD_GATEWAY); };
    let body = if method == Method::HEAD { Body::empty() } else {
        let cancel = state.cancel.clone();
        let client = state.client.clone();
        let stream = futures_util::stream::try_unfold((upstream, length, permit, cancel, client), |(mut response, remaining, permit, cancel, client)| async move {
            if remaining == 0 { return Ok::<_, std::io::Error>(None); }
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(std::io::Error::other("playback session closed")),
                chunk = tokio::time::timeout(Duration::from_secs(20), response.chunk()) => chunk,
            };
            match chunk {
                Ok(Ok(Some(bytes))) if !bytes.is_empty() && bytes.len() as u64 <= remaining => {
                    let remaining = remaining - bytes.len() as u64;
                    Ok(Some((bytes, (response, remaining, permit, cancel, client))))
                }
                Ok(Err(_)) | Ok(Ok(None)) => {
                    // Body read failures can be categorized as decode errors by reqwest;
                    // an EOF with remaining bytes is also a transport interruption.
                    client.note_network_failure();
                    Err(std::io::Error::other("media transfer interrupted"))
                }
                Err(_) => {
                    client.note_network_failure();
                    Err(std::io::Error::other("media transfer stalled"))
                }
                _ => Err(std::io::Error::other("media transfer interrupted")),
            }
        });
        Body::from_stream(stream)
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = output;
    response
}
fn empty(status: StatusCode) -> Response { Response::builder().status(status).body(Body::empty()).unwrap() }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_single_bounded_byte_range() {
        for value in ["bytes=0-", "bytes=0-99", "bytes=-1024"] { assert!(parse_range(value).is_some()); }
        for value in ["bytes=", "bytes=-", "bytes=-0", "bytes=10-9", "bytes=0-1,4-9", "bytes=18446744073709551616-", "other=0-1", "bytes=+0-1", "bytes=0-+1"] { assert!(parse_range(value).is_none()); }
    }
    #[test]
    fn rejects_bad_media_headers_and_copies_no_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "10".parse().unwrap());
        headers.insert(header::CONTENT_RANGE, "bytes 0-9/100".parse().unwrap());
        headers.insert(header::SET_COOKIE, "private=token".parse().unwrap());
        let (output, length) = response_headers(StatusCode::PARTIAL_CONTENT, &headers, parse_range("bytes=0-9")).unwrap();
        assert_eq!(length, 10);
        assert!(!output.contains_key(header::SET_COOKIE));
        assert!(response_headers(StatusCode::OK, &headers, parse_range("bytes=0-9")).is_err());
        headers.insert(header::CONTENT_RANGE, "bytes 0-10/100".parse().unwrap());
        assert!(response_headers(StatusCode::PARTIAL_CONTENT, &headers, parse_range("bytes=0-9")).is_err());
    }
    #[test]
    fn resolves_open_bounded_and_suffix_ranges_without_overflow() {
        for (value, expected) in [
            ("bytes=10-19", Some((10, 19))), ("bytes=30-", Some((30, 35))),
            ("bytes=-5", Some((31, 35))), ("bytes=-100", Some((0, 35))),
            ("bytes=30-999", Some((30, 35))), ("bytes=36-", None),
            ("bytes=18446744073709551615-", None),
        ] {
            assert_eq!(parse_range(value).unwrap().resolve(36), expected, "{value}");
        }
        assert_eq!(parse_range("bytes=0-").unwrap().resolve(0), None);
    }

    #[test]
    fn rejects_shifted_ambiguous_encoded_and_unrequested_ranges() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "10".parse().unwrap());
        headers.insert(header::CONTENT_RANGE, "bytes 0-9/100".parse().unwrap());
        assert!(response_headers(StatusCode::PARTIAL_CONTENT, &headers, parse_range("bytes=10-19")).is_err());
        assert!(response_headers(StatusCode::OK, &headers, None).is_err());
        headers.insert(header::CONTENT_ENCODING, "gzip".parse().unwrap());
        assert!(response_headers(StatusCode::PARTIAL_CONTENT, &headers, parse_range("bytes=0-9")).is_err());
        headers.remove(header::CONTENT_ENCODING);
        headers.append(header::CONTENT_RANGE, "bytes 0-9/100".parse().unwrap());
        assert!(response_headers(StatusCode::PARTIAL_CONTENT, &headers, parse_range("bytes=0-9")).is_err());
    }

    #[tokio::test]
    async fn enforces_capability_host_and_range_before_upstream() {
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = proxy.local_addr().unwrap();
        drop(proxy);
        let client = SearchClient::new(address).unwrap();
        let session = PlaybackSession::start(client, crate::media::fixture_stream()).await.unwrap();
        let http = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(3)).build().unwrap();
        let wrong = format!("http://{}/wrong/media", session.address);
        assert_eq!(http.get(wrong).send().await.unwrap().status(), StatusCode::NOT_FOUND);
        assert_eq!(http.get(session.local_url()).header(header::HOST, "evil.test").send().await.unwrap().status(), StatusCode::NOT_FOUND);
        assert_eq!(http.get(session.local_url()).header(header::RANGE, "bytes=0-1,3-4").send().await.unwrap().status(), StatusCode::RANGE_NOT_SATISFIABLE);
        let response = http.get(session.local_url()).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        assert!(!response.text().await.unwrap().contains("must-not-leak"));
    }

    #[tokio::test]
    async fn dropping_session_releases_listener() {
        let client = SearchClient::new("127.0.0.1:9".parse().unwrap()).unwrap();
        let session = PlaybackSession::start(client, crate::media::fixture_stream()).await.unwrap();
        let address = session.address;
        assert!(tokio::net::TcpStream::connect(address).await.is_ok());
        drop(session);
        tokio::time::timeout(Duration::from_secs(2), async {
            while tokio::net::TcpStream::connect(address).await.is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
    }

}


#[cfg(test)]
#[path = "playback_transport_tests.rs"]
mod transport_tests;
