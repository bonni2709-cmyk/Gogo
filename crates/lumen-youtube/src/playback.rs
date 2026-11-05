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

fn valid_range(range: &str) -> bool {
    if range.len() > 80 { return false; }
    let Some(value) = range.strip_prefix("bytes=") else { return false; };
    let Some((start, end)) = value.split_once('-') else { return false; };
    if start.is_empty() { return end.parse::<u64>().is_ok_and(|n| n > 0); }
    let Ok(start) = start.parse::<u64>() else { return false; };
    end.is_empty() || end.parse::<u64>().is_ok_and(|end| end >= start)
}

fn response_headers(status: StatusCode, headers: &HeaderMap, ranged: bool) -> Result<(HeaderMap, u64)> {
    if (ranged && status != StatusCode::PARTIAL_CONTENT) || (!ranged && status != StatusCode::OK) { bail!("unexpected CDN status"); }
    if headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).and_then(|s| s.split(';').next()) != Some("video/mp4") { bail!("CDN is not MP4"); }
    let length = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0).ok_or_else(|| anyhow::anyhow!("CDN content length missing"))?;
    if ranged {
        let range = headers.get(header::CONTENT_RANGE).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("bytes ")).ok_or_else(|| anyhow::anyhow!("invalid CDN range"))?;
        let (interval, total) = range.split_once('/').ok_or_else(|| anyhow::anyhow!("invalid CDN range"))?;
        let (start, end) = interval.split_once('-').ok_or_else(|| anyhow::anyhow!("invalid CDN range"))?;
        let (start, end, total) = (start.parse::<u64>()?, end.parse::<u64>()?, total.parse::<u64>()?);
        if end < start || end >= total || end.checked_sub(start).and_then(|n| n.checked_add(1)) != Some(length) { bail!("inconsistent CDN range"); }
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
        Some(value) => match value.to_str() { Ok(value) if valid_range(value) => Some(value), _ => return empty(StatusCode::RANGE_NOT_SATISFIABLE) },
        None => None,
    };
    let upstream = tokio::select! {
        _ = state.cancel.cancelled() => return empty(StatusCode::GONE),
        result = state.client.open_media(&state.stream, range, method == Method::HEAD) => result,
    };
    let Ok(upstream) = upstream else { return empty(StatusCode::BAD_GATEWAY); };
    let status = upstream.status();
    let Ok((output, length)) = response_headers(status, upstream.headers(), range.is_some()) else { return empty(StatusCode::BAD_GATEWAY); };
    let body = if method == Method::HEAD { Body::empty() } else {
        let cancel = state.cancel.clone();
        let stream = futures_util::stream::try_unfold((upstream, length, permit, cancel), |(mut response, remaining, permit, cancel)| async move {
            if remaining == 0 { return Ok::<_, std::io::Error>(None); }
            let chunk = tokio::select! {
                _ = cancel.cancelled() => return Err(std::io::Error::other("playback session closed")),
                chunk = tokio::time::timeout(Duration::from_secs(20), response.chunk()) => chunk,
            };
            match chunk {
                Ok(Ok(Some(bytes))) if !bytes.is_empty() && bytes.len() as u64 <= remaining => {
                    let remaining = remaining - bytes.len() as u64;
                    Ok(Some((bytes, (response, remaining, permit, cancel))))
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
        for value in ["bytes=0-", "bytes=0-99", "bytes=-1024"] { assert!(valid_range(value)); }
        for value in ["bytes=", "bytes=-", "bytes=-0", "bytes=10-9", "bytes=0-1,4-9", "bytes=18446744073709551616-", "other=0-1"] { assert!(!valid_range(value)); }
    }
    #[test]
    fn rejects_bad_media_headers_and_copies_no_tokens() {
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, "video/mp4".parse().unwrap());
        headers.insert(header::CONTENT_LENGTH, "10".parse().unwrap());
        headers.insert(header::CONTENT_RANGE, "bytes 0-9/100".parse().unwrap());
        headers.insert(header::SET_COOKIE, "private=token".parse().unwrap());
        let (output, length) = response_headers(StatusCode::PARTIAL_CONTENT, &headers, true).unwrap();
        assert_eq!(length, 10);
        assert!(!output.contains_key(header::SET_COOKIE));
        assert!(response_headers(StatusCode::OK, &headers, true).is_err());
        headers.insert(header::CONTENT_RANGE, "bytes 0-10/100".parse().unwrap());
        assert!(response_headers(StatusCode::PARTIAL_CONTENT, &headers, true).is_err());
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
