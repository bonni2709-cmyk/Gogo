//! Loopback-only, bounded CONNECT / SOCKS5 gateway. HTTPS targets on port 443 only.
use anyhow::{bail, Result};
use lumen_core::{connect, Strategy};
use std::{sync::Arc, time::Duration};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{TcpListener, TcpStream}, sync::Semaphore, time::timeout};

pub async fn serve(listener: TcpListener, strategy: Strategy) -> Result<()> {
    if !listener.local_addr()?.ip().is_loopback() { bail!("proxy must bind loopback"); }
    let slots = Arc::new(Semaphore::new(64));
    loop {
        let permit = slots.clone().acquire_owned().await?;
        let (mut client, _) = listener.accept().await?;
        tokio::spawn(async move {
            let _permit = permit;
            let result = timeout(Duration::from_secs(15), handshake(&mut client, strategy)).await;
            match result {
                Ok(Ok(mut remote)) => {
                    // Bound idle/stalled sessions by a maximum lifetime.
                    let _ = timeout(Duration::from_secs(4 * 3600), tokio::io::copy_bidirectional(&mut client, &mut remote)).await;
                }
                other => tracing::debug!(?other, "proxy handshake rejected"),
            }
        });
    }
}

async fn handshake(client: &mut TcpStream, strategy: Strategy) -> Result<tokio::io::DuplexStream> {
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
        match connect(host, port, strategy).await {
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
        match connect(host, port.parse()?, strategy).await {
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
        let task = tokio::spawn(async move { handshake(&mut server, Strategy::Direct).await.is_err() });
        client.write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 405"));
        assert!(task.await.unwrap());
    }

    #[tokio::test]
    async fn rejects_socks_without_no_auth_method() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move { handshake(&mut server, Strategy::Direct).await.is_err() });
        client.write_all(&[5, 1, 2]).await.unwrap();
        let mut response = [0; 2];
        client.read_exact(&mut response).await.unwrap();
        assert_eq!(response, [5, 255]);
        assert!(task.await.unwrap());
    }
    #[tokio::test]
    async fn connect_rejects_loopback_destination() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move { handshake(&mut server, Strategy::Direct).await.is_err() });
        client.write_all(b"CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n").await.unwrap();
        let mut response = Vec::new();
        client.read_to_end(&mut response).await.unwrap();
        assert!(response.starts_with(b"HTTP/1.1 502"));
        assert!(task.await.unwrap());
    }

    #[tokio::test]
    async fn socks_rejects_disallowed_domain() {
        let (mut client, mut server) = pair().await;
        let task = tokio::spawn(async move { handshake(&mut server, Strategy::Direct).await.is_err() });
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

}
