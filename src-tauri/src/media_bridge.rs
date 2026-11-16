use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::task::{Context, Poll};
use std::time::Duration;

use handshaker_proto::proto::{SspAudioAlbum, SspImageFile, SspVideoFile};
use handshaker_proto::Session;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{watch, RwLock, Semaphore};
use uuid::Uuid;

pub const MAX_STREAM_CHUNK_BYTES: u64 = 2 * 1024 * 1024;
const MAX_HTTP_HEADER_BYTES: u64 = 16 * 1024;
const MAX_HTTP_HEADER_LINE: usize = 8 * 1024;
const MAX_HTTP_HEADERS: usize = 32;
const HTTP_HEADER_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Localhost HTTP media proxy for cached thumbnails (`/thumb`) and byte-range streaming (`/stream`).
#[derive(Clone)]
pub struct MediaBridge {
    port: u16,
    token: String,
    cache_root: PathBuf,
    session_slot: Arc<RwLock<Option<Session>>>,
    inflight_thumbs: Arc<StdMutex<HashMap<String, watch::Receiver<bool>>>>,
    thumb_sem: Arc<Semaphore>,
}

impl MediaBridge {
    pub async fn start(session_slot: Arc<RwLock<Option<Session>>>) -> anyhow::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let token = Uuid::new_v4().to_string();
        let cache_root = dirs::cache_dir()
            .unwrap_or_else(|| PathBuf::from(".cache"))
            .join("handshaker-linux")
            .join("thumbs");

        let bridge = Self {
            port,
            token,
            cache_root,
            session_slot,
            inflight_thumbs: Arc::new(StdMutex::new(HashMap::new())),
            thumb_sem: Arc::new(Semaphore::new(4)),
        };

        let server_clone = bridge.clone();
        tokio::spawn(async move {
            while let Ok((stream, _addr)) = listener.accept().await {
                let b = server_clone.clone();
                tokio::spawn(async move {
                    let _ = handle_http_conn(stream, b).await;
                });
            }
        });

        Ok(bridge)
    }

    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn thumb_url(&self, kind: &str, id: u64, remote_path: &str) -> String {
        format!(
            "{}/thumb?kind={}&id={}&path={}&token={}",
            self.base_url(),
            url_encode(kind),
            id,
            url_encode(remote_path),
            self.token
        )
    }

    pub fn stream_url(&self, remote_path: &str, file_size: u64, mime: &str) -> String {
        format!(
            "{}/stream?path={}&size={}&mime={}&token={}",
            self.base_url(),
            url_encode(remote_path),
            file_size,
            url_encode(mime),
            self.token
        )
    }

    fn cache_path_for(&self, device_uuid: &str, kind: &str, id: u64, remote_path: &str) -> PathBuf {
        let safe_dev: String = device_uuid
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let path_hash = hex_sha256_prefix(remote_path);
        self.cache_root
            .join(if safe_dev.is_empty() { "default" } else { &safe_dev })
            .join(format!("{kind}_{id}_{path_hash}.jpg"))
    }
}

async fn handle_http_conn(stream: TcpStream, bridge: MediaBridge) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half.take(MAX_HTTP_HEADER_BYTES));

    let head_result = tokio::time::timeout(HTTP_HEADER_READ_TIMEOUT, async {
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).await? == 0 {
            return Ok::<_, std::io::Error>(None);
        }
        if request_line.len() > MAX_HTTP_HEADER_LINE {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Request line too long",
            ));
        }

        let mut range_header = None;
        let mut header_count = 0usize;
        let mut ended_cleanly = false;
        loop {
            let mut line = String::new();
            let n = reader.read_line(&mut line).await?;
            if n == 0 {
                break;
            }
            if line.len() > MAX_HTTP_HEADER_LINE {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Header line too long",
                ));
            }
            if line.trim().is_empty() {
                ended_cleanly = true;
                break;
            }
            header_count += 1;
            if header_count > MAX_HTTP_HEADERS {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Too many headers",
                ));
            }
            if let Some((k, v)) = line.split_once(':') {
                if k.trim().eq_ignore_ascii_case("range") {
                    range_header = Some(v.trim().to_owned());
                }
            }
        }
        if !ended_cleanly && reader.get_ref().limit() == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "HTTP headers exceed limit",
            ));
        }
        Ok(Some((request_line, range_header)))
    })
    .await;

    let (request_line, range_header) = match head_result {
        Ok(Ok(Some(parsed))) => parsed,
        Ok(Ok(None)) => return Ok(()),
        Ok(Err(_)) => return write_status(&mut write_half, 400, "Bad Request", &[]).await,
        Err(_) => return write_status(&mut write_half, 408, "Request Timeout", &[]).await,
    };

    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("/");
    if !matches!(method, "GET" | "HEAD") {
        return write_status(&mut write_half, 405, "Method Not Allowed", &[]).await;
    }
    let is_head = method == "HEAD";

    let (route, query) = target.split_once('?').unwrap_or((target, ""));
    let params = parse_query(query);
    if params.get("token").map(String::as_str) != Some(bridge.token()) {
        return write_status(&mut write_half, 403, "Forbidden", &[]).await;
    }

    match route {
        "/thumb" => serve_thumb(&mut write_half, &bridge, &params, is_head).await,
        "/stream" => serve_stream(&mut write_half, &bridge, &params, range_header.as_deref(), is_head).await,
        _ => write_status(&mut write_half, 404, "Not Found", &[]).await,
    }
}

async fn serve_thumb(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    bridge: &MediaBridge,
    params: &HashMap<String, String>,
    is_head: bool,
) -> anyhow::Result<()> {
    let kind = params.get("kind").map(String::as_str).unwrap_or("");
    let id = params.get("id").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let remote_path = params.get("path").map(String::as_str).unwrap_or("");

    let session = {
        let guard = bridge.session_slot.read().await;
        guard.clone()
    };
    let Some(session) = session else {
        return write_status(writer, 503, "Service Unavailable", &[]).await;
    };

    let dev_uuid = session.handshake_info().device_uuid.clone();
    let cache_path = bridge.cache_path_for(&dev_uuid, kind, id, remote_path);
    if let Ok(bytes) = tokio::fs::read(&cache_path).await {
        return write_binary(writer, 200, "OK", "image/jpeg", &bytes, is_head).await;
    }

    let key = cache_path.to_string_lossy().into_owned();
    enum Role {
        Leader(watch::Sender<bool>),
        Follower(watch::Receiver<bool>),
    }
    let role = {
        let mut guard = bridge.inflight_thumbs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = guard.get(&key) {
            Role::Follower(existing.clone())
        } else {
            let (tx, rx) = watch::channel(false);
            guard.insert(key.clone(), rx);
            Role::Leader(tx)
        }
    };

    let tx = match role {
        Role::Follower(mut rx) => {
            let _ = rx.wait_for(|&done| done).await;
            if let Ok(bytes) = tokio::fs::read(&cache_path).await {
                return write_binary(writer, 200, "OK", "image/jpeg", &bytes, is_head).await;
            }
            return write_status(writer, 404, "Not Found", &[]).await;
        }
        Role::Leader(tx) => tx,
    };

    let fetch_res = async {
        let _permit = bridge.thumb_sem.acquire().await.ok()?;
        let (images, videos, audio_albums) = match kind {
            "image" => (
                vec![SspImageFile {
                    media_id: Some(id),
                    path: (!remote_path.is_empty()).then(|| remote_path.to_owned()),
                    ..Default::default()
                }],
                vec![],
                vec![],
            ),
            "video" => (
                vec![],
                vec![SspVideoFile {
                    media_id: Some(id),
                    path: (!remote_path.is_empty()).then(|| remote_path.to_owned()),
                    ..Default::default()
                }],
                vec![],
            ),
            "audio_album" => (
                vec![],
                vec![],
                vec![SspAudioAlbum {
                    album_id: Some(id),
                    album_path: (!remote_path.is_empty()).then(|| remote_path.to_owned()),
                    ..Default::default()
                }],
            ),
            _ => return None,
        };

        let resp = session.get_thumbnails(images, videos, audio_albums).await.ok()?;
        let bytes = match kind {
            "image" => resp.image.into_iter().next().and_then(|i| i.thumbnail),
            "video" => resp.video.into_iter().next().and_then(|v| v.thumbnail),
            "audio_album" => resp.audio_album.into_iter().next().and_then(|a| a.thumbnail),
            _ => None,
        }?;

        if bytes.is_empty() {
            return None;
        }
        if let Err(e) = write_cache_atomic(&cache_path, &bytes).await {
            eprintln!(
                "[media_bridge] failed writing thumb cache {}: {e}",
                cache_path.display()
            );
        }
        Some(bytes)
    }
    .await;

    {
        let mut guard = bridge.inflight_thumbs.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove(&key);
    }
    let _ = tx.send(true);

    match fetch_res {
        Some(bytes) => write_binary(writer, 200, "OK", "image/jpeg", &bytes, is_head).await,
        None => write_status(writer, 404, "Not Found", &[]).await,
    }
}

struct LazyHttp200Writer<'a> {
    inner: &'a mut tokio::net::tcp::OwnedWriteHalf,
    pending_header: Vec<u8>,
    header_offset: usize,
}

impl<'a> LazyHttp200Writer<'a> {
    fn new(inner: &'a mut tokio::net::tcp::OwnedWriteHalf, header: String) -> Self {
        Self {
            inner,
            pending_header: header.into_bytes(),
            header_offset: 0,
        }
    }

    fn header_flushed(&self) -> bool {
        self.header_offset > 0
    }
}

impl AsyncWrite for LazyHttp200Writer<'_> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        while self.header_offset < self.pending_header.len() {
            let offset = self.header_offset;
            let header_len = self.pending_header.len();
            let res = {
                let this = &mut *self;
                Pin::new(&mut *this.inner).poll_write(cx, &this.pending_header[offset..header_len])
            };
            match res {
                Poll::Ready(Ok(0)) => {
                    return Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::WriteZero,
                        "failed writing HTTP header",
                    )));
                }
                Poll::Ready(Ok(n)) => self.header_offset += n,
                Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Pin::new(&mut *self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_shutdown(cx)
    }
}

async fn serve_stream(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    bridge: &MediaBridge,
    params: &HashMap<String, String>,
    range_header: Option<&str>,
    is_head: bool,
) -> anyhow::Result<()> {
    let Some(remote_path) = params.get("path").filter(|s| !s.is_empty()) else {
        return write_status(writer, 400, "Bad Request", &[]).await;
    };
    let declared_size = params.get("size").and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
    let mime = params
        .get("mime")
        .map(String::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or("application/octet-stream");

    let session = {
        let guard = bridge.session_slot.read().await;
        guard.clone()
    };
    let Some(session) = session else {
        return write_status(writer, 503, "Service Unavailable", &[]).await;
    };

    if declared_size > 0 {
        if let Some(rh) = range_header {
            if let Some((start, end)) = parse_http_range(rh, declared_size) {
                let len = end - start + 1;
                if is_head {
                    let hdr = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{end}/{declared_size}\r\nContent-Length: {len}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
                    );
                    writer.write_all(hdr.as_bytes()).await?;
                    return Ok(());
                }
                let mut buf = Vec::with_capacity(len.min(MAX_STREAM_CHUNK_BYTES) as usize);
                match session.download_to_writer(remote_path, start, len, &mut buf, |_, _| {}).await {
                    Ok(_) => {
                        let actual_len = buf.len() as u64;
                        let actual_end = start + actual_len.saturating_sub(1);
                        let hdr = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\nContent-Range: bytes {start}-{actual_end}/{declared_size}\r\nContent-Length: {actual_len}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n"
                        );
                        writer.write_all(hdr.as_bytes()).await?;
                        writer.write_all(&buf).await?;
                        writer.flush().await?;
                        return Ok(());
                    }
                    Err(_) => return write_status(writer, 502, "Bad Gateway", &[]).await,
                }
            } else {
                return write_status(writer, 416, "Range Not Satisfiable", &[]).await;
            }
        }
    }

    if is_head {
        let len_line = if declared_size > 0 {
            format!("Content-Length: {declared_size}\r\n")
        } else {
            String::new()
        };
        let hdr = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\n{len_line}Cache-Control: no-store\r\nConnection: close\r\n\r\n"
        );
        writer.write_all(hdr.as_bytes()).await?;
        return Ok(());
    }

    let len_line = if declared_size > 0 {
        format!("Content-Length: {declared_size}\r\n")
    } else {
        String::new()
    };
    let hdr = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nAccept-Ranges: bytes\r\n{len_line}Cache-Control: no-store\r\nConnection: close\r\n\r\n"
    );
    let mut lazy_writer = LazyHttp200Writer::new(writer, hdr);
    match session
        .download_to_writer(remote_path, 0, 0, &mut lazy_writer, |_, _| {})
        .await
    {
        Ok(_) => {
            if !lazy_writer.header_flushed() {
                let hdr_bytes = lazy_writer.pending_header.clone();
                writer.write_all(&hdr_bytes).await?;
            }
            writer.flush().await?;
            Ok(())
        }
        Err(_) => {
            if !lazy_writer.header_flushed() {
                write_status(writer, 404, "Not Found", &[]).await
            } else {
                Ok(())
            }
        }
    }
}

async fn write_cache_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = path.with_extension(format!("jpg.{}.tmp", Uuid::new_v4()));
    tokio::fs::write(&tmp, bytes).await?;
    tokio::fs::rename(&tmp, path).await
}

async fn write_status(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    code: u16,
    reason: &str,
    body: &[u8],
) -> anyhow::Result<()> {
    let hdr = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    writer.write_all(hdr.as_bytes()).await?;
    if !body.is_empty() {
        writer.write_all(body).await?;
    }
    writer.flush().await?;
    Ok(())
}

async fn write_binary(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    code: u16,
    reason: &str,
    content_type: &str,
    body: &[u8],
    is_head: bool,
) -> anyhow::Result<()> {
    let hdr = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {content_type}\r\nAccept-Ranges: bytes\r\nContent-Length: {}\r\nCache-Control: public, max-age=86400\r\nConnection: close\r\n\r\n",
        body.len()
    );
    writer.write_all(hdr.as_bytes()).await?;
    if !is_head && !body.is_empty() {
        writer.write_all(body).await?;
    }
    writer.flush().await?;
    Ok(())
}

pub fn parse_http_range(header: &str, total: u64) -> Option<(u64, u64)> {
    if total == 0 {
        return None;
    }
    let spec = header.strip_prefix("bytes=")?.trim();
    let first = spec.split(',').next()?.trim();
    let (start_s, end_s) = first.split_once('-')?;
    if start_s.is_empty() {
        let suffix = end_s.parse::<u64>().ok()?;
        if suffix == 0 {
            return None;
        }
        let capped_suffix = suffix.min(MAX_STREAM_CHUNK_BYTES);
        let start = total.saturating_sub(capped_suffix);
        Some((start, total - 1))
    } else {
        let start = start_s.parse::<u64>().ok()?;
        if start >= total {
            return None;
        }
        let requested_end = if end_s.is_empty() {
            total - 1
        } else {
            end_s.parse::<u64>().ok()?.min(total - 1)
        };
        if requested_end < start {
            return None;
        }
        // Cap single HTTP 206 chunk to MAX_STREAM_CHUNK_BYTES (2 MiB) so seeks never buffer gigabytes in RAM.
        let end = requested_end.min(start.saturating_add(MAX_STREAM_CHUNK_BYTES - 1));
        Some((start, end))
    }
}

pub fn parse_query(query: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        map.insert(url_decode(k), url_decode(v));
    }
    map
}

pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for b in input.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(*b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn url_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        } else if bytes[i] == b'+' {
            out.push(b' ');
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn hex_sha256_prefix(s: &str) -> String {
    let digest = Sha256::digest(s.as_bytes());
    digest[..8].iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_encode_decode_roundtrip_unicode_paths() {
        let p = "/storage/emulated/0/DCIM/Camera/照片 2026+1.jpg";
        let enc = url_encode(p);
        assert!(!enc.contains(' '));
        assert_eq!(url_decode(&enc), p);
    }

    #[test]
    fn http_range_parsing_caps_open_ranges() {
        assert_eq!(
            parse_http_range("bytes=0-", 10_000_000),
            Some((0, MAX_STREAM_CHUNK_BYTES - 1))
        );
        assert_eq!(
            parse_http_range("bytes=0-99999999", 100_000_000),
            Some((0, MAX_STREAM_CHUNK_BYTES - 1))
        );
        assert_eq!(
            parse_http_range("bytes=-10000000", 100_000_000),
            Some((100_000_000 - MAX_STREAM_CHUNK_BYTES, 99_999_999))
        );
        assert_eq!(parse_http_range("bytes=100-499", 1000), Some((100, 499)));
        assert_eq!(parse_http_range("bytes=-200", 1000), Some((800, 999)));
        assert_eq!(parse_http_range("bytes=1000-", 1000), None);
    }

    #[tokio::test]
    async fn inflight_watch_channel_has_no_lost_wakeup() {
        let (tx, mut rx) = watch::channel(false);
        // Simulate leader finishing BEFORE follower calls wait_for.
        let _ = tx.send(true);
        tokio::time::timeout(Duration::from_millis(200), rx.wait_for(|&done| done))
            .await
            .expect("follower should resolve immediately without missing wakeup")
            .unwrap();
    }
}
