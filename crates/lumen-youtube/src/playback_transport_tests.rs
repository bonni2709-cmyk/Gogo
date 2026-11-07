//! Integration tests of production gateway + reqwest via a test CONNECT proxy and
//! a certificate-verified local TLS origin. No production DNS/allowlist exceptions.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{io::{AsyncRead, AsyncReadExt, AsyncWriteExt}, net::TcpStream, task::JoinSet, time::timeout};
use tokio_rustls::{rustls::{self, pki_types::PrivatePkcs8KeyDer}, TlsAcceptor};

const CONTENT: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";

#[derive(Clone, Copy)]
enum Mode { Normal, WrongRange, Redirect, Truncated, Stalled }

struct Fixture {
    client: SearchClient,
    connect_count: Arc<AtomicUsize>,
    origin_count: Arc<AtomicUsize>,
    origin: tokio::task::JoinHandle<()>,
    proxy: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) { self.origin.abort(); self.proxy.abort(); }
}

async fn read_head(socket: &mut (impl AsyncRead + Unpin)) -> anyhow::Result<String> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        if bytes.len() >= 8192 { bail!("test header too large"); }
        bytes.push(socket.read_u8().await?);
    }
    Ok(String::from_utf8(bytes)?)
}

fn origin_range(header: Option<&str>) -> (usize, usize) {
    // Independent tiny fixture implementation, not production ByteRange::resolve.
    let Some(value) = header else { return (0, CONTENT.len() - 1); };
    let (start, end) = value.strip_prefix("bytes=").unwrap().split_once('-').unwrap();
    if start.is_empty() {
        let length: usize = end.parse().unwrap();
        return (CONTENT.len().saturating_sub(length), CONTENT.len() - 1);
    }
    let start: usize = start.parse().unwrap();
    let end = if end.is_empty() { CONTENT.len() - 1 } else { end.parse::<usize>().unwrap().min(CONTENT.len() - 1) };
    (start, end)
}

impl Fixture {
    async fn new(mode: Mode, trust_certificate: bool) -> Self {
        let certificate = rcgen::generate_simple_self_signed(vec!["rr1.googlevideo.com".into()]).unwrap();
        let der = certificate.cert.der().clone();
        let key = PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der());
        let config = rustls::ServerConfig::builder().with_no_client_auth()
            .with_single_cert(vec![der.clone()], key.into()).unwrap();
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_address = listener.local_addr().unwrap();
        let origin_count = Arc::new(AtomicUsize::new(0));
        let count = origin_count.clone();
        let origin = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (socket, _) = accepted.unwrap();
                        let acceptor = acceptor.clone();
                        let count = count.clone();
                        tasks.spawn(async move {
                            let Ok(Ok(mut tls)) = timeout(Duration::from_secs(3), acceptor.accept(socket)).await else { return; };
                            let Ok(Ok(request)) = timeout(Duration::from_secs(3), read_head(&mut tls)).await else { return; };
                            count.fetch_add(1, Ordering::SeqCst);
                            let first = request.lines().next().unwrap();
                            assert!(first.starts_with("GET /videoplayback?") || first.starts_with("HEAD /videoplayback?"));
                            let head = first.starts_with("HEAD ");
                            let range = request.lines().find_map(|line| {
                                let (name, value) = line.split_once(':')?;
                                name.eq_ignore_ascii_case("range").then(|| value.trim())
                            });
                            if matches!(mode, Mode::Redirect) {
                                let _ = tls.write_all(b"HTTP/1.1 302 Found\r\nLocation: https://evil.test/token\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                                let _ = tls.shutdown().await;
                                return;
                            }
                            let (start, end) = if matches!(mode, Mode::WrongRange) && range.is_some() {
                                (0, 9) // Internally valid, but wrong for bytes=10-19.
                            } else { origin_range(range) };
                            let length = end - start + 1;
                            let status = if range.is_some() { "206 Partial Content" } else { "200 OK" };
                            let partial = if range.is_some() { format!("Content-Range: bytes {start}-{end}/{}\r\n", CONTENT.len()) } else { String::new() };
                            let headers = format!("HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\nContent-Length: {length}\r\n{partial}Set-Cookie: private=must-not-leak\r\nX-CDN-Secret: must-not-leak\r\nConnection: close\r\n\r\n");
                            if tls.write_all(headers.as_bytes()).await.is_err() { return; }
                            if !head {
                                if matches!(mode, Mode::Stalled) {
                                    let _ = tls.write_all(&CONTENT[start..start + 1]).await;
                                    let _ = tls.flush().await;
                                    // Await close by the gateway; no arbitrary sleeps in this server.
                                    let mut byte = [0];
                                    let _ = tls.read(&mut byte).await;
                                    return;
                                }
                                let body = if matches!(mode, Mode::Truncated) { &CONTENT[start..start + 1] } else { &CONTENT[start..=end] };
                                let _ = tls.write_all(body).await;
                            }
                            let _ = tls.shutdown().await;
                        });
                    }
                    _ = tasks.join_next(), if !tasks.is_empty() => (),
                }
            }
        });
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = listener.local_addr().unwrap();
        let connect_count = Arc::new(AtomicUsize::new(0));
        let count = connect_count.clone();
        let proxy = tokio::spawn(async move {
            let mut tasks = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut client, _) = accepted.unwrap();
                        let count = count.clone();
                        tasks.spawn(async move {
                            let Ok(Ok(header)) = timeout(Duration::from_secs(3), read_head(&mut client)).await else { return; };
                            assert_eq!(header.lines().next(), Some("CONNECT rr1.googlevideo.com:443 HTTP/1.1"));
                            count.fetch_add(1, Ordering::SeqCst);
                            let mut remote = TcpStream::connect(origin_address).await.unwrap();
                            client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await.unwrap();
                            let _ = tokio::io::copy_bidirectional(&mut client, &mut remote).await;
                        });
                    }
                    _ = tasks.join_next(), if !tasks.is_empty() => (),
                }
            }
        });
        let mut builder = reqwest::Client::builder().no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://{proxy_address}")).unwrap())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(2)).timeout(Duration::from_secs(5));
        if trust_certificate {
            builder = builder.add_root_certificate(reqwest::Certificate::from_der(der.as_ref()).unwrap());
        }
        let client = builder.build().unwrap();
        Self { client: SearchClient(client.clone(), client), origin_count, connect_count, origin, proxy }
    }
    async fn session(&self) -> PlaybackSession {
        PlaybackSession::start(self.client.clone(), crate::media::fixture_stream()).await.unwrap()
    }
}

fn player() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(5)).build().unwrap()
}

#[tokio::test]
async fn full_head_and_seek_bytes_cross_connect_tls_gateway() {
    let fixture = Fixture::new(Mode::Normal, true).await;
    fixture.client.inspect_stream(&crate::media::fixture_stream()).await.unwrap();
    let session = fixture.session().await;
    let http = player();
    let response = http.get(session.local_url()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(!response.headers().contains_key(header::SET_COOKIE));
    assert!(!response.headers().contains_key("x-cdn-secret"));
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.bytes().await.unwrap().as_ref(), CONTENT);
    let response = http.head(session.local_url()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_LENGTH].to_str().unwrap(), CONTENT.len().to_string());
    assert!(response.bytes().await.unwrap().is_empty());
    for (range, expected) in [("bytes=10-19", &CONTENT[10..20]), ("bytes=30-", &CONTENT[30..]), ("bytes=-5", &CONTENT[CONTENT.len()-5..])] {
        let response = http.get(session.local_url()).header(header::RANGE, range).send().await.unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.bytes().await.unwrap().as_ref(), expected);
    }
    assert_eq!(fixture.origin_count.load(Ordering::SeqCst), 6);
    assert_eq!(fixture.connect_count.load(Ordering::SeqCst), 6);
}

#[tokio::test]
async fn rejects_valid_but_different_cdn_range() {
    let fixture = Fixture::new(Mode::WrongRange, true).await;
    let session = fixture.session().await;
    let response = player().get(session.local_url()).header(header::RANGE, "bytes=10-19").send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(response.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn rejects_untrusted_tls_without_reaching_origin_http() {
    let fixture = Fixture::new(Mode::Normal, false).await;
    let session = fixture.session().await;
    let response = player().get(session.local_url()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert_eq!(fixture.connect_count.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.origin_count.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn does_not_follow_cdn_redirect_or_leak_response_headers() {
    let fixture = Fixture::new(Mode::Redirect, true).await;
    let session = fixture.session().await;
    let response = player().get(session.local_url()).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    assert!(!response.headers().contains_key(header::LOCATION));
    assert!(response.text().await.unwrap().is_empty());
    assert_eq!(fixture.connect_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn reports_truncated_body_as_error_instead_of_successful_video() {
    let fixture = Fixture::new(Mode::Truncated, true).await;
    let session = fixture.session().await;
    // Depending on transport scheduling, hyper may abort before or after sending headers.
    if let Ok(response) = player().get(session.local_url()).send().await {
        assert!(response.bytes().await.is_err());
    }
    assert_eq!(fixture.origin_count.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.connect_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn closing_session_cancels_an_active_stalled_body() {
    let fixture = Fixture::new(Mode::Stalled, true).await;
    let session = fixture.session().await;
    let mut response = player().get(session.local_url()).send().await.unwrap();
    assert_eq!(response.chunk().await.unwrap().unwrap().as_ref(), &CONTENT[..1]);
    drop(session);
    let result = timeout(Duration::from_secs(2), response.chunk()).await.expect("session cancellation must interrupt streaming");
    assert!(result.is_err());
}
