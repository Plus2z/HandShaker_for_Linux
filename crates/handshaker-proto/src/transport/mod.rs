pub mod aoa;
pub mod tcp;

use std::fmt;

use bytes::Bytes;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::error::Result;

/// Underlying physical connection medium backing a [`crate::session::Session`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    WifiTcp,
    UsbAoa,
    Mock,
}

impl fmt::Display for TransportKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportKind::WifiTcp => f.write_str("wifi"),
            TransportKind::UsbAoa => f.write_str("usb_aoa"),
            TransportKind::Mock => f.write_str("mock"),
        }
    }
}

/// Bi-directional raw byte stream transport feeding [`crate::framing::InboundDemuxer`]
/// and draining outbound 9B-header frames.
pub struct TransportLink {
    pub kind: TransportKind,
    pub label: String,
    pub rx: mpsc::Receiver<Result<Bytes>>,
    pub tx: mpsc::Sender<Bytes>,
}

impl TransportLink {
    /// Adapts any Tokio `AsyncRead` + `AsyncWrite` pair (TCP socket or `tokio::io::duplex`)
    /// into a [`TransportLink`].
    pub fn from_async_rw<R, W>(
        mut reader: R,
        mut writer: W,
        kind: TransportKind,
        label: impl Into<String>,
    ) -> Self
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (rx_tx, rx) = mpsc::channel::<Result<Bytes>>(128);
        let (tx, mut tx_rx) = mpsc::channel::<Bytes>(128);

        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        let bytes = Bytes::copy_from_slice(&buf[..n]);
                        if rx_tx.send(Ok(bytes)).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        let _ = rx_tx.send(Err(e.into())).await;
                        break;
                    }
                }
            }
        });

        tokio::spawn(async move {
            while let Some(frame) = tx_rx.recv().await {
                if writer.write_all(&frame).await.is_err() {
                    break;
                }
                if tx_rx.is_empty() {
                    let _ = writer.flush().await;
                }
            }
            let _ = writer.shutdown().await;
        });

        Self {
            kind,
            label: label.into(),
            rx,
            tx,
        }
    }
}
