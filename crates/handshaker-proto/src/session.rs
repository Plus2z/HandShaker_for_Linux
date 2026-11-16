use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use prost::Message;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::timeout;

use crate::crypto::{gzip_compress_text, gzip_decompress_text, RsaIdentity, RSA_SIGNATURE_LEN};
use crate::error::{ProtoError, Result};
use crate::framing::{
    encode_outbound_frame, DemuxEvent, FrameFlag, InboundDemuxer, MAX_CHUNK_BODY_LEN,
};
use crate::proto::{
    SspAudioAlbum, SspAudioLibraryChange, SspCancelRequest, SspClearClipboardRequest,
    SspClearClipboardResponse,
    SspClipboard, SspClipboardChange, SspCreateFolderRequest, SspCreateFolderResponse,
    SspDataRange, SspDeleteClipboardRequest, SspDeleteClipboardResponse, SspDeleteFileRequest,
    SspDeleteFileResponse, SspDownloadFileRequest, SspDownloadFileResponseHeader, SspFile,
    SspFileChange, SspFileExistRequest, SspFileExistResponse, SspGetAudioLibraryRequest,
    SspGetAudioLibraryResponse, SspGetClipboardRequest, SspGetClipboardResponse,
    SspGetDeviceInfoRequest, SspGetDeviceInfoResponse, SspGetDirFilesRequest,
    SspGetDirFilesResponse, SspGetFileCountRequest, SspGetFileCountResponse,
    SspGetPhotoLibraryRequest, SspGetPhotoLibraryResponse, SspGetThumbnailRequest,
    SspGetThumbnailResponse, SspGetVideoLibraryRequest, SspGetVideoLibraryResponse,
    SspHandShakeRequest01, SspHandShakeRequest02, SspHandShakeResponse01,
    SspHandShakeResponse02, SspHandShakeTrustType, SspHeartBeatRequest, SspHeartBeatResponse,
    SspImageFile, SspMonitorFolderRequest, SspMonitorFolderResponse,
    SspMonitorFolderResponseHeader, SspPhotoLibraryChange, SspPostClipboardRequest,
    SspPostClipboardResponse, SspQuitRequest, SspRenameFileRequest, SspRenameFileResponse,
    SspRequest, SspRequestType, SspUploadFileRequest, SspUploadFileResponse,
    SspUploadFileResponseHeader, SspVideoFile, SspVideoLibraryChange,
};
use crate::transport::{TransportKind, TransportLink};
use crate::trust::IdentityStore;

const DEFAULT_RPC_TIMEOUT: Duration = Duration::from_secs(20);
const DEFAULT_TRUST_TIMEOUT: Duration = Duration::from_secs(60);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(4);
const PROTOCOL_VERSION: &str = "2.1.0";
const APP_VERSION: &str = "2.5.0";
const MIN_CLIENT_VERSION: &str = "1";
const HOST_APP_VERSION_CODE: u32 = 333;

/// Configuration applied when initiating a client handshake.
#[derive(Clone, Debug)]
pub struct ConnectConfig {
    pub host_name: String,
    pub host_model: String,
    pub trust_timeout: Duration,
    pub rpc_timeout: Duration,
    pub enable_heartbeat: bool,
    pub heartbeat_timeout_secs: u64,
}

impl Default for ConnectConfig {
    fn default() -> Self {
        let host_name = std::env::var("HOSTNAME")
            .ok()
            .filter(|s| !s.is_empty())
            .or_else(|| std::fs::read_to_string("/etc/hostname").ok().map(|s| s.trim().to_owned()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Linux-HandShaker".to_owned());

        Self {
            host_name,
            host_model: "Linux".to_owned(),
            trust_timeout: DEFAULT_TRUST_TIMEOUT,
            rpc_timeout: DEFAULT_RPC_TIMEOUT,
            enable_heartbeat: true,
            heartbeat_timeout_secs: 10,
        }
    }
}

/// Summary returned upon a completed Handshake 01/02 exchange.
#[derive(Clone, Debug)]
pub struct HandshakeInfo {
    pub device_uuid: String,
    pub device_name: String,
    pub apk_version: String,
    pub apk_version_name: String,
    pub trust_type: i32,
    pub transport: TransportKind,
    pub transport_label: String,
    pub is_smartisan_device: bool,
}

/// Decoded clipboard entry with timestamp (ms) and plain UTF-8 text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardEntry {
    pub text: String,
    pub timestamp_ms: i64,
    pub raw_gzip: Vec<u8>,
}

/// Unsolicited push & lifecycle notifications emitted by an active [`Session`].
#[derive(Clone, Debug)]
pub enum SessionEvent {
    TrustWaiting {
        device_uuid: String,
        device_name: String,
    },
    ClipboardChanged(Vec<ClipboardEntry>),
    PhotoLibChanged {
        added: Vec<SspImageFile>,
        deleted: Vec<SspImageFile>,
    },
    VideoLibChanged {
        added: Vec<SspVideoFile>,
        deleted: Vec<SspVideoFile>,
        updated: Vec<SspVideoFile>,
    },
    AudioLibChanged(SspAudioLibraryChange),
    FolderChanged(SspMonitorFolderResponse),
    FileChanged(SspFileChange),
    RemoteCancelled {
        session_id: u64,
        error_code: Option<i32>,
    },
    QuitRequested,
    Disconnected {
        reason: String,
    },
}

#[derive(Debug)]
enum DownloadFrame {
    Header(Box<SspDownloadFileResponseHeader>),
    Data(Bytes),
    RemoteError(ProtoError),
}

enum PendingTarget {
    Rpc(mpsc::Sender<Bytes>),
    Download(mpsc::UnboundedSender<DownloadFrame>),
}

struct SessionInner {
    tx: mpsc::Sender<Bytes>,
    rsa: RsaIdentity,
    next_sid: AtomicU32,
    pending: Arc<StdMutex<HashMap<u32, PendingTarget>>>,
    download_sids: Arc<StdMutex<HashSet<u32>>>,
    events_tx: broadcast::Sender<SessionEvent>,
    disconnected: Arc<AtomicBool>,
    rpc_timeout: Duration,
    handshake: HandshakeInfo,
}

/// Active multiplexed HandShaker client session over Wi-Fi TCP, USB AOA, or Mock transport.
#[derive(Clone)]
pub struct Session {
    inner: Arc<SessionInner>,
}

impl Session {
    /// Connects over an already-established [`TransportLink`] and completes Handshake 01/02.
    pub async fn connect_with_link(
        link: TransportLink,
        store: &mut IdentityStore,
        config: ConnectConfig,
    ) -> Result<Self> {
        let mut rx = link.rx;
        let tx = link.tx;
        let kind = link.kind;
        let label = link.label;

        let pending: Arc<StdMutex<HashMap<u32, PendingTarget>>> =
            Arc::new(StdMutex::new(HashMap::new()));
        let download_sids: Arc<StdMutex<HashSet<u32>>> = Arc::new(StdMutex::new(HashSet::new()));
        let (events_tx, _) = broadcast::channel(64);
        let rsa = store.rsa().clone();
        let next_sid = AtomicU32::new(1);

        // Spawn reader loop immediately so handshake responses are routed cleanly.
        let pending_reader = Arc::clone(&pending);
        let download_sids_reader = Arc::clone(&download_sids);
        let events_reader = events_tx.clone();
        let disconnected_flag = Arc::new(AtomicBool::new(false));
        let disconnected_reader = Arc::clone(&disconnected_flag);

        tokio::spawn(async move {
            let mut demuxer = InboundDemuxer::new();
            let mut events_buf = Vec::new();
            let mut registered_downloads = HashSet::new();
            let disconnect_reason: String;

            loop {
                match rx.recv().await {
                    Some(Ok(chunk)) => {
                        // Sync newly registered download sids into our local demuxer before pushing wire bytes.
                        {
                            let mut ds = download_sids_reader
                                .lock()
                                .unwrap_or_else(|e| e.into_inner());
                            for sid in ds.drain() {
                                registered_downloads.insert(sid);
                                demuxer.register_download_sid(sid);
                            }
                        }

                        events_buf.clear();
                        if let Err(e) = demuxer.push_wire_bytes(&chunk, &mut events_buf) {
                            disconnect_reason = format!("Framing error: {e}");
                            break;
                        }

                        let mut idx = 0;
                        while idx < events_buf.len() {
                            let ev = events_buf[idx].clone();
                            idx += 1;

                            match ev {
                                DemuxEvent::ProtoMessage { sid, payload } => {
                                    if registered_downloads.contains(&sid) {
                                        match SspDownloadFileResponseHeader::decode(payload) {
                                            Ok(header) => {
                                                let ready = header.ready.unwrap_or(false);
                                                let data_len = if ready {
                                                    header
                                                        .range
                                                        .as_ref()
                                                        .and_then(|r| r.length)
                                                        .unwrap_or(0)
                                                } else {
                                                    0
                                                };

                                                send_download_event(
                                                    &pending_reader,
                                                    sid,
                                                    DownloadFrame::Header(Box::new(header)),
                                                );

                                                let mut tail = Vec::new();
                                                if let Err(e) = demuxer.set_download_data_len(
                                                    sid,
                                                    data_len,
                                                    &mut tail,
                                                ) {
                                                    send_download_event(
                                                        &pending_reader,
                                                        sid,
                                                        DownloadFrame::RemoteError(e),
                                                    );
                                                }
                                                if data_len == 0 {
                                                    registered_downloads.remove(&sid);
                                                }
                                                if !tail.is_empty() {
                                                    events_buf.extend(tail);
                                                }
                                            }
                                            Err(e) => {
                                                demuxer.finish_download_sid(sid);
                                                registered_downloads.remove(&sid);
                                                send_download_event(
                                                    &pending_reader,
                                                    sid,
                                                    DownloadFrame::RemoteError(e.into()),
                                                );
                                            }
                                        }
                                    } else {
                                        dispatch_proto_message(
                                            &pending_reader,
                                            &events_reader,
                                            sid,
                                            payload,
                                        );
                                    }
                                }
                                DemuxEvent::DownloadBytes { sid, data } => {
                                    send_download_event(
                                        &pending_reader,
                                        sid,
                                        DownloadFrame::Data(data),
                                    );
                                }
                            }
                        }
                    }
                    Some(Err(e)) => {
                        disconnect_reason = format!("Transport read failed: {e}");
                        break;
                    }
                    None => {
                        disconnect_reason = "Peer closed connection".to_owned();
                        break;
                    }
                }
            }

            disconnected_reader.store(true, Ordering::SeqCst);
            // Unblock all pending waiters cleanly on disconnect.
            pending_reader
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            let _ = events_reader.send(SessionEvent::Disconnected {
                reason: disconnect_reason,
            });
        });

        let hs_ctx = HandshakeCtx {
            rpc: RpcCtx {
                tx: &tx,
                rsa: &rsa,
                next_sid: &next_sid,
                pending: &pending,
                rpc_timeout: config.rpc_timeout,
                disconnected: &disconnected_flag,
            },
            events_tx: &events_tx,
            config: &config,
            kind,
            label,
        };
        let summary = if kind == TransportKind::UsbAoa {
            handshake_usb_aoa(hs_ctx).await?
        } else {
            handshake_wifi(hs_ctx, store).await?
        };

        let inner = Arc::new(SessionInner {
            tx,
            rsa,
            next_sid,
            pending,
            download_sids,
            events_tx,
            disconnected: Arc::clone(&disconnected_flag),
            rpc_timeout: config.rpc_timeout,
            handshake: summary,
        });

        let session = Self { inner };

        let inner_weak = Arc::downgrade(&session.inner);
        let disc_flag = disconnected_flag;

        if config.enable_heartbeat {
            let mut ev_rx = session.subscribe();
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(HEARTBEAT_INTERVAL);
                ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                ticker.tick().await;
                loop {
                    tokio::select! {
                        _ = ticker.tick() => {
                            let Some(strong) = inner_weak.upgrade() else { break; };
                            if strong.disconnected.load(Ordering::Relaxed) || disc_flag.load(Ordering::Relaxed) {
                                strong.disconnected.store(true, Ordering::SeqCst);
                                break;
                            }
                            let hb_session = Session { inner: strong };
                            if hb_session.heartbeat().await.is_err() {
                                break;
                            }
                        }
                        ev = ev_rx.recv() => {
                            if matches!(ev, Ok(SessionEvent::Disconnected { .. }) | Err(broadcast::error::RecvError::Closed)) {
                                if let Some(strong) = inner_weak.upgrade() {
                                    strong.disconnected.store(true, Ordering::SeqCst);
                                }
                                break;
                            }
                        }
                    }
                }
            });
        }

        Ok(session)
    }

    pub fn handshake_info(&self) -> &HandshakeInfo {
        &self.inner.handshake
    }

    pub fn is_connected(&self) -> bool {
        !self.inner.disconnected.load(Ordering::Relaxed)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.inner.events_tx.subscribe()
    }

    fn alloc_sid(&self) -> u32 {
        self.inner.next_sid.fetch_add(1, Ordering::Relaxed)
    }

    async fn signed_rpc<Req, Resp>(&self, req: &Req) -> Result<Resp>
    where
        Req: Message,
        Resp: Message + Default,
    {
        let rpc = RpcCtx {
            tx: &self.inner.tx,
            rsa: &self.inner.rsa,
            next_sid: &self.inner.next_sid,
            pending: &self.inner.pending,
            rpc_timeout: self.inner.rpc_timeout,
            disconnected: &self.inner.disconnected,
        };
        send_signed_rpc_raw(&rpc, req).await
    }

    pub async fn heartbeat(&self) -> Result<SspHeartBeatResponse> {
        let req = SspHeartBeatRequest {
            r#type: Some(SspRequestType::HeartBeatRequest as i32),
            host_timestamp: Some(unix_now_secs()),
        };
        self.signed_rpc(&req).await
    }

    pub async fn get_device_info(&self) -> Result<SspGetDeviceInfoResponse> {
        let req = build_get_device_info_request();
        self.signed_rpc(&req).await
    }

    pub async fn list_dir(&self, path: &str, maxdepth: u32) -> Result<SspGetDirFilesResponse> {
        let req = SspGetDirFilesRequest {
            r#type: Some(SspRequestType::GetDirFilesRequest as i32),
            dir: Some(simple_file(path, true)),
            maxdepth: Some(maxdepth.max(1)),
        };
        self.signed_rpc(&req).await
    }

    pub async fn file_exists(&self, path: &str) -> Result<bool> {
        let req = SspFileExistRequest {
            r#type: Some(SspRequestType::GetFileExistRequest as i32),
            file: Some(simple_file(path, false)),
        };
        let resp: SspFileExistResponse = self.signed_rpc(&req).await?;
        Ok(resp.exist.unwrap_or(false))
    }

    pub async fn get_file_count(
        &self,
        path: &str,
        maxdepth: u32,
        exclusions: Vec<String>,
    ) -> Result<u64> {
        let req = SspGetFileCountRequest {
            r#type: Some(SspRequestType::GetFileCountRequest as i32),
            dir: Some(simple_file(path, true)),
            maxdepth: Some(maxdepth.max(1)),
            exclusion_pattern: exclusions,
        };
        let resp: SspGetFileCountResponse = self.signed_rpc(&req).await?;
        Ok(resp.count.unwrap_or(0))
    }

    pub async fn create_folder(&self, path: &str) -> Result<()> {
        let req = SspCreateFolderRequest {
            r#type: Some(SspRequestType::GetCreateFolderRequest as i32),
            file: Some(simple_file(path, true)),
        };
        let resp: SspCreateFolderResponse = self.signed_rpc(&req).await?;
        if resp.succeed.unwrap_or(false) {
            Ok(())
        } else {
            Err(ProtoError::RemoteIo {
                code: resp.error_code,
                message: resp.error_message.unwrap_or_else(|| "create folder failed".into()),
            })
        }
    }

    pub async fn rename_file(&self, src_path: &str, dst_path: &str) -> Result<()> {
        let req = SspRenameFileRequest {
            r#type: Some(SspRequestType::GetRenameFileRequest as i32),
            source_file: Some(simple_file(src_path, false)),
            target_file: Some(simple_file(dst_path, false)),
        };
        let resp: SspRenameFileResponse = self.signed_rpc(&req).await?;
        if resp.succeed.unwrap_or(false) {
            Ok(())
        } else {
            Err(ProtoError::RemoteIo {
                code: resp.error_code,
                message: resp.error_message.unwrap_or_else(|| "rename failed".into()),
            })
        }
    }

    pub async fn delete_files(&self, paths: &[String], is_trash: bool) -> Result<()> {
        let files = paths
            .iter()
            .map(|p| SspFile {
                path: Some(p.clone()),
                is_trash: Some(is_trash),
                ..Default::default()
            })
            .collect();
        let req = SspDeleteFileRequest {
            r#type: Some(SspRequestType::GetDeleteFileRequest as i32),
            file: files,
            is_sync: Some(false),
            is_trash: Some(is_trash),
        };
        let resp: SspDeleteFileResponse = self.signed_rpc(&req).await?;
        if resp.succeed.unwrap_or(false) {
            Ok(())
        } else {
            Err(ProtoError::RemoteIo {
                code: resp.error_code,
                message: resp.error_message.unwrap_or_else(|| "delete failed".into()),
            })
        }
    }

    pub async fn monitor_folder(&self, path: &str, register: bool) -> Result<()> {
        let req = SspMonitorFolderRequest {
            r#type: Some(SspRequestType::MonitorFolderRequest as i32),
            file: Some(simple_file(path, true)),
            register: Some(register),
        };
        let resp: SspMonitorFolderResponseHeader = self.signed_rpc(&req).await?;
        if resp.succeed.unwrap_or(false) {
            Ok(())
        } else {
            Err(ProtoError::RemoteIo {
                code: None,
                message: resp
                    .error_message
                    .unwrap_or_else(|| "monitor folder failed".into()),
            })
        }
    }

    /// Downloads a remote file range (`length=0` fetches whole file) directly into an `AsyncWrite`.
    pub async fn download_to_writer<W, F>(
        &self,
        remote_path: &str,
        offset: u64,
        length: u64,
        writer: &mut W,
        on_progress: F,
    ) -> Result<SspDownloadFileResponseHeader>
    where
        W: AsyncWrite + Unpin + Send,
        F: FnMut(u64, u64) + Send,
    {
        self.download_to_writer_cancellable(remote_path, offset, length, writer, None, on_progress)
            .await
    }

    /// Cancellable download into an `AsyncWrite` (`cancel_rx` triggers `FrameFlag::Cancel` + `ProtoError::Cancelled`).
    pub async fn download_to_writer_cancellable<W, F>(
        &self,
        remote_path: &str,
        offset: u64,
        length: u64,
        writer: &mut W,
        mut cancel_rx: Option<watch::Receiver<bool>>,
        mut on_progress: F,
    ) -> Result<SspDownloadFileResponseHeader>
    where
        W: AsyncWrite + Unpin + Send,
        F: FnMut(u64, u64) + Send,
    {
        if !self.is_connected() {
            return Err(ProtoError::Disconnected);
        }
        let sid = self.alloc_sid();
        let req = SspDownloadFileRequest {
            r#type: Some(SspRequestType::GetDownloadFileRequest as i32),
            file: Some(simple_file(remote_path, false)),
            range: Some(SspDataRange {
                offset: Some(offset),
                length: Some(length),
            }),
            need_md5: Some(false),
            gzip: Some(false),
            is_sync: Some(false),
        };

        let proto_bytes = req.encode_to_vec();
        let sig = self.inner.rsa.sign_rpc_payload(&proto_bytes);
        let mut payload = Vec::with_capacity(RSA_SIGNATURE_LEN + proto_bytes.len());
        payload.extend_from_slice(&sig);
        payload.extend_from_slice(&proto_bytes);

        let (dl_tx, mut dl_rx) = mpsc::unbounded_channel::<DownloadFrame>();
        {
            self.inner
                .download_sids
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(sid);
            self.inner
                .pending
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(sid, PendingTarget::Download(dl_tx));
        }

        let frame = encode_outbound_frame(sid, FrameFlag::SignedRpc, &payload);
        if self.inner.tx.send(frame).await.is_err() {
            remove_pending(&self.inner.pending, sid);
            return Err(ProtoError::Disconnected);
        }

        let header = match timeout(self.inner.rpc_timeout, dl_rx.recv())
            .await
            .map_err(|_| {
                remove_pending(&self.inner.pending, sid);
                ProtoError::Timeout("download header")
            })? {
            Some(DownloadFrame::Header(h)) => *h,
            Some(DownloadFrame::RemoteError(e)) => {
                remove_pending(&self.inner.pending, sid);
                return Err(e);
            }
            Some(DownloadFrame::Data(_)) => {
                remove_pending(&self.inner.pending, sid);
                return Err(ProtoError::Framing("Unexpected data before download header".into()));
            }
            None => {
                remove_pending(&self.inner.pending, sid);
                return Err(ProtoError::Disconnected);
            }
        };

        if !header.ready.unwrap_or(false) {
            remove_pending(&self.inner.pending, sid);
            return Err(ProtoError::RemoteIo {
                code: header.error_code,
                message: format!("Remote refused download for {remote_path}"),
            });
        }

        let total = header
            .range
            .as_ref()
            .and_then(|r| r.length)
            .unwrap_or(0);
        let mut received: u64 = 0;
        on_progress(0, total);

        while received < total {
            let next_msg = tokio::select! {
                _ = async {
                    if let Some(rx) = &mut cancel_rx {
                        while !*rx.borrow() {
                            if rx.changed().await.is_err() {
                                std::future::pending::<()>().await;
                            }
                        }
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    let _ = self.cancel_sid(sid).await;
                    remove_pending(&self.inner.pending, sid);
                    return Err(ProtoError::Cancelled);
                }
                res = timeout(Duration::from_secs(60), dl_rx.recv()) => res,
            };

            match next_msg {
                Ok(Some(DownloadFrame::Data(chunk))) => {
                    writer.write_all(&chunk).await?;
                    received += chunk.len() as u64;
                    on_progress(received, total);
                }
                Ok(Some(DownloadFrame::RemoteError(e))) => {
                    remove_pending(&self.inner.pending, sid);
                    return Err(e);
                }
                Ok(Some(DownloadFrame::Header(_))) => {
                    remove_pending(&self.inner.pending, sid);
                    return Err(ProtoError::Framing("Duplicate download header".into()));
                }
                Ok(None) => {
                    remove_pending(&self.inner.pending, sid);
                    return Err(ProtoError::Disconnected);
                }
                Err(_) => {
                    let _ = self.cancel_sid(sid).await;
                    remove_pending(&self.inner.pending, sid);
                    return Err(ProtoError::Timeout("download body chunk"));
                }
            }
        }

        writer.flush().await?;
        remove_pending(&self.inner.pending, sid);
        Ok(header)
    }

    /// Convenience helper downloading `remote_path` into a local file path.
    pub async fn download_file<F>(
        &self,
        remote_path: &str,
        local_path: impl AsRef<Path>,
        on_progress: F,
    ) -> Result<SspDownloadFileResponseHeader>
    where
        F: FnMut(u64, u64) + Send,
    {
        if let Some(parent) = local_path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent).await?;
            }
        }
        let mut file = tokio::fs::File::create(local_path).await?;
        self.download_to_writer(remote_path, 0, 0, &mut file, on_progress)
            .await
    }

    /// Uploads `file_size` bytes from an `AsyncRead` to `remote_path` using `flag=1` header
    /// followed by `flag=3` raw binary chunks (`<= 32761B` each) and awaits `SSPUploadFileResponse`.
    pub async fn upload_from_reader<R, F>(
        &self,
        reader: &mut R,
        file_size: u64,
        remote_path: &str,
        mtime_secs: u64,
        on_progress: F,
    ) -> Result<SspUploadFileResponse>
    where
        R: AsyncRead + Unpin + Send,
        F: FnMut(u64, u64) + Send,
    {
        self.upload_from_reader_cancellable(
            reader,
            file_size,
            remote_path,
            mtime_secs,
            None,
            on_progress,
        )
        .await
    }

    /// Cancellable upload from an `AsyncRead`.
    pub async fn upload_from_reader_cancellable<R, F>(
        &self,
        reader: &mut R,
        file_size: u64,
        remote_path: &str,
        mtime_secs: u64,
        cancel_rx: Option<watch::Receiver<bool>>,
        mut on_progress: F,
    ) -> Result<SspUploadFileResponse>
    where
        R: AsyncRead + Unpin + Send,
        F: FnMut(u64, u64) + Send,
    {
        if !self.is_connected() {
            return Err(ProtoError::Disconnected);
        }
        let sid = self.alloc_sid();
        let req = SspUploadFileRequest {
            r#type: Some(SspRequestType::GetUploadFileRequestHeader as i32),
            file: Some(SspFile {
                path: Some(remote_path.to_owned()),
                file_size: Some(file_size),
                created_timestamp: Some(mtime_secs),
                modified_timestamp: Some(mtime_secs),
                is_directory: Some(false),
                ..Default::default()
            }),
            data_md5: None,
            gzip: Some(false),
            is_sync: Some(false),
        };

        let proto_bytes = req.encode_to_vec();
        let sig = self.inner.rsa.sign_rpc_payload(&proto_bytes);
        let mut payload = Vec::with_capacity(RSA_SIGNATURE_LEN + proto_bytes.len());
        payload.extend_from_slice(&sig);
        payload.extend_from_slice(&proto_bytes);

        let mut rx = register_rpc(&self.inner.pending, sid);
        let header_frame = encode_outbound_frame(sid, FrameFlag::SignedRpc, &payload);
        if self.inner.tx.send(header_frame).await.is_err() {
            remove_pending(&self.inner.pending, sid);
            return Err(ProtoError::Disconnected);
        }

        let raw_ack = timeout(self.inner.rpc_timeout, rx.recv())
            .await
            .map_err(|_| {
                remove_pending(&self.inner.pending, sid);
                ProtoError::Timeout("upload ready response")
            })?
            .ok_or(ProtoError::Disconnected)?;
        let ack = SspUploadFileResponseHeader::decode(raw_ack)?;
        if !ack.ready.unwrap_or(false) {
            remove_pending(&self.inner.pending, sid);
            return Err(ProtoError::RemoteIo {
                code: ack.error_code,
                message: format!("Remote refused upload for {remote_path}"),
            });
        }

        on_progress(0, file_size);
        if file_size == 0 {
            // Send an explicit 0-byte UploadChunk frame so Android SspFileTransferTask evaluates 0 == 0.
            let zero_frame = encode_outbound_frame(sid, FrameFlag::UploadChunk, &[]);
            if self.inner.tx.send(zero_frame).await.is_err() {
                remove_pending(&self.inner.pending, sid);
                return Err(ProtoError::Disconnected);
            }
        } else {
            let mut sent: u64 = 0;
            let mut buf = vec![0u8; MAX_CHUNK_BODY_LEN];
            while sent < file_size {
                if cancel_rx.as_ref().is_some_and(|c| *c.borrow()) {
                    let _ = self.cancel_sid(sid).await;
                    remove_pending(&self.inner.pending, sid);
                    return Err(ProtoError::Cancelled);
                }
                let want = ((file_size - sent) as usize).min(MAX_CHUNK_BODY_LEN);
                let n = reader.read(&mut buf[..want]).await?;
                if n == 0 {
                    let _ = self.cancel_sid(sid).await;
                    remove_pending(&self.inner.pending, sid);
                    return Err(ProtoError::InvalidArgument(format!(
                        "Upload source hit unexpected EOF after {sent}/{file_size} bytes"
                    )));
                }
                let data_frame = encode_outbound_frame(sid, FrameFlag::UploadChunk, &buf[..n]);
                if self.inner.tx.send(data_frame).await.is_err() {
                    remove_pending(&self.inner.pending, sid);
                    return Err(ProtoError::Disconnected);
                }
                sent += n as u64;
                on_progress(sent, file_size);
            }
        }

        let raw_done = timeout(Duration::from_secs(60), rx.recv())
            .await
            .map_err(|_| {
                remove_pending(&self.inner.pending, sid);
                ProtoError::Timeout("upload completion ack")
            })?
            .ok_or(ProtoError::Disconnected)?;
        remove_pending(&self.inner.pending, sid);

        let done = SspUploadFileResponse::decode(raw_done)?;
        if done.canceled.unwrap_or(false) {
            return Err(ProtoError::Cancelled);
        }
        if !done.succeed.unwrap_or(false) {
            return Err(ProtoError::RemoteIo {
                code: done.error_code,
                message: format!("Upload failed for {remote_path}"),
            });
        }
        Ok(done)
    }

    /// Convenience helper uploading a local file path to `remote_path`.
    pub async fn upload_file<F>(
        &self,
        local_path: impl AsRef<Path>,
        remote_path: &str,
        on_progress: F,
    ) -> Result<SspUploadFileResponse>
    where
        F: FnMut(u64, u64) + Send,
    {
        let meta = tokio::fs::metadata(local_path.as_ref()).await?;
        let file_size = meta.len();
        let mtime_secs = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
            .unwrap_or_else(unix_now_secs);
        let mut file = tokio::fs::File::open(local_path).await?;
        self.upload_from_reader(&mut file, file_size, remote_path, mtime_secs, on_progress)
            .await
    }

    /// Cancels an active transfer `target_sid` (`flag=2` with 4-byte `u32BE` payload).
    pub async fn cancel_sid(&self, target_sid: u32) -> Result<()> {
        let sid = self.alloc_sid();
        let frame = encode_outbound_frame(sid, FrameFlag::Cancel, &target_sid.to_be_bytes());
        self.inner
            .tx
            .send(frame)
            .await
            .map_err(|_| ProtoError::Disconnected)
    }

    /// Sends clean session quit (`flag=5`).
    pub async fn quit(&self) -> Result<()> {
        let sid = self.alloc_sid();
        let req = SspQuitRequest {
            r#type: Some(SspRequestType::QuitRequest as i32),
        };
        let frame = encode_outbound_frame(sid, FrameFlag::Quit, &req.encode_to_vec());
        let _ = self.inner.tx.send(frame).await;
        self.inner.disconnected.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub async fn get_photo_library(&self) -> Result<SspGetPhotoLibraryResponse> {
        let req = SspGetPhotoLibraryRequest {
            r#type: Some(SspRequestType::GetPhotoLibRequest as i32),
        };
        self.signed_rpc(&req).await
    }

    pub async fn get_video_library(&self) -> Result<SspGetVideoLibraryResponse> {
        let req = SspGetVideoLibraryRequest {
            r#type: Some(SspRequestType::GetVideoLibRequest as i32),
        };
        self.signed_rpc(&req).await
    }

    pub async fn get_audio_library(&self) -> Result<SspGetAudioLibraryResponse> {
        let req = SspGetAudioLibraryRequest {
            r#type: Some(SspRequestType::GetAudioLibRequest as i32),
        };
        self.signed_rpc(&req).await
    }

    pub async fn get_thumbnails(
        &self,
        images: Vec<SspImageFile>,
        videos: Vec<SspVideoFile>,
        audio_albums: Vec<SspAudioAlbum>,
    ) -> Result<SspGetThumbnailResponse> {
        if images.is_empty() && videos.is_empty() && audio_albums.is_empty() {
            return Ok(SspGetThumbnailResponse::default());
        }
        let req = SspGetThumbnailRequest {
            r#type: Some(SspRequestType::GetThumbnailRequest as i32),
            image: images,
            video: videos,
            audio_album: audio_albums,
        };
        self.signed_rpc(&req).await
    }

    pub async fn get_clipboard(&self) -> Result<Vec<ClipboardEntry>> {
        let req = SspGetClipboardRequest {
            r#type: Some(SspRequestType::GetClipboardRequest as i32),
        };
        let resp: SspGetClipboardResponse = self.signed_rpc(&req).await?;
        Ok(decode_clipboard_entries(&resp.clipboard))
    }

    pub async fn post_clipboard(&self, text: &str) -> Result<bool> {
        let gz = gzip_compress_text(text)?;
        let req = SspPostClipboardRequest {
            r#type: Some(SspRequestType::PostClipboardRequest as i32),
            clipboard: SspClipboard {
                content: Some(gz),
                mstimestamp: Some(unix_now_millis()),
            },
        };
        let resp: SspPostClipboardResponse = self.signed_rpc(&req).await?;
        Ok(resp.succeed.unwrap_or(false))
    }

    pub async fn clear_clipboard(&self) -> Result<bool> {
        let req = SspClearClipboardRequest {
            r#type: Some(SspRequestType::ClearClipboardRequest as i32),
        };
        let resp: SspClearClipboardResponse = self.signed_rpc(&req).await?;
        Ok(resp.succeed.unwrap_or(false))
    }

    pub async fn delete_clipboard(&self, raw_gzip: Vec<u8>, timestamp_ms: i64) -> Result<bool> {
        let req = SspDeleteClipboardRequest {
            r#type: Some(SspRequestType::DeleteClipboardRequest as i32),
            clipboard: SspClipboard {
                content: Some(raw_gzip),
                mstimestamp: Some(timestamp_ms),
            },
        };
        let resp: SspDeleteClipboardResponse = self.signed_rpc(&req).await?;
        Ok(resp.succeed.unwrap_or(false))
    }
}

fn simple_file(path: &str, is_dir: bool) -> SspFile {
    SspFile {
        path: Some(path.to_owned()),
        is_directory: Some(is_dir),
        ..Default::default()
    }
}

fn unix_now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn unix_now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn decode_clipboard_entries(items: &[SspClipboard]) -> Vec<ClipboardEntry> {
    items
        .iter()
        .filter_map(|item| {
            let raw = item.content.as_ref()?;
            let text = gzip_decompress_text(raw).ok()?;
            Some(ClipboardEntry {
                text,
                timestamp_ms: item.mstimestamp.unwrap_or(0),
                raw_gzip: raw.clone(),
            })
        })
        .collect()
}

fn register_rpc(
    pending: &Arc<StdMutex<HashMap<u32, PendingTarget>>>,
    sid: u32,
) -> mpsc::Receiver<Bytes> {
    let (tx, rx) = mpsc::channel(8);
    pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(sid, PendingTarget::Rpc(tx));
    rx
}

fn remove_pending(pending: &Arc<StdMutex<HashMap<u32, PendingTarget>>>, sid: u32) {
    pending
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&sid);
}

fn send_download_event(
    pending: &Arc<StdMutex<HashMap<u32, PendingTarget>>>,
    sid: u32,
    item: DownloadFrame,
) {
    let tx = {
        let guard = pending.lock().unwrap_or_else(|e| e.into_inner());
        match guard.get(&sid) {
            Some(PendingTarget::Download(tx)) => Some(tx.clone()),
            _ => None,
        }
    };
    if let Some(tx) = tx {
        let _ = tx.send(item);
    }
}

fn dispatch_proto_message(
    pending: &Arc<StdMutex<HashMap<u32, PendingTarget>>>,
    events_tx: &broadcast::Sender<SessionEvent>,
    sid: u32,
    payload: Bytes,
) {
    let target = {
        let guard = pending.lock().unwrap_or_else(|e| e.into_inner());
        match guard.get(&sid) {
            Some(PendingTarget::Rpc(tx)) => Some(tx.clone()),
            _ => None,
        }
    };
    if let Some(tx) = target {
        let _ = tx.try_send(payload);
        return;
    }

    // Unsolicited server push (clipboard change, library change, remote cancel, quit)
    let Ok(header) = SspRequest::decode(payload.clone()) else {
        return;
    };
    let Some(req_type) = header.r#type else {
        return;
    };

    if req_type == SspRequestType::PhotoLibChange as i32 {
        if let Ok(msg) = SspPhotoLibraryChange::decode(payload) {
            let _ = events_tx.send(SessionEvent::PhotoLibChanged {
                added: msg.added_image,
                deleted: msg.deleted_image,
            });
        }
    } else if req_type == SspRequestType::VideoLibChange as i32 {
        if let Ok(msg) = SspVideoLibraryChange::decode(payload) {
            let _ = events_tx.send(SessionEvent::VideoLibChanged {
                added: msg.added_video,
                deleted: msg.deleted_video,
                updated: msg.updated_video,
            });
        }
    } else if req_type == SspRequestType::AudioLibChange as i32 {
        if let Ok(msg) = SspAudioLibraryChange::decode(payload) {
            let _ = events_tx.send(SessionEvent::AudioLibChanged(msg));
        }
    } else if req_type == SspRequestType::MonitorFolderResponse as i32 {
        if let Ok(msg) = SspMonitorFolderResponse::decode(payload) {
            let _ = events_tx.send(SessionEvent::FolderChanged(msg));
        }
    } else if req_type == SspRequestType::ClipboardChange as i32 {
        if let Ok(msg) = SspClipboardChange::decode(payload) {
            let _ = events_tx.send(SessionEvent::ClipboardChanged(decode_clipboard_entries(
                &msg.clipboard,
            )));
        }
    } else if req_type == SspRequestType::QuitRequest as i32 {
        let _ = events_tx.send(SessionEvent::QuitRequested);
    } else if req_type == SspRequestType::CancelRequest as i32 {
        if let Ok(msg) = SspCancelRequest::decode(payload) {
            if let Some(target_sid) = msg.session_id.and_then(|s| u32::try_from(s).ok()) {
                send_download_event(
                    pending,
                    target_sid,
                    DownloadFrame::RemoteError(ProtoError::Cancelled),
                );
            }
            let _ = events_tx.send(SessionEvent::RemoteCancelled {
                session_id: msg.session_id.unwrap_or(0),
                error_code: msg.error_code,
            });
        }
    } else if req_type == SspRequestType::FileChange as i32 {
        if let Ok(msg) = SspFileChange::decode(payload) {
            let _ = events_tx.send(SessionEvent::FileChanged(msg));
        }
    }
}

struct RpcCtx<'a> {
    tx: &'a mpsc::Sender<Bytes>,
    rsa: &'a RsaIdentity,
    next_sid: &'a AtomicU32,
    pending: &'a Arc<StdMutex<HashMap<u32, PendingTarget>>>,
    rpc_timeout: Duration,
    disconnected: &'a AtomicBool,
}

struct HandshakeCtx<'a> {
    rpc: RpcCtx<'a>,
    events_tx: &'a broadcast::Sender<SessionEvent>,
    config: &'a ConnectConfig,
    kind: TransportKind,
    label: String,
}

fn build_get_device_info_request() -> SspGetDeviceInfoRequest {
    SspGetDeviceInfoRequest {
        r#type: Some(SspRequestType::GetDeviceInfoRequest as i32),
        host_timestamp: Some(unix_now_secs()),
        host_smart_sync_protocol_version: Some(PROTOCOL_VERSION.to_owned()),
        need_device_info_callback: Some(true),
        need_photo_library_callback: Some(true),
        need_audio_library_callback: Some(true),
        need_video_library_callback: Some(true),
        host_app_version: Some(APP_VERSION.to_owned()),
        host_min_client_version: Some(MIN_CLIENT_VERSION.to_owned()),
        host_type: Some(1),
        host_app_version_code: Some(HOST_APP_VERSION_CODE),
    }
}

async fn send_signed_rpc_raw<Req, Resp>(ctx: &RpcCtx<'_>, req: &Req) -> Result<Resp>
where
    Req: Message,
    Resp: Message + Default,
{
    if ctx.disconnected.load(Ordering::Relaxed) {
        return Err(ProtoError::Disconnected);
    }
    let sid = ctx.next_sid.fetch_add(1, Ordering::Relaxed);
    let proto_bytes = req.encode_to_vec();
    let sig = ctx.rsa.sign_rpc_payload(&proto_bytes);
    let mut payload = Vec::with_capacity(RSA_SIGNATURE_LEN + proto_bytes.len());
    payload.extend_from_slice(&sig);
    payload.extend_from_slice(&proto_bytes);

    let mut rx = register_rpc(ctx.pending, sid);
    let frame = encode_outbound_frame(sid, FrameFlag::SignedRpc, &payload);
    if ctx.tx.send(frame).await.is_err() {
        remove_pending(ctx.pending, sid);
        ctx.disconnected.store(true, Ordering::SeqCst);
        return Err(ProtoError::Disconnected);
    }

    let raw = timeout(ctx.rpc_timeout, rx.recv())
        .await
        .map_err(|_| {
            remove_pending(ctx.pending, sid);
            ProtoError::Timeout("RPC response")
        })?
        .ok_or_else(|| {
            remove_pending(ctx.pending, sid);
            ProtoError::Disconnected
        })?;
    remove_pending(ctx.pending, sid);

    if raw.as_ref() == b"rsa verify failed" {
        return Err(ProtoError::Crypto("Android reported rsa verify failed".into()));
    }
    Ok(Resp::decode(raw)?)
}

async fn handshake_usb_aoa(ctx: HandshakeCtx<'_>) -> Result<HandshakeInfo> {
    let sid1 = ctx.rpc.next_sid.fetch_add(1, Ordering::Relaxed);
    let usb_payload = ctx.rpc.rsa.usb_handshake_payload()?;
    let mut hs1_rx = register_rpc(ctx.rpc.pending, sid1);
    let frame1 = encode_outbound_frame(sid1, FrameFlag::Handshake, &usb_payload);
    if ctx.rpc.tx.send(frame1).await.is_err() {
        remove_pending(ctx.rpc.pending, sid1);
        return Err(ProtoError::Disconnected);
    }

    let initial_hs_timeout = ctx.config.rpc_timeout.min(Duration::from_secs(3));
    let first_raw = timeout(initial_hs_timeout, hs1_rx.recv())
        .await
        .map_err(|_| {
            remove_pending(ctx.rpc.pending, sid1);
            ProtoError::Timeout("USB AOA handshake response")
        })?
        .ok_or_else(|| {
            remove_pending(ctx.rpc.pending, sid1);
            ProtoError::Disconnected
        })?;

    let raw_hs1 = if first_raw.as_ref() == b"locked" {
        let _ = ctx.events_tx.send(SessionEvent::TrustWaiting {
            device_uuid: String::new(),
            device_name: "USB Android Phone (Locked)".to_owned(),
        });
        timeout(ctx.config.trust_timeout, hs1_rx.recv())
            .await
            .map_err(|_| {
                remove_pending(ctx.rpc.pending, sid1);
                ProtoError::PhoneLocked
            })?
            .ok_or_else(|| {
                remove_pending(ctx.rpc.pending, sid1);
                ProtoError::Disconnected
            })?
    } else {
        first_raw
    };
    remove_pending(ctx.rpc.pending, sid1);

    let result_str = std::str::from_utf8(&raw_hs1).map_err(|e| {
        ProtoError::Crypto(format!("USB handshake response is not UTF-8: {e}"))
    })?;
    let decrypted = ctx.rpc.rsa.decrypt_handshake_result(result_str)?;
    if decrypted == "locked" {
        return Err(ProtoError::PhoneLocked);
    }
    if decrypted != "ok" {
        return Err(ProtoError::HandshakeRejected {
            trust_type: SspHandShakeTrustType::TrustNo as i32,
            reason: decrypted,
        });
    }

    let dev_req = build_get_device_info_request();
    let dev_info: SspGetDeviceInfoResponse = send_signed_rpc_raw(&ctx.rpc, &dev_req).await?;

    Ok(HandshakeInfo {
        device_uuid: dev_info.phone_id.filter(|s| !s.is_empty()).unwrap_or_default(),
        device_name: dev_info
            .phone_name
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Android Phone".to_owned()),
        apk_version: dev_info.apk_version.unwrap_or_default(),
        apk_version_name: dev_info.apk_version_name.unwrap_or_default(),
        trust_type: SspHandShakeTrustType::TrustAlways as i32,
        transport: ctx.kind,
        transport_label: ctx.label,
        is_smartisan_device: dev_info
            .smartisan_version
            .as_deref()
            .is_some_and(|s| !s.is_empty()),
    })
}

async fn handshake_wifi(
    ctx: HandshakeCtx<'_>,
    store: &mut IdentityStore,
) -> Result<HandshakeInfo> {
    // 1. Handshake 01
    let sid1 = ctx.rpc.next_sid.fetch_add(1, Ordering::Relaxed);
    let (md5_bytes, enckey) = ctx.rpc.rsa.enckey_and_md5()?;
    let hs1_req = SspHandShakeRequest01 {
        r#type: Some(SspRequestType::HandshakeRequest01 as i32),
        host_uuid: Some(store.host_uuid().to_owned()),
        host_name: Some(ctx.config.host_name.clone()),
        host_timestamp: Some(unix_now_secs()),
        host_smart_sync_protocol_version: Some(PROTOCOL_VERSION.to_owned()),
        host_app_version: Some(APP_VERSION.to_owned()),
        host_min_client_version: Some(MIN_CLIENT_VERSION.to_owned()),
        md5: Some(md5_bytes.to_vec()),
        enckey: Some(enckey),
        host_model: Some(ctx.config.host_model.clone()),
        heartbeat_timeout_second: Some(ctx.config.heartbeat_timeout_secs),
    };
    let mut hs1_rx = register_rpc(ctx.rpc.pending, sid1);
    let frame1 = encode_outbound_frame(sid1, FrameFlag::Handshake, &hs1_req.encode_to_vec());
    if ctx.rpc.tx.send(frame1).await.is_err() {
        remove_pending(ctx.rpc.pending, sid1);
        return Err(ProtoError::Disconnected);
    }

    let raw_hs1 = timeout(ctx.config.rpc_timeout, hs1_rx.recv())
        .await
        .map_err(|_| {
            remove_pending(ctx.rpc.pending, sid1);
            ProtoError::Timeout("Handshake 01 response")
        })?
        .ok_or_else(|| {
            remove_pending(ctx.rpc.pending, sid1);
            ProtoError::Disconnected
        })?;
    remove_pending(ctx.rpc.pending, sid1);

    if raw_hs1.as_ref() == b"locked" {
        return Err(ProtoError::PhoneLocked);
    }
    let hs1 = SspHandShakeResponse01::decode(raw_hs1)?;
    let device_uuid = hs1.device_uuid.clone().unwrap_or_default();
    let device_name = hs1
        .device_name
        .clone()
        .unwrap_or_else(|| "Android Phone".to_owned());
    let saved_derived_key = store.derived_key_for(&device_uuid)?;

    // 2. Handshake 02 (supports sequential TRUST_WAITING -> user confirmation -> result)
    let sid2 = ctx.rpc.next_sid.fetch_add(1, Ordering::Relaxed);
    let hs2_req = SspHandShakeRequest02 {
        r#type: Some(SspRequestType::HandshakeRequest02 as i32),
        host_uuid: Some(store.host_uuid().to_owned()),
        derived_key: saved_derived_key,
        trust_type: Some(SspHandShakeTrustType::TrustWaiting as i32),
    };
    let mut hs2_rx = register_rpc(ctx.rpc.pending, sid2);
    let frame2 = encode_outbound_frame(sid2, FrameFlag::Handshake, &hs2_req.encode_to_vec());
    if ctx.rpc.tx.send(frame2).await.is_err() {
        remove_pending(ctx.rpc.pending, sid2);
        return Err(ProtoError::Disconnected);
    }

    let trust_deadline = tokio::time::Instant::now() + ctx.config.trust_timeout;
    let final_hs2 = loop {
        let remaining = trust_deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            remove_pending(ctx.rpc.pending, sid2);
            let _ = ctx
                .rpc
                .tx
                .send(encode_outbound_frame(sid2, FrameFlag::TrustCancel, &[]))
                .await;
            return Err(ProtoError::Timeout("phone trust confirmation"));
        }
        let raw_hs2 = timeout(remaining, hs2_rx.recv())
            .await
            .map_err(|_| {
                remove_pending(ctx.rpc.pending, sid2);
                ProtoError::Timeout("phone trust confirmation")
            })?
            .ok_or_else(|| {
                remove_pending(ctx.rpc.pending, sid2);
                ProtoError::Disconnected
            })?;

        let hs2 = SspHandShakeResponse02::decode(raw_hs2)?;
        let trust_type = hs2.trust_type.unwrap_or(0);
        if trust_type == SspHandShakeTrustType::TrustWaiting as i32 {
            let _ = ctx.events_tx.send(SessionEvent::TrustWaiting {
                device_uuid: device_uuid.clone(),
                device_name: device_name.clone(),
            });
            continue;
        }
        break hs2;
    };
    remove_pending(ctx.rpc.pending, sid2);

    let trust_type = final_hs2.trust_type.unwrap_or(0);
    let result_str = final_hs2.result.clone().unwrap_or_default();
    let decrypted = ctx.rpc.rsa.decrypt_handshake_result(&result_str)?;
    if decrypted == "locked" {
        return Err(ProtoError::PhoneLocked);
    }
    if decrypted != "ok" || trust_type == SspHandShakeTrustType::TrustNo as i32 {
        return Err(ProtoError::HandshakeRejected {
            trust_type,
            reason: decrypted,
        });
    }

    if trust_type == SspHandShakeTrustType::TrustAlways as i32 {
        if let Some(dk) = final_hs2.derived_key.as_ref().filter(|b| !b.is_empty()) {
            store.save_trusted_device(&device_uuid, &device_name, dk)?;
        }
    }

    Ok(HandshakeInfo {
        device_uuid,
        device_name,
        apk_version: hs1.apk_version.unwrap_or_default(),
        apk_version_name: hs1.apk_version_name.unwrap_or_default(),
        trust_type,
        transport: ctx.kind,
        transport_label: ctx.label,
        is_smartisan_device: hs1.is_smartisan_device.unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::{Buf, BytesMut};
    use tempfile::TempDir;

    use crate::crypto::{
        decode_usb_handshake_payload, encrypt_handshake_result, verify_rpc_signature,
    };
    use crate::framing::{encode_inbound_proto_chunks, OUTBOUND_HEADER_LEN};

    fn spawn_usb_mock_link(send_locked_first: bool) -> TransportLink {
        let (to_phone_tx, mut to_phone_rx) = mpsc::channel::<Bytes>(32);
        let (from_phone_tx, from_phone_rx) = mpsc::channel::<Result<Bytes>>(32);

        tokio::spawn(async move {
            let mut buf = BytesMut::new();
            let mut client_pub = None;

            loop {
                while buf.len() < OUTBOUND_HEADER_LEN {
                    let Some(chunk) = to_phone_rx.recv().await else {
                        return;
                    };
                    buf.extend_from_slice(&chunk);
                }
                let sid = (&buf[0..4]).get_u32();
                let flag = buf[4];
                let len = (&buf[5..9]).get_u32() as usize;
                while buf.len() < OUTBOUND_HEADER_LEN + len {
                    let Some(chunk) = to_phone_rx.recv().await else {
                        return;
                    };
                    buf.extend_from_slice(&chunk);
                }
                buf.advance(OUTBOUND_HEADER_LEN);
                let payload = buf.split_to(len).freeze();

                match flag {
                    0 => {
                        let pub_key = decode_usb_handshake_payload(&payload)
                            .expect("Android decoder.b.a(byte[]) must accept [md5 || enckey]");
                        if send_locked_first {
                            for chunk in encode_inbound_proto_chunks(sid, b"locked") {
                                let _ = from_phone_tx.send(Ok(chunk)).await;
                            }
                            tokio::time::sleep(Duration::from_millis(30)).await;
                        }
                        let encrypted_ok = encrypt_handshake_result(&pub_key, "ok").unwrap();
                        // Match Android Base64.DEFAULT (trailing newline).
                        let b64_with_nl = format!("{encrypted_ok}\n");
                        client_pub = Some(pub_key);
                        for chunk in encode_inbound_proto_chunks(sid, b64_with_nl.as_bytes()) {
                            let _ = from_phone_tx.send(Ok(chunk)).await;
                        }
                    }
                    1 => {
                        let sig = &payload[..RSA_SIGNATURE_LEN];
                        let proto_bytes = &payload[RSA_SIGNATURE_LEN..];
                        let pub_key = client_pub.as_ref().expect("handshake must complete first");
                        verify_rpc_signature(pub_key, sig, proto_bytes).unwrap();
                        let header = SspRequest::decode(proto_bytes).unwrap();
                        if header.r#type == Some(SspRequestType::GetDeviceInfoRequest as i32) {
                            let resp = SspGetDeviceInfoResponse {
                                r#type: Some(SspRequestType::GetDeviceInfoRequest as i32),
                                phone_id: Some("mock-usb-android-id".into()),
                                phone_name: Some("vivo X200 Pro mini USB".into()),
                                apk_version: Some("9".into()),
                                apk_version_name: Some("1.2.0-r9".into()),
                                root_path: Some("/storage/emulated/0".into()),
                                disk_size: Some(256_000_000_000),
                                used_disk_size: Some(64_000_000_000),
                                ..Default::default()
                            };
                            for chunk in encode_inbound_proto_chunks(sid, &resp.encode_to_vec()) {
                                let _ = from_phone_tx.send(Ok(chunk)).await;
                            }
                        }
                    }
                    5 => return,
                    _ => {}
                }
            }
        });

        TransportLink {
            kind: TransportKind::UsbAoa,
            label: "usb:001:018 (18d1:2d01 if0)".to_owned(),
            rx: from_phone_rx,
            tx: to_phone_tx,
        }
    }

    #[tokio::test]
    async fn usb_aoa_one_step_handshake_and_device_info_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let mut store = IdentityStore::load_or_create(tmp.path().join("trust.json"), true).unwrap();
        let link = spawn_usb_mock_link(false);
        let cfg = ConnectConfig {
            enable_heartbeat: false,
            ..ConnectConfig::default()
        };

        let session = Session::connect_with_link(link, &mut store, cfg)
            .await
            .unwrap();
        let hs = session.handshake_info();
        assert_eq!(hs.device_uuid, "mock-usb-android-id");
        assert_eq!(hs.device_name, "vivo X200 Pro mini USB");
        assert_eq!(hs.apk_version, "9");
        assert_eq!(hs.apk_version_name, "1.2.0-r9");
        assert_eq!(hs.transport, TransportKind::UsbAoa);
        session.quit().await.unwrap();
    }

    #[tokio::test]
    async fn usb_aoa_locked_then_unlocked_handshake_succeeds() {
        let tmp = TempDir::new().unwrap();
        let mut store = IdentityStore::load_or_create(tmp.path().join("trust.json"), true).unwrap();
        let link = spawn_usb_mock_link(true);
        let cfg = ConnectConfig {
            enable_heartbeat: false,
            rpc_timeout: Duration::from_secs(2),
            trust_timeout: Duration::from_secs(5),
            ..ConnectConfig::default()
        };

        let session = Session::connect_with_link(link, &mut store, cfg)
            .await
            .unwrap();
        assert_eq!(session.handshake_info().device_uuid, "mock-usb-android-id");
        session.quit().await.unwrap();
    }
}
