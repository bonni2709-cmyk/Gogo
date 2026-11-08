//! Loopback-only, bounded CONNECT / SOCKS5 gateway. HTTPS targets on port 443 only.
use anyhow::{bail, Result};
use lumen_core::{connect, Strategy};
use std::{sync::Arc, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{TcpListener, TcpStream}, sync::{Semaphore, watch}, time::timeout};

/// Compatibility entry point for a fixed, already verified transport.
pub async fn serve(listener: TcpListener, strategy: Strategy) -> Result<()> {
    let (_sender, strategies) = watch::channel(Some(strategy));
    serve_dynamic(listener, strategies).await
}

/// Keeps one listener while the controller publishes newly verified strategies.
/// None rejects new upstream connections; existing tunnels retain their strategy.
pub async fn serve_dynamic(listener: TcpListener, strategies: watch::Receiver<Option<Strategy>>) -> Result<()> {
    if !listener.local_addr()?.ip().is_loopback() { bail!("proxy must bind loopback"); }
    let slots = Arc::new(Semaphore::new(64));
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            accepted = async {
                let permit = slots.clone().acquire_owned().await?;
                let (client, _) = listener.accept().await?;
                Ok::<_, anyhow::Error>((client, permit))
            } => {
                let (mut client, permit) = accepted?;
                let strategies = strategies.clone();
                tasks.spawn(async move {
                    let _permit = permit;
                    let result = timeout(Duration::from_secs(15), handshake(&mut client, &strategies)).await;
                    match result {
                        Ok(Ok(mut remote)) => {
                            let _ = timeout(Duration::from_secs(4 * 3600), tokio::io::copy_bidirectional(&mut client, &mut remote)).await;
                        }
                        other => tracing::debug!(?other, "proxy handshake rejected"),
                    }
                });
            }
            _ = tasks.join_next(), if !tasks.is_empty() => (),
        }
    }
}

fn selected(strategies: &watch::Receiver<Option<Strategy>>) -> Result<Strategy> {
    if strategies.has_changed().is_err() { bail!("strategy controller is closed"); }
    (*strategies.borrow()).ok_or_else(|| anyhow::anyhow!("network recovery: no verified transport"))
}

async fn upstream(host: &str, port: u16, strategies: &watch::Receiver<Option<Strategy>>) -> Result<tokio::io::DuplexStream> {
    let strategy = selected(strategies)?;
    connect(host, port, strategy).await
}

async fn handshake(client: &mut TcpStream, strategies: &watch::Receiver<Option<Strategy>>) -> Result<tokio::io::DuplexStream> {
    let first = client.read_u8().await?;
    if first == 5 {
        let count = client.read_u8().await? as usize;
        let mut methods = vec![0; count];
        client.read_exact(&mut methods).await?;
        if !methods.contains(&0) { client.write_all(&[5, 255]).await?; bail!("authentication unsupported"); }
        client.write_all(&[5, 0]).await?;
        let mut request = [0; 4];
        client.read_exact(&mut request).await?;
        if request != [5, 1, 0, 3] { client.write_all(&[5, 7, 0, 1, 0, 0, 0, 0, 0, 0]).await?; bail!("domain CONNECT required"); }
        let length = client.read_u8().await? as usize;
        let mut domain = vec![0; length];
        client.read_exact(&mut domain).await?;
        let host = std::str::from_utf8(&domain)?;
        let port = client.read_u16().await?;
        match upstream(host, port, strategies).await {
            Ok(remote) => { client.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0]).await?; Ok(remote) }
            Err(error) => { client.write_all(&[5, 2, 0, 1, 0, 0, 0, 0, 0, 0]).await?; Err(error) }
        }
    } else {
        let mut header = vec![first];
        while !header.ends_with(b"\r\n\r\n") {
            if header.len() >= 8192 { bail!("header too large"); }
            header.push(client.read_u8().await?);
        }
        let text = std::str::from_utf8(&header)?;
        let line = text.lines().next().unwrap_or_default();
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.len() != 3 || fields[0] != "CONNECT" || fields[2] != "HTTP/1.1" {
            client.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nConnection: close\r\n\r\n").await?;
            bail!("only CONNECT supported");
        }
        let (host, port) = fields[1].rsplit_once(':').ok_or_else(|| anyhow::anyhow!("missing port"))?;
        match upstream(host, port.parse()?, strategies).await {
            Ok(remote) => { client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?; Ok(remote) }
            Err(error) => { client.write_all(b"HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n").await?; Err(error) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).await.unwrap();
        let (server, _) = listener.accept().await.unwrap();
        (client, server)
    }

    #[tokio::test]
    async fn rejects_plain_http() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move {
            let (_controller, strategies) = watch::channel(Some(Strategy::Direct));
            handshake(&mut server, &strategies).await.is_err()
        });
        client.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 405"));
        assert!(task.await.unwrap());
    }

    #[tokio::test]
    async fn rejects_socks_without_no_auth_method() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move {
            let (_controller, strategies) = watch::channel(Some(Strategy::Direct));
            handshake(&mut server, &strategies).await.is_err()
        });
        client.write_all(&[5, 1, 2]).await.unwrap();
        let mut response = [0; 2];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(response, [5, 255]);
        assert!(task.await.unwrap());
    }
    #[tokio::test]
    async fn connect_rejects_loopback_destination() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move {
            let (_controller, strategies) = watch::channel(Some(Strategy::Direct));
            handshake(&mut server, &strategies).await.is_err()
        });
        client.write_all(b"CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 502"));
        assert!(task.await.unwrap());
    }

    #[tokio::test]
    async fn socks_rejects_disallowed_domain() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move {
            let (_controller, strategies) = watch::channel(Some(Strategy::Direct));
            handshake(&mut server, &strategies).await.is_err()
        });
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut greeting = [0; 2];
        client.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 0]);
        let mut request = vec![5, 1, 0, 3, 9];
        request.extend_from_slice(b"localhost");
        request.extend_from_slice(&443u16.to_be_bytes());
        client.write_all(&request).await.unwrap();
        let mut response = [0; 10];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(response[1], 2);
        assert!(task.await.unwrap());
    }

    #[test]
    fn follows_only_live_verified_strategy_updates() {
        let (controller, strategies) = watch::channel(None);
        assert!(selected(&strategies).is_err());
        controller.send_replace(Some(Strategy::Direct));
        assert_eq!(selected(&strategies).unwrap(), Strategy::Direct);
        let split = Strategy::SplitHello { chunk: 32, delay_ms: 5 };
        controller.send_replace(Some(split));
        assert_eq!(selected(&strategies).unwrap(), split);
        controller.send_replace(None);
        assert!(selected(&strategies).is_err());
        controller.send_replace(Some(Strategy::Direct));
        drop(controller);
        assert!(selected(&strategies).is_err(), "do not use stale strategy after controller exit");
    }

    #[tokio::test]
    async fn recovery_keeps_listener_and_rejects_new_connections() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (controller, strategies) = watch::channel(None);
        let service = tokio::spawn(serve_dynamic(listener, strategies));
        for strategy in [None, Some(Strategy::Direct), None] {
            controller.send_replace(strategy);
            let mut client = TcpStream::connect(address).await.unwrap();
            // In the enabled phase use a forbidden destination, never the public network.
            let host = if strategy.is_some() { "127.0.0.1" } else { "www.youtube.com" };
            client.write_all(format!("CONNECT {host}:443 HTTP/1.1\r\n\r\n").as_bytes()).await.unwrap();
            let mut response = Vec::new();
            timeout(Duration::from_secs(1), client.read_to_end(&mut response)).await.unwrap().unwrap();
            assert!(response.starts_with(b"HTTP/1.1 502"));
        }
        service.abort();
        let _ = service.await;
    }

    #[tokio::test]
    async fn recovery_socks_request_fails_without_dns_or_upstream_connect() {
        let (mut client, mut server) = pair().await;
        let (_controller, strategies) = watch::channel(None);
        let task = tokio::spawn(async move { handshake(&mut server, &strategies).await.is_err() });
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut greeting = [0; 2];
        client.read_exact(&mut greeting).await.unwrap();
        assert_eq!(greeting, [5, 0]);
        let mut request = vec![5, 1, 0, 3, 15];
        request.extend_from_slice(b"www.youtube.com");
        request.extend_from_slice(&443u16.to_be_bytes());
        client.write_all(&request).await.unwrap();
        let mut response = [0; 10];
        timeout(Duration::from_secs(1), client.read_exact(&mut response)).await.unwrap().unwrap();
        assert_eq!(response[1], 2);
        assert!(task.await.unwrap());
    }

    #[tokio::test]
    async fn stopping_proxy_closes_incomplete_client_handshakes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (_controller, strategies) = watch::channel(None);
        let service = tokio::spawn(serve_dynamic(listener, strategies));
        let mut client = TcpStream::connect(address).await.unwrap();
        // SOCKS method acknowledgement proves a handler has accepted this connection.
        client.write_all(&[5, 1, 0]).await.unwrap();
        let mut greeting = [0; 2];
        client.read_exact(&mut greeting).await.unwrap();
        service.abort();
        let _ = service.await;
        let mut byte = [0];
        assert_eq!(timeout(Duration::from_secs(1), client.read(&mut byte)).await.unwrap().unwrap(), 0);
    }

}
