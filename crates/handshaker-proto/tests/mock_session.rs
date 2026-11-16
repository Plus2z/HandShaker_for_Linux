use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;

use bytes::{BufMut, Bytes, BytesMut};
use handshaker_proto::crypto::{
    decode_enckey_and_verify_md5, encrypt_handshake_result, gzip_compress_text,
    verify_rpc_signature, RSA_SIGNATURE_LEN,
};
use handshaker_proto::framing::{
    encode_inbound_proto_chunks, encode_raw_chunks, FrameFlag, OutboundFrameParser,
};
use handshaker_proto::proto::{
    SspClearClipboardResponse, SspClipboard, SspClipboardChange, SspCreateFolderResponse,
    SspDataRange, SspDeleteFileRequest, SspDeleteFileResponse, SspDownloadFileRequest,
    SspDownloadFileResponseHeader, SspFile, SspFileExistRequest, SspFileExistResponse,
    SspGetAudioLibraryResponse, SspGetDeviceInfoResponse, SspGetDirFilesRequest,
    SspGetDirFilesResponse, SspGetPhotoLibraryResponse, SspGetThumbnailRequest,
    SspGetThumbnailResponse, SspGetVideoLibraryResponse, SspHandShakeRequest01,
    SspHandShakeRequest02, SspHandShakeResponse01, SspHandShakeResponse02,
    SspHandShakeTrustType, SspHeartBeatResponse, SspImageAlbum, SspImageFile,
    SspPostClipboardRequest, SspPostClipboardResponse, SspRenameFileRequest,
    SspRenameFileResponse, SspRequest, SspRequestType, SspUploadFileRequest,
    SspUploadFileResponse, SspUploadFileResponseHeader, SspVideoFile, SspAudioFile,
    SspGetClipboardResponse,
};
use handshaker_proto::{
    ConnectConfig, IdentityStore, ProtoError, Session, SessionEvent, TransportKind, TransportLink,
};
use prost::Message;
use rsa::RsaPublicKey;
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::sync::{mpsc, Mutex};

#[derive(Clone, Copy)]
enum MockTrustPolicy {
    WaitingThenAlways,
    AlwaysImmediateIfKnown,
    RejectNo,
}

#[derive(Clone)]
struct MockState {
    files: HashMap<String, Vec<u8>>,
    dirs: HashSet<String>,
    clipboard: Vec<SspClipboard>,
    trusted_derived_keys: HashSet<Vec<u8>>,
}

impl Default for MockState {
    fn default() -> Self {
        let mut dirs = HashSet::new();
        dirs.insert("/storage/emulated/0".to_owned());
        dirs.insert("/storage/emulated/0/Download".to_owned());
        Self {
            files: HashMap::new(),
            dirs,
            clipboard: Vec::new(),
            trusted_derived_keys: HashSet::new(),
        }
    }
}

struct ActiveUpload {
    path: String,
    expected_size: u64,
    data: Vec<u8>,
}

fn spawn_mock_phone(
    state: Arc<Mutex<MockState>>,
    policy: MockTrustPolicy,
) -> (TransportLink, mpsc::Sender<(u32, Vec<u8>)>) {
    let (client_io, server_io) = tokio::io::duplex(512 * 1024);
    let (cr, cw) = tokio::io::split(client_io);
    let link = TransportLink::from_async_rw(cr, cw, TransportKind::Mock, "mock-android");
    let (push_tx, mut push_rx) = mpsc::channel::<(u32, Vec<u8>)>(16);

    tokio::spawn(async move {
        let _ = run_mock_server(server_io, state, policy, &mut push_rx).await;
    });

    (link, push_tx)
}

async fn write_chunks(writer: &mut tokio::io::WriteHalf<DuplexStream>, chunks: Vec<Bytes>) {
    for c in chunks {
        if writer.write_all(&c).await.is_err() {
            return;
        }
    }
    let _ = writer.flush().await;
}

async fn run_mock_server(
    io: DuplexStream,
    state: Arc<Mutex<MockState>>,
    policy: MockTrustPolicy,
    push_rx: &mut mpsc::Receiver<(u32, Vec<u8>)>,
) -> anyhow::Result<()> {
    let (mut reader, mut writer) = tokio::io::split(io);
    let mut parser = OutboundFrameParser::new();
    let mut read_buf = vec![0u8; 64 * 1024];
    let mut client_pub_key: Option<RsaPublicKey> = None;
    let mut uploads: HashMap<u32, ActiveUpload> = HashMap::new();

    loop {
        tokio::select! {
            push = push_rx.recv() => {
                if let Some((sid, proto_bytes)) = push {
                    write_chunks(&mut writer, encode_inbound_proto_chunks(sid, &proto_bytes)).await;
                }
            }
            n_res = reader.read(&mut read_buf) => {
                let n = match n_res {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                for frame in parser.push_bytes(&read_buf[..n]) {
                    if frame.flag == FrameFlag::Quit as u8 {
                        return Ok(());
                    }
                    if frame.flag == FrameFlag::Cancel as u8 {
                        if frame.payload.len() >= 4 {
                            let target = u32::from_be_bytes([frame.payload[0], frame.payload[1], frame.payload[2], frame.payload[3]]);
                            uploads.remove(&target);
                        }
                        continue;
                    }
                    if frame.flag == FrameFlag::UploadChunk as u8 {
                        if let Some(up) = uploads.get_mut(&frame.sid) {
                            up.data.extend_from_slice(&frame.payload);
                            if up.data.len() as u64 >= up.expected_size {
                                let completed = uploads.remove(&frame.sid).unwrap();
                                state.lock().await.files.insert(completed.path.clone(), completed.data);
                                let done = SspUploadFileResponse {
                                    r#type: Some(SspRequestType::GetUploadFileResponse as i32),
                                    file: Some(SspFile {
                                        path: Some(completed.path),
                                        file_size: Some(completed.expected_size),
                                        ..Default::default()
                                    }),
                                    canceled: Some(false),
                                    succeed: Some(true),
                                    error_code: None,
                                };
                                write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &done.encode_to_vec())).await;
                            }
                        }
                        continue;
                    }

                    if frame.flag == FrameFlag::Handshake as u8 {
                        let req_hdr = SspRequest::decode(frame.payload.clone())?;
                        let rtype = req_hdr.r#type.unwrap_or(0);
                        if rtype == SspRequestType::HandshakeRequest01 as i32 {
                            let hs1 = SspHandShakeRequest01::decode(frame.payload)?;
                            let pubk = decode_enckey_and_verify_md5(
                                hs1.md5.as_deref().unwrap_or_default(),
                                hs1.enckey.as_deref().unwrap_or_default(),
                            )?;
                            client_pub_key = Some(pubk);
                            let resp = SspHandShakeResponse01 {
                                r#type: Some(SspRequestType::HandshakeResponse01 as i32),
                                apk_version: Some("235".into()),
                                apk_version_name: Some("2.5.0".into()),
                                client_timestamp: Some(1_700_000_000),
                                client_smart_sync_protocol_version: Some("2.1.0".into()),
                                client_min_host_version: Some("1.0".into()),
                                device_uuid: Some("mock-vivo-uuid".into()),
                                device_name: Some("vivo X200 Pro mini".into()),
                                usb_serial: None,
                                is_smartisan_device: Some(false),
                                client_min_host_version_code: Some(1),
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::HandshakeRequest02 as i32 {
                            let hs2 = SspHandShakeRequest02::decode(frame.payload)?;
                            let pubk = client_pub_key.as_ref().expect("HS01 must precede HS02");
                            let dk_known = if let Some(k) = hs2.derived_key.as_ref() {
                                state.lock().await.trusted_derived_keys.contains(k)
                            } else {
                                false
                            };

                            match policy {
                                MockTrustPolicy::RejectNo => {
                                    let resp = SspHandShakeResponse02 {
                                        r#type: Some(SspRequestType::HandshakeResponse02 as i32),
                                        trust_type: Some(SspHandShakeTrustType::TrustNo as i32),
                                        device_uuid: Some("mock-vivo-uuid".into()),
                                        device_name: Some("vivo X200 Pro mini".into()),
                                        derived_key: None,
                                        result: Some("failed".into()),
                                    };
                                    write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                                }
                                MockTrustPolicy::WaitingThenAlways | MockTrustPolicy::AlwaysImmediateIfKnown => {
                                    if matches!(policy, MockTrustPolicy::WaitingThenAlways) && !dk_known {
                                        let wait_resp = SspHandShakeResponse02 {
                                            r#type: Some(SspRequestType::HandshakeResponse02 as i32),
                                            trust_type: Some(SspHandShakeTrustType::TrustWaiting as i32),
                                            device_uuid: Some("mock-vivo-uuid".into()),
                                            device_name: Some("vivo X200 Pro mini".into()),
                                            derived_key: None,
                                            result: Some("needauth".into()),
                                        };
                                        write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &wait_resp.encode_to_vec())).await;
                                        tokio::time::sleep(Duration::from_millis(25)).await;
                                    }

                                    let new_dk = vec![0xAB; 256];
                                    state.lock().await.trusted_derived_keys.insert(new_dk.clone());
                                    let enc_ok = encrypt_handshake_result(pubk, "ok")?;
                                    let ok_resp = SspHandShakeResponse02 {
                                        r#type: Some(SspRequestType::HandshakeResponse02 as i32),
                                        trust_type: Some(SspHandShakeTrustType::TrustAlways as i32),
                                        device_uuid: Some("mock-vivo-uuid".into()),
                                        device_name: Some("vivo X200 Pro mini".into()),
                                        derived_key: Some(new_dk),
                                        result: Some(enc_ok),
                                    };
                                    write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &ok_resp.encode_to_vec())).await;
                                }
                            }
                        }
                        continue;
                    }

                    if frame.flag == FrameFlag::SignedRpc as u8 {
                        let pubk = client_pub_key.as_ref().expect("Must handshake first");
                        assert!(frame.payload.len() >= RSA_SIGNATURE_LEN);
                        let (sig, proto_bytes) = frame.payload.split_at(RSA_SIGNATURE_LEN);
                        if verify_rpc_signature(pubk, sig, proto_bytes).is_err() {
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, b"rsa verify failed")).await;
                            continue;
                        }

                        let req_hdr = SspRequest::decode(proto_bytes)?;
                        let rtype = req_hdr.r#type.unwrap_or(0);

                        if rtype == SspRequestType::HeartBeatRequest as i32 {
                            let resp = SspHeartBeatResponse {
                                r#type: Some(SspRequestType::HeartBeatRequest as i32),
                                host_timestamp: Some(1),
                                client_timestamp: Some(2),
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetDeviceInfoRequest as i32 {
                            let resp = SspGetDeviceInfoResponse {
                                phone_name: Some("vivo X200 Pro mini".into()),
                                phone_model: Some("V2419A".into()),
                                product_brand: Some("vivo".into()),
                                product_manufacturer: Some("vivo".into()),
                                apk_version_name: Some("2.5.0".into()),
                                disk_size: Some(256_000_000_000),
                                used_disk_size: Some(64_000_000_000),
                                root_path: Some("/storage/emulated/0".into()),
                                battery_percentage: Some(92),
                                pic_video_size: Some(12_000_000_000),
                                audio_size: Some(2_000_000_000),
                                download_size: Some(4_000_000_000),
                                app_size: Some(18_000_000_000),
                                cache_size: Some(1_000_000_000),
                                other_size: Some(27_000_000_000),
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetDirFilesRequest as i32 {
                            let req = SspGetDirFilesRequest::decode(proto_bytes)?;
                            let base = req.dir.and_then(|d| d.path).unwrap_or_default();
                            let prefix = format!("{}/", base.trim_end_matches('/'));
                            let guard = state.lock().await;
                            let mut entries = Vec::new();
                            for d in &guard.dirs {
                                if let Some(rest) = d.strip_prefix(&prefix) {
                                    if !rest.is_empty() && !rest.contains('/') {
                                        entries.push(SspFile {
                                            path: Some(d.clone()),
                                            is_directory: Some(true),
                                            file_size: Some(0),
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                            for (p, data) in &guard.files {
                                if let Some(rest) = p.strip_prefix(&prefix) {
                                    if !rest.is_empty() && !rest.contains('/') {
                                        entries.push(SspFile {
                                            path: Some(p.clone()),
                                            is_directory: Some(false),
                                            file_size: Some(data.len() as u64),
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                            let resp = SspGetDirFilesResponse {
                                file: entries,
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetFileExistRequest as i32 {
                            let req = SspFileExistRequest::decode(proto_bytes)?;
                            let path = req.file.and_then(|f| f.path).unwrap_or_default();
                            let guard = state.lock().await;
                            let exist = guard.files.contains_key(&path) || guard.dirs.contains(&path);
                            let resp = SspFileExistResponse {
                                exist: Some(exist),
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetCreateFolderRequest as i32 {
                            let req = handshaker_proto::proto::SspCreateFolderRequest::decode(proto_bytes)?;
                            let path = req.file.and_then(|f| f.path).unwrap_or_default();
                            state.lock().await.dirs.insert(path);
                            let resp = SspCreateFolderResponse {
                                succeed: Some(true),
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetRenameFileRequest as i32 {
                            let req = SspRenameFileRequest::decode(proto_bytes)?;
                            let src = req.source_file.and_then(|f| f.path).unwrap_or_default();
                            let dst = req.target_file.and_then(|f| f.path).unwrap_or_default();
                            let mut guard = state.lock().await;
                            let ok = if let Some(data) = guard.files.remove(&src) {
                                guard.files.insert(dst, data);
                                true
                            } else {
                                false
                            };
                            let resp = SspRenameFileResponse {
                                succeed: Some(ok),
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetDeleteFileRequest as i32 {
                            let req = SspDeleteFileRequest::decode(proto_bytes)?;
                            let mut guard = state.lock().await;
                            for f in &req.file {
                                if let Some(p) = &f.path {
                                    guard.files.remove(p);
                                    guard.dirs.remove(p);
                                }
                            }
                            let resp = SspDeleteFileResponse {
                                succeed: Some(true),
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetUploadFileRequestHeader as i32 {
                            let req = SspUploadFileRequest::decode(proto_bytes)?;
                            let f = req.file.unwrap_or_default();
                            let path = f.path.unwrap_or_default();
                            let size = f.file_size.unwrap_or(0);
                            uploads.insert(frame.sid, ActiveUpload { path: path.clone(), expected_size: size, data: Vec::new() });
                            let ack = SspUploadFileResponseHeader {
                                r#type: Some(SspRequestType::GetUploadFileResponse as i32),
                                file: Some(SspFile { path: Some(path), ..Default::default() }),
                                ready: Some(true),
                                error_code: None,
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &ack.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetDownloadFileRequest as i32 {
                            let req = SspDownloadFileRequest::decode(proto_bytes)?;
                            let path = req.file.and_then(|f| f.path).unwrap_or_default();
                            let offset = req.range.as_ref().and_then(|r| r.offset).unwrap_or(0) as usize;
                            let req_len = req.range.as_ref().and_then(|r| r.length).unwrap_or(0) as usize;
                            let data_opt = state.lock().await.files.get(&path).cloned();
                            match data_opt {
                                None => {
                                    let hdr = SspDownloadFileResponseHeader {
                                        ready: Some(false),
                                        ..Default::default()
                                    };
                                    write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &hdr.encode_to_vec())).await;
                                }
                                Some(full) => {
                                    let end = if req_len == 0 {
                                        full.len()
                                    } else {
                                        (offset + req_len).min(full.len())
                                    };
                                    let slice = &full[offset.min(full.len())..end];
                                    let hdr = SspDownloadFileResponseHeader {
                                        r#type: Some(SspRequestType::GetDownloadFileResponseHeader as i32),
                                        file: Some(SspFile { path: Some(path), file_size: Some(full.len() as u64), ..Default::default() }),
                                        range: Some(SspDataRange {
                                            offset: Some(offset as u64),
                                            length: Some(slice.len() as u64),
                                        }),
                                        ready: Some(true),
                                        ..Default::default()
                                    };
                                    let hdr_bytes = hdr.encode_to_vec();
                                    // Coalesce 8B header length + header + raw body into one raw stream (matches Android `g/a.java`).
                                    let mut combined = BytesMut::with_capacity(8 + hdr_bytes.len() + slice.len());
                                    combined.put_u64(hdr_bytes.len() as u64);
                                    combined.put_slice(&hdr_bytes);
                                    combined.put_slice(slice);
                                    write_chunks(&mut writer, encode_raw_chunks(frame.sid, &combined)).await;
                                }
                            }
                        } else if rtype == SspRequestType::GetPhotoLibRequest as i32 {
                            let resp = SspGetPhotoLibraryResponse {
                                album: vec![SspImageAlbum {
                                    album_id: Some(10),
                                    album_name: Some("Camera".into()),
                                    album_path: Some("/storage/emulated/0/DCIM/Camera".into()),
                                    ..Default::default()
                                }],
                                image: vec![SspImageFile {
                                    path: Some("/storage/emulated/0/DCIM/Camera/IMG_001.jpg".into()),
                                    media_id: Some(1001),
                                    album_id: Some(10),
                                    width: Some(4032),
                                    height: Some(3024),
                                    mime_type: Some("image/jpeg".into()),
                                    ..Default::default()
                                }],
                                camera_album_id: Some(10),
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetVideoLibRequest as i32 {
                            let resp = SspGetVideoLibraryResponse {
                                video: vec![SspVideoFile {
                                    path: Some("/storage/emulated/0/Movies/clip.mp4".into()),
                                    media_id: Some(2002),
                                    duration: Some(12.5),
                                    ..Default::default()
                                }],
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetAudioLibRequest as i32 {
                            let resp = SspGetAudioLibraryResponse {
                                audio: vec![SspAudioFile {
                                    path: Some("/storage/emulated/0/Music/song.flac".into()),
                                    media_id: Some(3003),
                                    title: Some("Test Song".into()),
                                    duration: Some(215.0),
                                    ..Default::default()
                                }],
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetThumbnailRequest as i32 {
                            let mut req = SspGetThumbnailRequest::decode(proto_bytes)?;
                            for img in &mut req.image {
                                img.thumbnail = Some(b"JPEG_THUMB_BYTES".to_vec());
                            }
                            let resp = SspGetThumbnailResponse {
                                r#type: Some(SspRequestType::GetThumbnailRequest as i32),
                                image: req.image,
                                video: req.video,
                                audio_album: req.audio_album,
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::PostClipboardRequest as i32 {
                            let req = SspPostClipboardRequest::decode(proto_bytes)?;
                            state.lock().await.clipboard.push(req.clipboard);
                            let resp = SspPostClipboardResponse {
                                succeed: Some(true),
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::GetClipboardRequest as i32 {
                            let items = state.lock().await.clipboard.clone();
                            let resp = SspGetClipboardResponse {
                                clipboard: items,
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        } else if rtype == SspRequestType::ClearClipboardRequest as i32 {
                            state.lock().await.clipboard.clear();
                            let resp = SspClearClipboardResponse {
                                succeed: Some(true),
                                ..Default::default()
                            };
                            write_chunks(&mut writer, encode_inbound_proto_chunks(frame.sid, &resp.encode_to_vec())).await;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn handshake_waiting_then_always_and_trusted_reconnect() {
    let tmp = TempDir::new().unwrap();
    let store_path = tmp.path().join("trust_store.json");
    let state = Arc::new(Mutex::new(MockState::default()));

    // First connection: should receive TRUST_WAITING then TRUST_ALWAYS and persist derived_key.
    let mut store = IdentityStore::load_or_create(&store_path, false).unwrap();
    let (link1, _push1) = spawn_mock_phone(Arc::clone(&state), MockTrustPolicy::WaitingThenAlways);
    let cfg = ConnectConfig {
        enable_heartbeat: false,
        ..ConnectConfig::default()
    };

    let session1 = Session::connect_with_link(link1, &mut store, cfg.clone())
        .await
        .unwrap();
    assert_eq!(session1.handshake_info().device_uuid, "mock-vivo-uuid");
    assert_eq!(store.trusted_devices().len(), 1);
    session1.quit().await.unwrap();

    // Reconnect using persisted IdentityStore -> mock skips TRUST_WAITING immediately.
    let mut store2 = IdentityStore::load_or_create(&store_path, false).unwrap();
    let (link2, _push2) = spawn_mock_phone(Arc::clone(&state), MockTrustPolicy::AlwaysImmediateIfKnown);
    let session2 = Session::connect_with_link(link2, &mut store2, cfg)
        .await
        .unwrap();
    assert_eq!(
        session2.handshake_info().trust_type,
        SspHandShakeTrustType::TrustAlways as i32
    );
    session2.quit().await.unwrap();
}

#[tokio::test]
async fn handshake_rejected_returns_error() {
    let tmp = TempDir::new().unwrap();
    let mut store = IdentityStore::load_or_create(tmp.path().join("trust.json"), true).unwrap();
    let state = Arc::new(Mutex::new(MockState::default()));
    let (link, _push) = spawn_mock_phone(state, MockTrustPolicy::RejectNo);
    let cfg = ConnectConfig {
        enable_heartbeat: false,
        ..ConnectConfig::default()
    };

    let err = match Session::connect_with_link(link, &mut store, cfg).await {
        Ok(_) => panic!("expected rejected handshake"),
        Err(e) => e,
    };
    assert!(matches!(err, ProtoError::HandshakeRejected { .. }));
}

#[tokio::test]
async fn full_file_transfer_media_and_clipboard_rpc_suite() {
    let tmp = TempDir::new().unwrap();
    let mut store = IdentityStore::load_or_create(tmp.path().join("trust.json"), false).unwrap();
    let state = Arc::new(Mutex::new(MockState::default()));
    let (link, push_tx) = spawn_mock_phone(Arc::clone(&state), MockTrustPolicy::AlwaysImmediateIfKnown);
    let cfg = ConnectConfig {
        enable_heartbeat: false,
        ..ConnectConfig::default()
    };

    let session = Session::connect_with_link(link, &mut store, cfg).await.unwrap();
    let mut ev_rx = session.subscribe();

    // 1. Device info & directory operations
    let dev = session.get_device_info().await.unwrap();
    assert_eq!(dev.phone_name.as_deref(), Some("vivo X200 Pro mini"));
    session.heartbeat().await.unwrap();
    session.create_folder("/storage/emulated/0/Download/NewDir").await.unwrap();
    assert!(session.file_exists("/storage/emulated/0/Download/NewDir").await.unwrap());

    // 2. Upload 0-byte file + multi-chunk 120 KiB binary file
    let mut empty_cur = Cursor::new(Vec::<u8>::new());
    session
        .upload_from_reader(
            &mut empty_cur,
            0,
            "/storage/emulated/0/Download/empty.bin",
            1700000000,
            |_, _| {},
        )
        .await
        .unwrap();
    assert!(state.lock().await.files.get("/storage/emulated/0/Download/empty.bin").unwrap().is_empty());

    let large_blob: Vec<u8> = (0..120_000).map(|i| ((i * 31) % 256) as u8).collect();
    let mut up_cur = Cursor::new(large_blob.clone());
    let mut up_progress = Vec::new();
    session
        .upload_from_reader(
            &mut up_cur,
            large_blob.len() as u64,
            "/storage/emulated/0/Download/big.bin",
            1700000001,
            |done, total| up_progress.push((done, total)),
        )
        .await
        .unwrap();
    assert_eq!(up_progress.last().copied(), Some((120_000, 120_000)));
    assert_eq!(state.lock().await.files.get("/storage/emulated/0/Download/big.bin"), Some(&large_blob));

    // 3. Rename, list, download full + range slice, delete
    session
        .rename_file(
            "/storage/emulated/0/Download/big.bin",
            "/storage/emulated/0/Download/big_renamed.bin",
        )
        .await
        .unwrap();
    let ls = session.list_dir("/storage/emulated/0/Download", 1).await.unwrap();
    assert!(ls.file.iter().any(|f| f.path.as_deref() == Some("/storage/emulated/0/Download/big_renamed.bin")));

    let mut downloaded = Vec::new();
    session
        .download_to_writer(
            "/storage/emulated/0/Download/big_renamed.bin",
            0,
            0,
            &mut downloaded,
            |_, _| {},
        )
        .await
        .unwrap();
    assert_eq!(downloaded, large_blob);

    let mut partial = Vec::new();
    session
        .download_to_writer(
            "/storage/emulated/0/Download/big_renamed.bin",
            1000,
            2500,
            &mut partial,
            |_, _| {},
        )
        .await
        .unwrap();
    assert_eq!(partial, &large_blob[1000..3500]);

    session
        .delete_files(&["/storage/emulated/0/Download/empty.bin".to_owned()], true)
        .await
        .unwrap();
    assert!(!session.file_exists("/storage/emulated/0/Download/empty.bin").await.unwrap());

    // 4. Media libraries + thumbnails
    let photos = session.get_photo_library().await.unwrap();
    assert_eq!(photos.image.len(), 1);
    let thumbs = session
        .get_thumbnails(vec![photos.image[0].clone()], vec![], vec![])
        .await
        .unwrap();
    assert_eq!(thumbs.image[0].thumbnail.as_deref(), Some(b"JPEG_THUMB_BYTES".as_slice()));
    assert_eq!(session.get_video_library().await.unwrap().video.len(), 1);
    assert_eq!(session.get_audio_library().await.unwrap().audio.len(), 1);

    // 5. Clipboard two-way RPC + unsolicited server push
    assert!(session.post_clipboard("Linux clipboard note 📝").await.unwrap());
    let clip = session.get_clipboard().await.unwrap();
    assert_eq!(clip.len(), 1);
    assert_eq!(clip[0].text, "Linux clipboard note 📝");

    let push_msg = SspClipboardChange {
        r#type: Some(SspRequestType::ClipboardChange as i32),
        clipboard: vec![SspClipboard {
            content: Some(gzip_compress_text("From Android Push").unwrap()),
            mstimestamp: Some(1700001234567),
        }],
    };
    push_tx.send((999_001, push_msg.encode_to_vec())).await.unwrap();

    let ev = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match ev_rx.recv().await {
                Ok(SessionEvent::ClipboardChanged(entries)) => return entries,
                Ok(_) => continue,
                Err(e) => panic!("event channel closed: {e}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(ev.len(), 1);
    assert_eq!(ev[0].text, "From Android Push");

    assert!(session.clear_clipboard().await.unwrap());
    assert!(session.get_clipboard().await.unwrap().is_empty());
    session.quit().await.unwrap();
}
