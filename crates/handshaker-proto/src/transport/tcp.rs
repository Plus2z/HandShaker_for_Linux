use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::error::{ProtoError, Result};
use crate::transport::{TransportKind, TransportLink};

/// Connects over TCP to an Android HandShaker server (`host:port`).
pub async fn connect_tcp(host: &str, port: u16, connect_timeout: Duration) -> Result<TransportLink> {
    let addr = format!("{host}:{port}");
    let stream = timeout(connect_timeout, TcpStream::connect(&addr))
        .await
        .map_err(|_| ProtoError::Timeout("TCP connect"))??;
    stream.set_nodelay(true)?;
    let (reader, writer) = stream.into_split();
    Ok(TransportLink::from_async_rw(
        reader,
        writer,
        TransportKind::WifiTcp,
        addr,
    ))
}
