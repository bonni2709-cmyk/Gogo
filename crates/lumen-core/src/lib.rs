//! Unprivileged transports. Splitting writes is best effort, not IP fragmentation.
use anyhow::{bail, Context, Result};
use std::{net::IpAddr, sync::Arc, time::{Duration, Instant}};
use tokio::{io::{AsyncReadExt, AsyncWriteExt, DuplexStream}, net::TcpStream, time::{sleep, timeout}};
use tokio_rustls::{rustls::{self, pki_types::ServerName}, TlsConnector};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Strategy {
    Direct,
    SplitHello { chunk: usize, delay_ms: u64 },
}

pub const CANDIDATES: [Strategy; 3] = [
    Strategy::Direct,
    Strategy::SplitHello { chunk: 32, delay_ms: 5 },
    Strategy::SplitHello { chunk: 8, delay_ms: 10 },
];

/// Only YouTube infrastructure is eligible for automatic probing and proxying.
/// No user-supplied SNI, arbitrary targets, or certificate verification bypass.
pub fn allowed_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    if host.len() > 253 || !host.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63
            && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) { return false; }
    ["youtube.com", "googlevideo.com", "ytimg.com", "youtubei.googleapis.com"]
        .iter().any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        && host.is_ascii() && !host.contains(['/', ':', '\\', '@'])
}

// Conservative destination policy, including IPv4-mapped IPv6 and metadata ranges.
fn public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(a == 0 || a == 10 || a == 127 || a >= 224
                || (a == 100 && (64..=127).contains(&b))
                || (a == 169 && b == 254)
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && (b == 168 || b == 0 || (b == 88 && c == 99)))
                || (a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                || (a == 203 && b == 0 && c == 113))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4() { return public_address(IpAddr::V4(v4)); }
            let segments = ip.segments();
            // Only global unicast 2000::/3; exclude special-purpose and docs ranges.
            (segments[0] & 0xe000) == 0x2000
                && !(segments[0] == 0x2001 && (segments[1] < 0x200 || segments[1] == 0xdb8))
                && segments[0] != 0x2002 // 6to4 can encode private IPv4 destinations
                && !(segments[0] == 0x3fff && segments[1] < 0x1000)
        }
    }
}

/// Always bounded; the worker owns the upstream socket for the lifetime of the tunnel.
pub async fn connect(host: &str, port: u16, strategy: Strategy) -> Result<DuplexStream> {
    if !allowed_host(host) || port != 443 { bail!("destination not permitted"); }
    if matches!(strategy, Strategy::SplitHello { chunk: 0, .. }) { bail!("zero chunk size"); }
    // Resolve once, validate, then connect to the validated addresses, never re-resolve.
    let upstream = timeout(Duration::from_secs(6), async {
        let addresses: Vec<_> = tokio::net::lookup_host((host, port)).await?.collect();
        if addresses.is_empty() || addresses.iter().any(|address| !public_address(address.ip())) {
            bail!("DNS returned an empty or non-public destination");
        }
        Ok::<_, anyhow::Error>(TcpStream::connect(addresses.as_slice()).await?)
    }).await??;
    transport(upstream, strategy)
}

fn transport(upstream: TcpStream, strategy: Strategy) -> Result<DuplexStream> {
    if matches!(strategy, Strategy::SplitHello { chunk: 0, .. }) { bail!("zero chunk size"); }
    upstream.set_nodelay(true)?;
    let (client, mut worker) = tokio::io::duplex(64 * 1024);
    tokio::spawn(async move {
        let mut upstream = upstream;
        let result: Result<()> = async {
            if let Strategy::SplitHello { chunk, delay_ms } = strategy {
                // Read one complete TLS handshake record, not arbitrary application traffic.
                let record = timeout(Duration::from_secs(5), async {
                    let mut header = [0u8; 5];
                    worker.read_exact(&mut header).await?;
                    let length = u16::from_be_bytes([header[3], header[4]]) as usize;
                    if header[0] != 22 || header[1] != 3 || length == 0 || length > 16384 {
                        bail!("expected a bounded TLS handshake record");
                    }
                    let mut record = header.to_vec();
                    record.resize(5 + length, 0);
                    worker.read_exact(&mut record[5..]).await?;
                    if record[5] != 1 { bail!("expected ClientHello"); }
                    Ok::<_, anyhow::Error>(record)
                }).await??;
                for part in record.chunks(chunk) {
                    upstream.write_all(part).await?;
                    sleep(Duration::from_millis(delay_ms.min(50))).await;
                }
            }
            tokio::io::copy_bidirectional(&mut worker, &mut upstream).await?;
            Ok(())
        }.await;
        if let Err(error) = result { tracing::debug!(%error, "tunnel ended"); }
    });
    Ok(client)
}

#[derive(Debug)]
pub struct Selection {
    pub strategy: Strategy,
    pub elapsed: Duration,
}

async fn probe(host: &str, strategy: Strategy) -> Result<()> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let socket = connect(host, 443, strategy).await?;
    let mut tls = connector.connect(ServerName::try_from(host.to_owned())?, socket).await?;
    tls.write_all(format!("HEAD / HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n").as_bytes()).await?;
    let mut prefix = [0u8; 12];
    tls.read_exact(&mut prefix).await?;
    if !prefix.starts_with(b"HTTP/1.") || !prefix[9..12].iter().all(u8::is_ascii_digit) {
        bail!("invalid HTTP response");
    }
    let status = std::str::from_utf8(&prefix[9..12])?.parse::<u16>()?;
    if !(200..400).contains(&status) { bail!("probe HTTP status {status}"); }
    Ok(())
}

/// Direct first, then progressively split. Every candidate must pass both probes.
/// Failure is explicit: no guessed strategy and no false claim of connectivity.
pub async fn select() -> Result<Selection> {
    for strategy in CANDIDATES {
        let started = Instant::now();
        let mut success = true;
        for host in ["www.youtube.com", "i.ytimg.com"] {
            match timeout(Duration::from_secs(12), probe(host, strategy)).await {
                Ok(Ok(())) => (),
                result => {
                    tracing::warn!(?strategy, host, ?result, "probe failed");
                    success = false;
                    break;
                }
            }
        }
        if success {
            tracing::info!(?strategy, "selected verified transport (media reachability not guaranteed)");
            return Ok(Selection { strategy, elapsed: started.elapsed() });
        }
    }
    Err(anyhow::anyhow!("No verified transport available; network may require a separate tunnel"))
        .context("automatic network self-test failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn destination_boundaries() {
        for host in ["youtube.com", "WWW.YOUTUBE.COM", "rr1.googlevideo.com"] { assert!(allowed_host(host)); }
        for host in ["evil-youtube.com", "youtube.com.evil.test", "127.0.0.1", "youtube.com@evil", "youtube.com:443", ".youtube.com", "a..youtube.com", "-a.youtube.com", "a_.youtube.com"] { assert!(!allowed_host(host)); }
    }
    #[tokio::test]
    async fn rejects_before_connect() {
        assert!(connect("localhost", 443, Strategy::Direct).await.is_err());
        assert!(connect("youtube.com", 80, Strategy::Direct).await.is_err());
        assert!(connect("youtube.com", 443, Strategy::SplitHello { chunk: 0, delay_ms: 0 }).await.is_err());
    }

    async fn tls_roundtrip(strategy: Strategy, trusted: bool) -> Result<()> {
        use tokio::net::TcpListener;
        use tokio_rustls::{TlsAcceptor, rustls::pki_types::PrivatePkcs8KeyDer};
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
        let cert_der = certificate.cert.der().clone();
        let key = PrivatePkcs8KeyDer::from(certificate.key_pair.serialize_der());
        let config = rustls::ServerConfig::builder()
            .with_no_client_auth().with_single_cert(vec![cert_der.clone()], key.into())?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await?;
            let mut tls = acceptor.accept(socket).await?;
            let mut request = [0; 4];
            tls.read_exact(&mut request).await?;
            assert_eq!(&request, b"ping");
            tls.write_all(b"pong").await?;
            tls.shutdown().await?;
            Ok::<_, anyhow::Error>(())
        });
        let mut roots = rustls::RootCertStore::empty();
        if trusted { roots.add(cert_der)?; }
        let config = rustls::ClientConfig::builder().with_root_certificates(roots).with_no_client_auth();
        let connector = TlsConnector::from(Arc::new(config));
        let socket = transport(TcpStream::connect(address).await?, strategy)?;
        let handshake = connector.connect(ServerName::try_from("localhost")?, socket).await;
        if !trusted {
            assert!(handshake.is_err(), "untrusted TLS certificate must be rejected");
            assert!(server.await?.is_err());
            return Ok(());
        }
        let mut tls = handshake?;
        tls.write_all(b"ping").await?;
        let mut response = [0; 4];
        tls.read_exact(&mut response).await?;
        assert_eq!(&response, b"pong");
        server.await??;
        Ok(())
    }

    #[tokio::test]
    async fn real_tls_survives_all_transports() {
        for strategy in CANDIDATES {
            timeout(Duration::from_secs(10), tls_roundtrip(strategy, true)).await.unwrap().unwrap();
        }
    }

    #[tokio::test]
    async fn split_rejects_non_tls_without_forwarding() {
        use tokio::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        let mut client = transport(socket, CANDIDATES[1]).unwrap();
        client.write_all(b"GET / HTTP/1.1\r\n\r\n").await.unwrap();
        let mut byte = [0];
        let received = timeout(Duration::from_secs(2), server.read(&mut byte)).await.unwrap().unwrap();
        assert_eq!(received, 0);
    }

    #[test]
    fn blocks_private_and_special_dns_answers() {
        for ip in ["127.0.0.1", "10.1.2.3", "100.100.100.200", "169.254.169.254",
            "172.16.0.1", "192.168.1.1", "198.18.0.1", "203.0.113.1", "224.0.0.1",
            "::1", "::ffff:127.0.0.1", "fc00::1", "fe80::1", "2001:db8::1", "2002:7f00:1::1"] {
            assert!(!public_address(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["142.250.185.78", "2607:f8b0:4005:80a::200e", "2001:4860:4860::8888"] {
            assert!(public_address(ip.parse().unwrap()), "{ip}");
        }
    }

    #[tokio::test]
    async fn all_transports_reject_untrusted_certificates() {
        for strategy in CANDIDATES {
            timeout(Duration::from_secs(10), tls_roundtrip(strategy, false)).await.unwrap().unwrap();
        }
    }

}
