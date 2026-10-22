//! Reaching a notary: plain TCP (`tcp://host:port`, local or self-hosted)
//! or WebSocket (`ws://` / `wss://`, e.g. the futures API's
//! `/oracle/notary`, which forwards to its notary).

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};

use async_tungstenite::tokio::{ConnectStream, connect_async};
use futures::{AsyncRead, AsyncWrite};
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};
use ws_stream_tungstenite::WsStream;

pub enum NotaryStream {
    Tcp(Compat<tokio::net::TcpStream>),
    Ws(Box<WsStream<ConnectStream>>),
}

/// Connects to `url`: `tcp://host:port`, `ws://…` or `wss://…`.
pub async fn connect(url: &str) -> io::Result<NotaryStream> {
    if let Some(addr) = url.strip_prefix("tcp://") {
        let tcp = tokio::net::TcpStream::connect(addr).await?;
        // MPC-TLS is chatty: Nagle's delay would sit on its critical path.
        tcp.set_nodelay(true)?;
        return Ok(NotaryStream::Tcp(tcp.compat()));
    }
    if url.starts_with("ws://") || url.starts_with("wss://") {
        let (ws, _) = connect_async(url)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::ConnectionRefused, e.to_string()))?;
        return Ok(NotaryStream::Ws(Box::new(WsStream::new(ws))));
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("`{url}` is not a notary address (tcp://, ws:// or wss://)"),
    ))
}

impl AsyncRead for NotaryStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_read(cx, buf),
            Self::Ws(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for NotaryStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_write(cx, buf),
            Self::Ws(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_flush(cx),
            Self::Ws(s) => Pin::new(s.as_mut()).poll_flush(cx),
        }
    }
    fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Tcp(s) => Pin::new(s).poll_close(cx),
            Self::Ws(s) => Pin::new(s.as_mut()).poll_close(cx),
        }
    }
}
