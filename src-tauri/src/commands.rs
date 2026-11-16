use std::collections::HashSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use handshaker_proto::device_profile::{format_brand_and_model, DeviceProfileStore};
use handshaker_proto::discovery::discover_all;
use handshaker_proto::transport::aoa::{open_aoa_transport, reset_and_reopen_aoa_transport};
use handshaker_proto::transport::tcp::connect_tcp;
use handshaker_proto::{ConnectConfig, ProtoError, Session, TransportLink};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::dto::{
    AppSettings, AppSettingsPatch, AudioAlbumDto, AudioItemDto, AudioLibraryDto, ClipboardItemDto,
    ConnectionStateDto, DeviceInfoDto, DiscoverySnapshotDto, FileEntryDto, ImageItemDto,
    MediaBridgeInfoDto, MediaPlaybackSupportDto, PhotoAlbumDto, PhotoLibraryDto, TransferItemDto,
    TrashItemDto, TrustedDeviceDto, UsbCandidateDto, VideoAlbumDto, VideoItemDto, VideoLibraryDto,
    WifiCandidateDto,
};
use crate::settings::{apply_settings_patch, normalize_trash_dir, save_settings};
use crate::state::{map_clipboard_dtos, AppState};
use crate::transfers::DownloadTarget;

#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<AppSettings, String> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
pub async fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: AppSettingsPatch,
) -> Result<AppSettings, String> {
    if let Some(raw_trash) = patch.trash_dir.as_deref() {
        if normalize_trash_dir(raw_trash).is_none() {
            return Err("Invalid trash directory path".to_owned());
        }
    }
    let updated = {
        let mut g = state.settings.write().await;
        apply_settings_patch(&mut g, patch);
        save_settings(&g)?;
        g.clone()
    };
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_decorations(updated.use_system_decorations);
    }
    let connected = state.session_slot.read().await.is_some();
    state
        .clipboard_bridge
        .set_enabled(connected && updated.clipboard_auto_sync);
    Ok(updated)
}

#[tauri::command]
pub async fn window_minimize(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.minimize().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn window_toggle_maximize(app: AppHandle) -> Result<bool, String> {
    if let Some(win) = app.get_webview_window("main") {
        if win.is_maximized().map_err(|e| e.to_string())? {
            win.unmaximize().map_err(|e| e.to_string())?;
            return Ok(false);
        } else {
            win.maximize().map_err(|e| e.to_string())?;
            return Ok(true);
        }
    }
    Ok(false)
}

#[tauri::command]
pub async fn window_close(app: AppHandle) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("main") {
        win.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn scan_devices(wifi_timeout_ms: Option<u64>) -> Result<DiscoverySnapshotDto, String> {
    let dur = Duration::from_millis(wifi_timeout_ms.unwrap_or(2800).clamp(500, 10_000));
    let snap = tokio::task::spawn_blocking(move || discover_all(dur))
        .await
        .map_err(|e| format!("Discovery task failed: {e}"))?;

    Ok(DiscoverySnapshotDto {
        wifi: snap
            .wifi
            .into_iter()
            .map(|w| WifiCandidateDto {
                service_name: w.service_name,
                display_name: w.display_name,
                host: w.host,
                port: w.port,
                addresses: w.addresses,
                hostname: w.hostname,
                device_uuid: w.device_uuid,
                device_name: w.device_name,
                phone_model: w.phone_model,
                product_brand: w.product_brand,
                product_manufacturer: w.product_manufacturer,
                apk_version_name: w.apk_version_name,
                is_trusted: w.is_trusted,
                is_locked: w.is_locked,
            })
            .collect(),
        usb: snap
            .usb
            .into_iter()
            .map(|u| UsbCandidateDto {
                bus: u.bus,
                address: u.address,
                vendor_id: u.vendor_id,
                product_id: u.product_id,
                manufacturer: u.manufacturer,
                product: u.product,
                serial: u.serial,
                already_aoa: u.already_aoa,
                interface_count: u.interface_count,
            })
            .collect(),
    })
}

#[tauri::command]
pub async fn list_trusted_devices(state: State<'_, AppState>) -> Result<Vec<TrustedDeviceDto>, String> {
    let _guard = state.identity_mutex.lock().await;
    let store = state.load_identity_store()?;
    let profiles = DeviceProfileStore::load_default();
    Ok(store
        .trusted_devices()
        .into_iter()
        .map(|d| {
            let p = profiles.get(&d.device_uuid);
            TrustedDeviceDto {
                device_uuid: d.device_uuid,
                device_name: d.device_name,
                paired_at_unix: d.paired_at_unix,
                phone_model: p.and_then(|x| x.phone_model.clone()),
                product_brand: p.and_then(|x| x.product_brand.clone()),
                product_manufacturer: p.and_then(|x| x.product_manufacturer.clone()),
            }
        })
        .collect())
}

#[tauri::command]
pub async fn remove_trusted_device(
    state: State<'_, AppState>,
    device_uuid: String,
) -> Result<bool, String> {
    let _guard = state.identity_mutex.lock().await;
    let mut store = state.load_identity_store()?;
    store.remove_trusted_device(&device_uuid).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_connection_state(state: State<'_, AppState>) -> Result<ConnectionStateDto, String> {
    Ok(state.connection_state.read().await.clone())
}

enum ConnectStepError {
    Proto(ProtoError),
    Other(String),
}

impl std::fmt::Display for ConnectStepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectStepError::Proto(e) => write!(f, "{e}"),
            ConnectStepError::Other(msg) => f.write_str(msg),
        }
    }
}

async fn emit_connection_status(
    app: &AppHandle,
    state: &AppState,
    status: &str,
    error: Option<String>,
) {
    let mut cs = state.connection_state.write().await;
    cs.status = status.to_owned();
    if status == "disconnected" {
        cs.handshake = None;
    }
    cs.error = error;
    let _ = app.emit("connection://state", cs.clone());
}

async fn attempt_connect(
    app: &AppHandle,
    state: &AppState,
    link: TransportLink,
) -> std::result::Result<ConnectionStateDto, ConnectStepError> {
    let _guard = state.identity_mutex.lock().await;
    let mut store = state
        .load_identity_store()
        .map_err(ConnectStepError::Other)?;

    emit_connection_status(app, state, "trust_waiting", None).await;

    match Session::connect_with_link(link, &mut store, ConnectConfig::default()).await {
        Ok(session) => {
            let _ = DeviceProfileStore::load_default().save_handshake(session.handshake_info());
            state.attach_session(app.clone(), session).await;
            Ok(state.connection_state.read().await.clone())
        }
        Err(e) => Err(ConnectStepError::Proto(e)),
    }
}

async fn finish_connect(
    app: AppHandle,
    state: &AppState,
    link: TransportLink,
) -> Result<ConnectionStateDto, String> {
    match attempt_connect(&app, state, link).await {
        Ok(cs) => Ok(cs),
        Err(e) => {
            let msg = e.to_string();
            emit_connection_status(&app, state, "disconnected", Some(msg.clone())).await;
            Err(msg)
        }
    }
}

async fn open_usb_link(
    filter: Option<(u8, u8)>,
    allow_switch: bool,
    reset_stale: bool,
) -> Result<TransportLink, String> {
    tokio::task::spawn_blocking(move || {
        let timeout = Duration::from_secs(12);
        if reset_stale {
            reset_and_reopen_aoa_transport(filter, timeout)
        } else {
            open_aoa_transport(filter, allow_switch, timeout)
        }
    })
    .await
    .map_err(|e| format!("USB task panicked: {e}"))?
    .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn connect_wifi(
    app: AppHandle,
    state: State<'_, AppState>,
    host: String,
    port: u16,
) -> Result<ConnectionStateDto, String> {
    disconnect_existing(&state).await;
    emit_connection_status(&app, &state, "connecting", None).await;

    let link = connect_tcp(&host, port, Duration::from_secs(8))
        .await
        .map_err(|e| e.to_string())?;
    finish_connect(app, &state, link).await
}

#[tauri::command]
pub async fn connect_usb(
    app: AppHandle,
    state: State<'_, AppState>,
    bus: Option<u8>,
    address: Option<u8>,
    allow_switch: bool,
) -> Result<ConnectionStateDto, String> {
    disconnect_existing(&state).await;
    emit_connection_status(&app, &state, "connecting", None).await;

    let filter = bus.zip(address);
    let link = match open_usb_link(filter, allow_switch, false).await {
        Ok(link) => link,
        Err(msg) => {
            emit_connection_status(&app, &state, "disconnected", Some(msg.clone())).await;
            return Err(msg);
        }
    };

    match attempt_connect(&app, &state, link).await {
        Ok(cs) => Ok(cs),
        Err(ConnectStepError::Proto(
            ProtoError::Timeout("USB AOA handshake response") | ProtoError::Disconnected,
        )) if allow_switch =>
        {
            emit_connection_status(&app, &state, "connecting", None).await;
            let retry_link = match open_usb_link(filter, allow_switch, true).await {
                Ok(link) => link,
                Err(msg) => {
                    emit_connection_status(&app, &state, "disconnected", Some(msg.clone())).await;
                    return Err(msg);
                }
            };
            finish_connect(app, &state, retry_link).await
        }
        Err(e) => {
            let msg = e.to_string();
            emit_connection_status(&app, &state, "disconnected", Some(msg.clone())).await;
            Err(msg)
        }
    }
}

#[tauri::command]
pub async fn disconnect_session(app: AppHandle, state: State<'_, AppState>) -> Result<(), String> {
    disconnect_existing(&state).await;
    let mut cs = state.connection_state.write().await;
    cs.status = "disconnected".to_owned();
    cs.handshake = None;
    cs.error = None;
    let _ = app.emit("connection://state", cs.clone());
    Ok(())
}

async fn disconnect_existing(state: &AppState) {
    state.clipboard_bridge.set_enabled(false);
    let existing = state.session_slot.write().await.take();
    if let Some(s) = existing {
        let _ = s.quit().await;
    }
}

async fn active_session(state: &AppState) -> Result<Session, String> {
    state
        .session_slot
        .read()
        .await
        .clone()
        .ok_or_else(|| "No active phone connection".to_owned())
}

#[tauri::command]
pub async fn get_device_info(state: State<'_, AppState>) -> Result<DeviceInfoDto, String> {
    let session = active_session(&state).await?;
    let info = session.get_device_info().await.map_err(|e| e.to_string())?;
    {
        let _guard = state.identity_mutex.lock().await;
        let _ = DeviceProfileStore::load_default()
            .save_device_info(&session.handshake_info().device_uuid, &info);
    }
    let disk_size = info.disk_size.unwrap_or(0);
    let used_disk_size = info.used_disk_size.unwrap_or(0);
    Ok(DeviceInfoDto {
        phone_name: info.phone_name.unwrap_or_else(|| "Android Phone".to_owned()),
        phone_model: info.phone_model.unwrap_or_default(),
        product_brand: info.product_brand.unwrap_or_default(),
        product_manufacturer: info.product_manufacturer.unwrap_or_default(),
        apk_version_name: info.apk_version_name.unwrap_or_default(),
        root_path: info.root_path.unwrap_or_else(|| "/storage/emulated/0".to_owned()),
        external_storage_path: info.external_storage_path.filter(|s| !s.is_empty()),
        battery_percentage: info.battery_percentage.unwrap_or(0),
        disk_size,
        used_disk_size,
        free_disk_size: disk_size.saturating_sub(used_disk_size),
        pic_video_size: info.pic_video_size.unwrap_or(0).max(0) as u64,
        audio_size: info.audio_size.unwrap_or(0).max(0) as u64,
        download_size: info.download_size.unwrap_or(0).max(0) as u64,
        app_size: info.app_size.unwrap_or(0).max(0) as u64,
        cache_size: info.cache_size.unwrap_or(0).max(0) as u64,
        other_size: info.other_size.unwrap_or(0).max(0) as u64,
    })
}

#[tauri::command]
pub async fn list_dir(
    state: State<'_, AppState>,
    path: String,
    maxdepth: Option<u32>,
) -> Result<Vec<FileEntryDto>, String> {
    let session = active_session(&state).await?;
    let resp = session
        .list_dir(&path, maxdepth.unwrap_or(1))
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp
        .file
        .into_iter()
        .filter_map(|f| {
            let full_path = f.path?;
            let name = full_path
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_owned();
            if name.is_empty() {
                return None;
            }
            Some(FileEntryDto {
                name,
                path: full_path,
                is_directory: f.is_directory.unwrap_or(false),
                file_size: f.file_size.unwrap_or(0),
                created_timestamp: f.created_timestamp.unwrap_or(0),
                modified_timestamp: f.modified_timestamp.unwrap_or(0),
                is_trash: f.is_trash.unwrap_or(false),
            })
        })
        .collect())
}

#[tauri::command]
pub async fn create_folder(state: State<'_, AppState>, path: String) -> Result<(), String> {
    let session = active_session(&state).await?;
    session.create_folder(&path).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn rename_file(
    state: State<'_, AppState>,
    src_path: String,
    dst_path: String,
) -> Result<(), String> {
    let session = active_session(&state).await?;
    session
        .rename_file(&src_path, &dst_path)
        .await
        .map_err(|e| e.to_string())
}

fn non_empty_trimmed_str(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

async fn resolve_trash_device_origin(
    state: &AppState,
    session: &Session,
    fetch_model_if_missing: bool,
) -> crate::trash::TrashDeviceOrigin {
    let hs = session.handshake_info();
    let mut origin = crate::trash::origin_from_handshake(hs);

    if fetch_model_if_missing {
        if let Some(ref uuid) = origin.device_uuid {
            let _guard = state.identity_mutex.lock().await;
            if let Some(profile) = DeviceProfileStore::load_default().get(uuid) {
                if origin.device_name.is_none() {
                    origin.device_name = non_empty_trimmed_str(&profile.device_name);
                }
                origin.device_model = format_brand_and_model(
                    profile.product_brand.as_deref(),
                    profile.product_manufacturer.as_deref(),
                    profile.phone_model.as_deref(),
                );
            }
        }
    }

    if origin.device_uuid.is_none() || (fetch_model_if_missing && origin.device_model.is_none()) {
        if let Ok(info) = session.get_device_info().await {
            if origin.device_uuid.is_none() {
                origin.device_uuid = info.phone_id.as_deref().and_then(non_empty_trimmed_str);
            }
            if origin.device_name.is_none() {
                origin.device_name = info.phone_name.as_deref().and_then(non_empty_trimmed_str);
            }
            if origin.device_model.is_none() {
                origin.device_model = format_brand_and_model(
                    info.product_brand.as_deref(),
                    info.product_manufacturer.as_deref(),
                    info.phone_model.as_deref(),
                );
            }
            if let Some(ref uuid) = origin.device_uuid {
                let _guard = state.identity_mutex.lock().await;
                let _ = DeviceProfileStore::load_default().save_device_info(uuid, &info);
            }
        }
    }

    origin
}

#[tauri::command]
pub async fn delete_files(
    state: State<'_, AppState>,
    paths: Vec<String>,
    is_trash: Option<bool>,
) -> Result<(), String> {
    let use_trash = is_trash.unwrap_or(true);
    let session = active_session(&state).await?;
    if use_trash {
        let trash_dir = state.settings.read().await.trash_dir.clone();
        let origin = resolve_trash_device_origin(&state, &session, true).await;
        crate::trash::move_remote_paths_to_linux_trash(
            &session,
            &paths,
            &trash_dir,
            Some(&origin),
        )
        .await
    } else {
        session
            .delete_files(&paths, false)
            .await
            .map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn list_trash_items(state: State<'_, AppState>) -> Result<Vec<TrashItemDto>, String> {
    let trash_dir = state.settings.read().await.trash_dir.clone();
    crate::trash::list_trash_items(&trash_dir).await
}

#[tauri::command]
pub async fn restore_trash_items(
    state: State<'_, AppState>,
    entry_names: Vec<String>,
) -> Result<Vec<String>, String> {
    let session = active_session(&state).await?;
    let trash_dir = state.settings.read().await.trash_dir.clone();
    let current_origin = resolve_trash_device_origin(&state, &session, false).await;
    crate::trash::restore_trash_items(
        &session,
        &trash_dir,
        &entry_names,
        Some(&current_origin),
        None,
    )
    .await
}

#[tauri::command]
pub async fn delete_trash_items(
    state: State<'_, AppState>,
    entry_names: Vec<String>,
) -> Result<usize, String> {
    let trash_dir = state.settings.read().await.trash_dir.clone();
    crate::trash::delete_trash_items(&trash_dir, &entry_names).await
}

#[tauri::command]
pub async fn empty_trash(state: State<'_, AppState>) -> Result<usize, String> {
    let trash_dir = state.settings.read().await.trash_dir.clone();
    crate::trash::empty_trash(&trash_dir).await
}

#[tauri::command]
pub async fn list_transfers(state: State<'_, AppState>) -> Result<Vec<TransferItemDto>, String> {
    Ok(state.transfers.list())
}

#[tauri::command]
pub async fn enqueue_download(
    state: State<'_, AppState>,
    targets: Vec<DownloadTarget>,
    local_dir: Option<String>,
) -> Result<usize, String> {
    let session = active_session(&state).await?;
    let dir = match local_dir.filter(|s| !s.trim().is_empty()) {
        Some(d) => PathBuf::from(d),
        None => PathBuf::from(state.settings.read().await.download_dir.clone()),
    };
    state
        .transfers
        .enqueue_downloads(&session, targets, &dir)
        .await
}

#[tauri::command]
pub async fn enqueue_upload(
    state: State<'_, AppState>,
    local_paths: Vec<String>,
    remote_dir: String,
) -> Result<usize, String> {
    let session = active_session(&state).await?;
    let paths = local_paths.into_iter().map(PathBuf::from).collect();
    state
        .transfers
        .enqueue_uploads(&session, paths, &remote_dir)
        .await
}

#[tauri::command]
pub async fn cancel_transfer(state: State<'_, AppState>, id: String) -> Result<bool, String> {
    Ok(state.transfers.cancel(&id))
}

#[tauri::command]
pub async fn clear_finished_transfers(state: State<'_, AppState>) -> Result<(), String> {
    state.transfers.clear_finished();
    Ok(())
}

#[tauri::command]
pub async fn get_media_base_url(state: State<'_, AppState>) -> Result<MediaBridgeInfoDto, String> {
    Ok(MediaBridgeInfoDto {
        base_url: state.media_bridge.base_url(),
        token: state.media_bridge.token().to_owned(),
    })
}

#[tauri::command]
pub async fn get_photo_library(state: State<'_, AppState>) -> Result<PhotoLibraryDto, String> {
    let session = active_session(&state).await?;
    let resp = session.get_photo_library().await.map_err(|e| e.to_string())?;
    let bridge = &state.media_bridge;

    let albums = resp
        .album
        .into_iter()
        .map(|a| {
            let cover_thumb_url = a.cover_image.as_ref().and_then(|img| {
                let id = img.media_id?;
                let p = img.path.as_deref().unwrap_or("");
                Some(bridge.thumb_url("image", id, p))
            });
            PhotoAlbumDto {
                album_id: a.album_id.unwrap_or(0),
                album_name: a.album_name.unwrap_or_else(|| "Album".to_owned()),
                album_path: a.album_path.unwrap_or_default(),
                cover_thumb_url,
            }
        })
        .collect();

    let images = resp
        .image
        .into_iter()
        .filter_map(|img| {
            let path = img.path?;
            let id = img.media_id.unwrap_or(0);
            let file_size = img.file_size.unwrap_or(0);
            let mime = img.mime_type.unwrap_or_else(|| "image/jpeg".to_owned());
            let fallback_title = path.rsplit('/').next().unwrap_or("photo").to_owned();
            Some(ImageItemDto {
                media_id: id,
                album_id: img.album_id.unwrap_or(0),
                album_name: img.album_name.unwrap_or_default(),
                thumb_url: bridge.thumb_url("image", id, &path),
                stream_url: bridge.stream_url(&path, file_size, &mime),
                path,
                title: img.title.filter(|t| !t.is_empty()).unwrap_or(fallback_title),
                mime_type: mime,
                file_size,
                width: img.width.unwrap_or(0),
                height: img.height.unwrap_or(0),
                orientation: img.orientation.unwrap_or(0),
                date_taken: img.date_taken.unwrap_or(0),
                modified_timestamp: img.modified_timestamp.unwrap_or(0),
                starred: img.starred.unwrap_or(false),
            })
        })
        .collect();

    Ok(PhotoLibraryDto {
        camera_album_id: resp.camera_album_id,
        albums,
        images,
    })
}

#[tauri::command]
pub async fn get_video_library(state: State<'_, AppState>) -> Result<VideoLibraryDto, String> {
    let session = active_session(&state).await?;
    let resp = session.get_video_library().await.map_err(|e| e.to_string())?;
    let bridge = &state.media_bridge;

    let albums = resp
        .album
        .into_iter()
        .map(|a| VideoAlbumDto {
            album_id: a.album_id.unwrap_or(0),
            album_name: a.album_name.unwrap_or_else(|| "Videos".to_owned()),
            album_path: a.album_path.unwrap_or_default(),
        })
        .collect();

    let videos = resp
        .video
        .into_iter()
        .filter_map(|v| {
            let path = v.path?;
            let id = v.media_id.unwrap_or(0);
            let file_size = v.file_size.unwrap_or(0);
            let mime = v.mime_type.unwrap_or_else(|| "video/mp4".to_owned());
            let name = path.rsplit('/').next().unwrap_or("video").to_owned();
            Some(VideoItemDto {
                media_id: id,
                album_id: v.album_id.unwrap_or(0),
                thumb_url: bridge.thumb_url("video", id, &path),
                stream_url: bridge.stream_url(&path, file_size, &mime),
                path,
                name,
                mime_type: mime,
                file_size,
                width: v.width.unwrap_or(0),
                height: v.height.unwrap_or(0),
                duration_secs: v.duration.unwrap_or(0.0),
                modified_timestamp: v.modified_timestamp.unwrap_or(0),
            })
        })
        .collect();

    Ok(VideoLibraryDto { albums, videos })
}

#[tauri::command]
pub async fn get_audio_library(state: State<'_, AppState>) -> Result<AudioLibraryDto, String> {
    let session = active_session(&state).await?;
    let resp = session.get_audio_library().await.map_err(|e| e.to_string())?;
    let bridge = &state.media_bridge;

    let albums = resp
        .album
        .into_iter()
        .map(|a| {
            let id = a.album_id.unwrap_or(0);
            let path = a.album_path.unwrap_or_default();
            AudioAlbumDto {
                album_id: id,
                album_name: a.album_name.unwrap_or_else(|| "Unknown Album".to_owned()),
                thumb_url: bridge.thumb_url("audio_album", id, &path),
                album_path: path,
                artist: a.artist.unwrap_or_else(|| "Unknown Artist".to_owned()),
                year: a.year.unwrap_or(0),
            }
        })
        .collect();

    let tracks = resp
        .audio
        .into_iter()
        .filter_map(|t| {
            let path = t.path?;
            let id = t.media_id.unwrap_or(0);
            let file_size = t.file_size.unwrap_or(0);
            let mime = t.mime_type.unwrap_or_else(|| "audio/mpeg".to_owned());
            let fallback_title = path.rsplit('/').next().unwrap_or("track").to_owned();
            Some(AudioItemDto {
                media_id: id,
                album_id: t.album_id.unwrap_or(0),
                stream_url: bridge.stream_url(&path, file_size, &mime),
                path,
                title: t.title.filter(|s| !s.is_empty()).unwrap_or(fallback_title),
                artist: t.artist.unwrap_or_else(|| "Unknown Artist".to_owned()),
                mime_type: mime,
                file_size,
                duration_secs: t.duration.unwrap_or(0.0),
                bitrate: t.bitrate.unwrap_or(0),
                sample_rate: t.sample_rate.unwrap_or(0.0),
                year: t.year.unwrap_or(0),
                track: t.track.unwrap_or(0),
            })
        })
        .collect();

    Ok(AudioLibraryDto { albums, tracks })
}

#[tauri::command]
pub async fn get_clipboard_history(state: State<'_, AppState>) -> Result<Vec<ClipboardItemDto>, String> {
    let session = active_session(&state).await?;
    let entries = session.get_clipboard().await.map_err(|e| e.to_string())?;
    Ok(map_clipboard_dtos(&entries))
}

#[tauri::command]
pub async fn post_clipboard_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    let session = active_session(&state).await?;
    state.clipboard_bridge.note_sent_to_phone(&text);
    session
        .post_clipboard(&text)
        .await
        .map_err(|e| e.to_string())?;
    if let Ok(entries) = session.get_clipboard().await {
        let _ = app.emit("clipboard://changed", map_clipboard_dtos(&entries));
    }
    Ok(())
}

#[tauri::command]
pub async fn clear_clipboard_history(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let session = active_session(&state).await?;
    session.clear_clipboard().await.map_err(|e| e.to_string())?;
    let _ = app.emit("clipboard://changed", Vec::<ClipboardItemDto>::new());
    Ok(())
}

#[tauri::command]
pub async fn delete_clipboard_item(
    app: AppHandle,
    state: State<'_, AppState>,
    timestamp_ms: i64,
    raw_gzip_b64: String,
) -> Result<(), String> {
    let session = active_session(&state).await?;
    let raw_gzip = BASE64
        .decode(raw_gzip_b64.as_bytes())
        .map_err(|e| format!("Invalid raw_gzip_b64: {e}"))?;
    session
        .delete_clipboard(raw_gzip, timestamp_ms)
        .await
        .map_err(|e| e.to_string())?;
    if let Ok(entries) = session.get_clipboard().await {
        let _ = app.emit("clipboard://changed", map_clipboard_dtos(&entries));
    }
    Ok(())
}

#[tauri::command]
pub async fn copy_to_linux_clipboard(
    state: State<'_, AppState>,
    text: String,
) -> Result<(), String> {
    state.clipboard_bridge.write_to_linux(text);
    Ok(())
}

static MEDIA_PLAYBACK_SUPPORT: OnceLock<MediaPlaybackSupportDto> = OnceLock::new();

#[tauri::command]
pub async fn get_media_playback_support() -> Result<MediaPlaybackSupportDto, String> {
    let support = tokio::task::spawn_blocking(|| {
        MEDIA_PLAYBACK_SUPPORT
            .get_or_init(probe_media_playback_support)
            .clone()
    })
    .await
    .map_err(|e| format!("Failed probing GStreamer media support: {e}"))?;
    Ok(support)
}

#[tauri::command]
pub async fn open_media_externally(
    state: State<'_, AppState>,
    url: String,
) -> Result<String, String> {
    let base = state.media_bridge.base_url();
    let token = state.media_bridge.token();
    if !is_valid_media_bridge_stream_url(&base, token, &url) {
        return Err("Refused to open URL outside local media stream bridge".to_owned());
    }

    tokio::task::spawn_blocking(move || launch_external_media_player(&url))
        .await
        .map_err(|e| format!("Failed spawning external media player: {e}"))?
}

fn is_valid_media_bridge_stream_url(base_url: &str, token: &str, url: &str) -> bool {
    let prefix = format!("{base_url}/stream?");
    let Some(query) = url.strip_prefix(&prefix) else {
        return false;
    };
    query
        .split('&')
        .any(|pair| pair.strip_prefix("token=") == Some(token))
}

/// Extracts a display filename from a local media-bridge stream URL's `path` query parameter.
fn extract_stream_file_name(url: &str) -> Option<String> {
    let (_, query) = url.split_once('?')?;
    let encoded_path = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("path="))?;
    let decoded = crate::media_bridge::url_decode(encoded_path);
    let trimmed = decoded.trim_end_matches(['/', '\\']);
    let base = trimmed.rsplit(['/', '\\']).next()?;
    let sanitized: String = base
        .chars()
        .map(|ch| if matches!(ch, '\r' | '\n' | '\0') { ' ' } else { ch })
        .collect();
    let name = sanitized.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_owned())
    }
}

/// Builds launcher CLI arguments so both audio and video streams open in a graphical window.
fn external_player_args(player: &str, url: &str) -> Vec<String> {
    if player == "mpv" {
        let mut args = vec![
            // Force mpv's GUI/window mode so audio-only streams show a window and OSC controls.
            "--player-operation-mode=pseudo-gui".to_owned(),
            "--force-window=yes".to_owned(),
            "--osc=yes".to_owned(),
        ];
        if let Some(title) = extract_stream_file_name(url) {
            args.push(format!("--force-media-title={title}"));
        }
        args.push("--".to_owned());
        args.push(url.to_owned());
        args
    } else {
        vec![url.to_owned()]
    }
}

fn launch_external_media_player(url: &str) -> Result<String, String> {
    const PLAYERS: &[&str] = &[
        "mpv",
        "vlc",
        "celluloid",
        "haruna",
        "totem",
        "smplayer",
        "audacious",
        "elisa",
        "rhythmbox",
    ];
    for player in PLAYERS {
        match Command::new(player)
            .args(external_player_args(player, url))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok((*player).to_owned()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("Failed launching {player}: {e}")),
        }
    }
    Err(
        "No standalone desktop media player found (tried mpv, vlc, celluloid, haruna, totem, smplayer, audacious, elisa, rhythmbox). Click Download to save locally or install gst-plugins-good for in-app playback."
            .to_owned(),
    )
}

fn evaluate_gstreamer_elements(found: &HashSet<&str>) -> MediaPlaybackSupportDto {
    let required = ["autoaudiosink", "souphttpsrc", "autovideosink"];
    let missing_elements: Vec<String> = required
        .iter()
        .filter(|el| !found.contains(**el))
        .map(|el| (*el).to_owned())
        .collect();

    // WebKitGTK 2.54+ MediaPlayerPrivateGStreamer initializes autoaudiosink for both <audio> and <video>
    // and streams loopback HTTP URLs over souphttpsrc; aborting WebKitWebProcess if missing.
    let has_audio_pipe = found.contains("autoaudiosink") && found.contains("souphttpsrc");
    let embedded_audio = has_audio_pipe;
    let embedded_video = has_audio_pipe && found.contains("autovideosink");

    let reason = (!missing_elements.is_empty()).then(|| {
        format!(
            "In-app playback is disabled to prevent WebKitGTK crash because GStreamer element(s) are missing: {}. Install gst-plugins-good (and gst-libav for extra codecs), or use Open in System Player / Download.",
            missing_elements.join(", ")
        )
    });

    MediaPlaybackSupportDto {
        embedded_audio,
        embedded_video,
        missing_elements,
        reason,
    }
}

fn probe_media_playback_support() -> MediaPlaybackSupportDto {
    if !cfg!(target_os = "linux") {
        return MediaPlaybackSupportDto {
            embedded_audio: true,
            embedded_video: true,
            missing_elements: Vec::new(),
            reason: None,
        };
    }

    let mut found = HashSet::new();
    let mut inspect_available = true;
    for el in ["autoaudiosink", "souphttpsrc", "autovideosink"] {
        match Command::new("gst-inspect-1.0")
            .arg(el)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) if status.success() => {
                found.insert(el);
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                inspect_available = false;
                break;
            }
            Err(_) => {}
        }
    }

    if !inspect_available {
        if has_gst_plugin_so("libgstautodetect.so") {
            found.insert("autoaudiosink");
            found.insert("autovideosink");
        }
        if has_gst_plugin_so("libgstsoup.so") {
            found.insert("souphttpsrc");
        }
    }

    evaluate_gstreamer_elements(&found)
}

fn has_gst_plugin_so(lib_name: &str) -> bool {
    let mut dirs = Vec::<PathBuf>::new();
    for env_key in ["GST_PLUGIN_PATH", "GST_PLUGIN_SYSTEM_PATH_1_0"] {
        if let Some(val) = std::env::var_os(env_key) {
            dirs.extend(std::env::split_paths(&val));
        }
    }
    dirs.extend(
        [
            "/usr/lib/gstreamer-1.0",
            "/usr/lib64/gstreamer-1.0",
            "/usr/lib/x86_64-linux-gnu/gstreamer-1.0",
            "/usr/lib/aarch64-linux-gnu/gstreamer-1.0",
        ]
        .into_iter()
        .map(PathBuf::from),
    );

    dirs.into_iter().any(|dir| dir.join(lib_name).is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gstreamer_capability_evaluation_prevents_missing_sink_crash() {
        let missing_good = HashSet::new();
        let res = evaluate_gstreamer_elements(&missing_good);
        assert!(!res.embedded_audio);
        assert!(!res.embedded_video);
        assert_eq!(
            res.missing_elements,
            vec!["autoaudiosink", "souphttpsrc", "autovideosink"]
        );
        assert!(res.reason.as_deref().unwrap_or("").contains("gst-plugins-good"));

        let audio_only = HashSet::from(["autoaudiosink", "souphttpsrc"]);
        let res_audio = evaluate_gstreamer_elements(&audio_only);
        assert!(res_audio.embedded_audio);
        assert!(!res_audio.embedded_video);
        assert_eq!(res_audio.missing_elements, vec!["autovideosink"]);

        let full = HashSet::from(["autoaudiosink", "souphttpsrc", "autovideosink"]);
        let res_full = evaluate_gstreamer_elements(&full);
        assert!(res_full.embedded_audio);
        assert!(res_full.embedded_video);
        assert!(res_full.missing_elements.is_empty());
        assert!(res_full.reason.is_none());
    }

    #[test]
    fn mpv_external_player_args_enable_gui_window_and_decoded_title() {
        let url = "http://127.0.0.1:41234/stream?token=tok123&path=%2Fstorage%2Femulated%2F0%2FMusic%2F%E6%99%A8%E9%9B%BE%2001.mp3";
        assert_eq!(
            external_player_args("mpv", url),
            vec![
                "--player-operation-mode=pseudo-gui",
                "--force-window=yes",
                "--osc=yes",
                "--force-media-title=晨雾 01.mp3",
                "--",
                url,
            ]
        );
    }

    #[test]
    fn external_player_args_handles_missing_title_and_other_players() {
        let root_url = "http://127.0.0.1:41234/stream?path=%2F&token=tok123";
        assert_eq!(
            external_player_args("mpv", root_url),
            vec![
                "--player-operation-mode=pseudo-gui",
                "--force-window=yes",
                "--osc=yes",
                "--",
                root_url,
            ]
        );
        assert_eq!(external_player_args("vlc", root_url), vec![root_url]);
    }

    #[test]
    fn media_bridge_url_validator_checks_exact_origin_and_token() {
        assert!(is_valid_media_bridge_stream_url(
            "http://127.0.0.1:41234",
            "tok123",
            "http://127.0.0.1:41234/stream?path=%2Ffoo.mp4&token=tok123"
        ));
        assert!(!is_valid_media_bridge_stream_url(
            "http://127.0.0.1:41234",
            "tok123",
            "http://127.0.0.1:41234/stream?path=%2Ffoo.mp4&token=tok1234"
        ));
        assert!(!is_valid_media_bridge_stream_url(
            "http://127.0.0.1:41234",
            "tok123",
            "https://example.com/stream?token=tok123"
        ));
    }
}
