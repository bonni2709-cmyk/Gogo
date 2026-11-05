//! Unprivileged transports. Splitting writes is best effort, not IP fragmentation.
use anyhow::{bail, Context, Result};
use std::{sync::Arc, time::{Duration, Instant}};
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
    ["youtube.com", "googlevideo.com", "ytimg.com", "youtubei.googleapis.com"]
        .iter().any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
        && host.is_ascii() && !host.contains(['/', ':', '\\', '@'])
}

/// Always bounded; the worker owns the upstream socket for the lifetime of the tunnel.
pub async fn connect(host: &str, port: u16, strategy: Strategy) -> Result<DuplexStream> {
    if !allowed_host(host) || port != 443 { bail!("destination not permitted"); }
    if matches!(strategy, Strategy::SplitHello { chunk: 0, .. }) { bail!("zero chunk size"); }
    let upstream = timeout(Duration::from_secs(6), TcpStream::connect((host, port))).await??;
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
        for host in ["evil-youtube.com", "youtube.com.evil.test", "127.0.0.1", "youtube.com@evil", "youtube.com:443"] { assert!(!allowed_host(host)); }
    }
    #[tokio::test]
    async fn rejects_before_connect() {
        assert!(connect("localhost", 443, Strategy::Direct).await.is_err());
        assert!(connect("youtube.com", 80, Strategy::Direct).await.is_err());
        assert!(connect("youtube.com", 443, Strategy::SplitHello { chunk: 0, delay_ms: 0 }).await.is_err());
    }
}
