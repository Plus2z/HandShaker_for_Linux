use std::collections::{HashMap, HashSet};

use bytes::{Buf, BufMut, Bytes, BytesMut};

use crate::error::{ProtoError, Result};

/// Maximum wire chunk sent from Android (`a$c.java:198`), including its 6-byte header.
pub const MAX_TRANSPORT_CHUNK_BYTES: usize = 32_767;
pub const TRANSPORT_CHUNK_HEADER_LEN: usize = 6;
pub const MAX_CHUNK_BODY_LEN: usize = MAX_TRANSPORT_CHUNK_BYTES - TRANSPORT_CHUNK_HEADER_LEN;
pub const OUTBOUND_HEADER_LEN: usize = 9;

/// Control / data flags on the 9-byte Desktop -> Phone packet header (`aoa/g/h.java:313`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum FrameFlag {
    Handshake = 0,
    SignedRpc = 1,
    Cancel = 2,
    UploadChunk = 3,
    TrustCancel = 4,
    Quit = 5,
}

/// Encodes one Desktop -> Phone packet: `[sid: u32BE][flag: u8][len: u32BE][payload]`.
pub fn encode_outbound_frame(sid: u32, flag: FrameFlag, payload: &[u8]) -> Bytes {
    let mut buf = BytesMut::with_capacity(OUTBOUND_HEADER_LEN + payload.len());
    buf.put_u32(sid);
    buf.put_u8(flag as u8);
    buf.put_u32(payload.len() as u32);
    buf.put_slice(payload);
    buf.freeze()
}

/// Splits a Phone -> Desktop logical message (`[u64BE len][payload]`) into <= 32767B chunks.
/// Primarily used by mock servers and roundtrip framing tests.
pub fn encode_inbound_proto_chunks(sid: u32, payload: &[u8]) -> Vec<Bytes> {
    let mut framed = BytesMut::with_capacity(8 + payload.len());
    framed.put_u64(payload.len() as u64);
    framed.put_slice(payload);
    encode_raw_chunks(sid, &framed)
}

/// Splits raw stream bytes into `[sid: u32BE][chunk_len: u16BE][chunk_bytes]` chunks.
pub fn encode_raw_chunks(sid: u32, data: &[u8]) -> Vec<Bytes> {
    if data.is_empty() {
        return Vec::new();
    }
    data.chunks(MAX_CHUNK_BODY_LEN)
        .map(|slice| {
            let mut chunk = BytesMut::with_capacity(TRANSPORT_CHUNK_HEADER_LEN + slice.len());
            chunk.put_u32(sid);
            chunk.put_u16(slice.len() as u16);
            chunk.put_slice(slice);
            chunk.freeze()
        })
        .collect()
}

/// Decoded item emitted by [`InboundDemuxer`] for a specific `sid`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DemuxEvent {
    /// A complete length-prefixed protobuf message `[u64BE len][payload]`.
    ProtoMessage { sid: u32, payload: Bytes },
    /// Raw file data chunk belonging to an active `GET_DOWNLOAD_FILE_REQUEST` session.
    DownloadBytes { sid: u32, data: Bytes },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SidMode {
    /// Normal message stream: repeat `[total_len: u64BE][payload]`.
    ProtoMessages,
    /// Download session awaiting the initial `SSPDownloadFileResponseHeader` protobuf message.
    DownloadAwaitingHeader,
    /// Header was emitted; hold remaining bytes in `sid_buffers` until caller reports `range.length`.
    DownloadPendingHeaderAck,
    /// Download session streaming raw file bytes after `SSPDownloadFileResponseHeader(ready=true)`.
    DownloadRawBytes { remaining: u64 },
}

/// Reassembles fragmented `[sid: u32BE][chunk_len: u16BE][body]` packets from Phone -> Desktop.
#[derive(Debug, Default)]
pub struct InboundDemuxer {
    wire_buf: BytesMut,
    sid_buffers: HashMap<u32, BytesMut>,
    download_sids: HashSet<u32>,
    download_modes: HashMap<u32, SidMode>,
}

impl InboundDemuxer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Marks `sid` as a download session so its first protobuf message is followed by raw bytes.
    pub fn register_download_sid(&mut self, sid: u32) {
        self.download_sids.insert(sid);
        self.download_modes.insert(sid, SidMode::DownloadAwaitingHeader);
    }

    /// Transitions a registered download `sid` into raw binary streaming and drains any queued bytes.
    pub fn set_download_data_len(
        &mut self,
        sid: u32,
        remaining: u64,
        out: &mut Vec<DemuxEvent>,
    ) -> Result<()> {
        if remaining == 0 {
            self.finish_download_sid(sid);
            return Ok(());
        }
        self.download_modes
            .insert(sid, SidMode::DownloadRawBytes { remaining });
        self.drain_sid(sid, out)
    }

    /// Clears download tracking (e.g. `ready=false`, error, cancel, or completion).
    pub fn finish_download_sid(&mut self, sid: u32) {
        self.download_sids.remove(&sid);
        self.download_modes.remove(&sid);
        self.sid_buffers.remove(&sid);
    }

    /// Appends newly read wire bytes and drains any complete transport chunks into `out`.
    pub fn push_wire_bytes(&mut self, data: &[u8], out: &mut Vec<DemuxEvent>) -> Result<()> {
        self.wire_buf.extend_from_slice(data);

        while self.wire_buf.len() >= TRANSPORT_CHUNK_HEADER_LEN {
            let sid = u32::from_be_bytes([
                self.wire_buf[0],
                self.wire_buf[1],
                self.wire_buf[2],
                self.wire_buf[3],
            ]);
            let chunk_len = u16::from_be_bytes([self.wire_buf[4], self.wire_buf[5]]) as usize;
            if chunk_len > MAX_CHUNK_BODY_LEN {
                return Err(ProtoError::Framing(format!(
                    "Chunk body length {chunk_len} exceeds max {MAX_CHUNK_BODY_LEN} (sid={sid})"
                )));
            }
            if self.wire_buf.len() < TRANSPORT_CHUNK_HEADER_LEN + chunk_len {
                break;
            }
            self.wire_buf.advance(TRANSPORT_CHUNK_HEADER_LEN);
            let body = self.wire_buf.split_to(chunk_len);
            let sid_buf = self.sid_buffers.entry(sid).or_default();
            sid_buf.extend_from_slice(&body);
            self.drain_sid(sid, out)?;
        }

        Ok(())
    }

    /// Drains complete messages or raw download bytes buffered for `sid`.
    pub fn drain_sid(&mut self, sid: u32, out: &mut Vec<DemuxEvent>) -> Result<()> {
        loop {
            let mode = self
                .download_modes
                .get(&sid)
                .copied()
                .unwrap_or(SidMode::ProtoMessages);

            match mode {
                SidMode::ProtoMessages | SidMode::DownloadAwaitingHeader => {
                    let Some(buf) = self.sid_buffers.get_mut(&sid) else {
                        break;
                    };
                    if buf.len() < 8 {
                        break;
                    }
                    let total_len = u64::from_be_bytes([
                        buf[0], buf[1], buf[2], buf[3], buf[4], buf[5], buf[6], buf[7],
                    ]);
                    // Sanity bound: control/media-list protobufs shouldn't exceed 256 MiB.
                    if total_len > 256 * 1024 * 1024 {
                        return Err(ProtoError::Framing(format!(
                            "Implausible message length {total_len} on sid={sid}"
                        )));
                    }
                    let total_usize = total_len as usize;
                    if buf.len() < 8 + total_usize {
                        break;
                    }
                    buf.advance(8);
                    let payload = buf.split_to(total_usize).freeze();
                    if buf.is_empty() && mode == SidMode::ProtoMessages {
                        self.sid_buffers.remove(&sid);
                    }
                    out.push(DemuxEvent::ProtoMessage { sid, payload });
                    // Stop draining this sid until caller inspects DownloadResponseHeader and sets remaining.
                    if mode == SidMode::DownloadAwaitingHeader {
                        self.download_modes
                            .insert(sid, SidMode::DownloadPendingHeaderAck);
                        break;
                    }
                }
                SidMode::DownloadPendingHeaderAck => {
                    break;
                }
                SidMode::DownloadRawBytes { remaining } => {
                    let Some(buf) = self.sid_buffers.get_mut(&sid) else {
                        break;
                    };
                    if buf.is_empty() || remaining == 0 {
                        break;
                    }
                    let take = (buf.len() as u64).min(remaining) as usize;
                    let data = buf.split_to(take).freeze();
                    let next_remaining = remaining - take as u64;
                    out.push(DemuxEvent::DownloadBytes { sid, data });
                    if next_remaining == 0 {
                        self.finish_download_sid(sid);
                        break;
                    } else {
                        self.download_modes.insert(
                            sid,
                            SidMode::DownloadRawBytes {
                                remaining: next_remaining,
                            },
                        );
                    }
                }
            }
        }
        Ok(())
    }
}

/// Parses `[sid: u32BE][flag: u8][len: u32BE][payload]` packets from Desktop -> Phone (used in mock server/tests).
#[derive(Debug, Default)]
pub struct OutboundFrameParser {
    buf: BytesMut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParsedOutboundFrame {
    pub sid: u32,
    pub flag: u8,
    pub payload: Bytes,
}

impl OutboundFrameParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_bytes(&mut self, data: &[u8]) -> Vec<ParsedOutboundFrame> {
        self.buf.extend_from_slice(data);
        let mut frames = Vec::new();
        while self.buf.len() >= OUTBOUND_HEADER_LEN {
            let sid = u32::from_be_bytes([self.buf[0], self.buf[1], self.buf[2], self.buf[3]]);
            let flag = self.buf[4];
            let len =
                u32::from_be_bytes([self.buf[5], self.buf[6], self.buf[7], self.buf[8]]) as usize;
            if self.buf.len() < OUTBOUND_HEADER_LEN + len {
                break;
            }
            self.buf.advance(OUTBOUND_HEADER_LEN);
            let payload = self.buf.split_to(len).freeze();
            frames.push(ParsedOutboundFrame { sid, flag, payload });
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outbound_frame_parser_handles_fragmented_reads() {
        let f1 = encode_outbound_frame(7, FrameFlag::SignedRpc, b"hello");
        let f2 = encode_outbound_frame(8, FrameFlag::Cancel, &42u32.to_be_bytes());
        let mut combined = Vec::new();
        combined.extend_from_slice(&f1);
        combined.extend_from_slice(&f2);

        let mut parser = OutboundFrameParser::new();
        let mut parsed = Vec::new();
        for piece in combined.chunks(3) {
            parsed.extend(parser.push_bytes(piece));
        }

        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].sid, 7);
        assert_eq!(parsed[0].flag, FrameFlag::SignedRpc as u8);
        assert_eq!(parsed[0].payload.as_ref(), b"hello");
        assert_eq!(parsed[1].sid, 8);
        assert_eq!(parsed[1].flag, FrameFlag::Cancel as u8);
        assert_eq!(parsed[1].payload.as_ref(), &42u32.to_be_bytes());
    }

    #[test]
    fn inbound_demuxer_reassembles_large_multichunk_proto() {
        let large_payload: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
        let chunks = encode_inbound_proto_chunks(12, &large_payload);
        assert!(chunks.len() >= 4);

        let mut stream = Vec::new();
        for c in chunks {
            stream.extend_from_slice(&c);
        }

        let mut demux = InboundDemuxer::new();
        let mut events = Vec::new();
        for piece in stream.chunks(777) {
            demux.push_wire_bytes(piece, &mut events).unwrap();
        }

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            DemuxEvent::ProtoMessage {
                sid: 12,
                payload: Bytes::from(large_payload)
            }
        );
    }

    #[test]
    fn inbound_demuxer_splits_coalesced_download_header_and_raw_bytes() {
        let sid = 99;
        let header_bytes = b"HDR_PROTO_PAYLOAD";
        let file_bytes = b"0123456789abcdefghijklmnopqrstuvwxyz";

        let mut combined_sid_stream = BytesMut::new();
        combined_sid_stream.put_u64(header_bytes.len() as u64);
        combined_sid_stream.put_slice(header_bytes);
        combined_sid_stream.put_slice(file_bytes);

        let wire_chunks = encode_raw_chunks(sid, &combined_sid_stream);
        assert_eq!(wire_chunks.len(), 1);

        let mut demux = InboundDemuxer::new();
        demux.register_download_sid(sid);

        let mut events = Vec::new();
        demux.push_wire_bytes(&wire_chunks[0], &mut events).unwrap();
        assert_eq!(
            events,
            vec![DemuxEvent::ProtoMessage {
                sid,
                payload: Bytes::from_static(header_bytes),
            }]
        );

        let mut tail = Vec::new();
        demux
            .set_download_data_len(sid, file_bytes.len() as u64, &mut tail)
            .unwrap();
        assert_eq!(
            tail,
            vec![DemuxEvent::DownloadBytes {
                sid,
                data: Bytes::from_static(file_bytes),
            }]
        );
    }
}
