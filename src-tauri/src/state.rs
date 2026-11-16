use std::sync::Arc;

use handshaker_proto::{IdentityStore, Session, SessionEvent};
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, RwLock};

use crate::clipboard_sync::ClipboardBridge;
use crate::dto::{AppSettings, ClipboardItemDto, ConnectionStateDto, HandshakeDto};
use crate::media_bridge::MediaBridge;
use crate::settings::load_settings;
use crate::transfers::TransferManager;

pub struct AppState {
    pub session_slot: Arc<RwLock<Option<Session>>>,
    pub connection_state: Arc<RwLock<ConnectionStateDto>>,
    pub identity_mutex: Arc<Mutex<()>>,
    pub settings: Arc<RwLock<AppSettings>>,
    pub media_bridge: MediaBridge,
    pub transfers: TransferManager,
    pub clipboard_bridge: Arc<ClipboardBridge>,
}

impl AppState {
    pub async fn init(app: AppHandle) -> anyhow::Result<Self> {
        let session_slot: Arc<RwLock<Option<Session>>> = Arc::new(RwLock::new(None));
        let connection_state = Arc::new(RwLock::new(ConnectionStateDto {
            status: "disconnected".to_owned(),
            handshake: None,
            error: None,
        }));
        let settings = Arc::new(RwLock::new(load_settings()));
        let media_bridge = MediaBridge::start(Arc::clone(&session_slot)).await?;

        let app_for_tx = app.clone();
        let transfers = TransferManager::new(
            Arc::clone(&session_slot),
            Arc::new(move |items| {
                let _ = app_for_tx.emit("transfer://updated", items);
            }),
        );

        let (clipboard_bridge, mut outbound_rx) = ClipboardBridge::start();
        let clipboard_bridge = Arc::new(clipboard_bridge);

        // Background task forwarding Linux OS clipboard changes to the phone when connected.
        let sess_clip = Arc::clone(&session_slot);
        let settings_clip = Arc::clone(&settings);
        let app_clip = app.clone();
        tokio::spawn(async move {
            while let Some(text) = outbound_rx.recv().await {
                if !settings_clip.read().await.clipboard_auto_sync {
                    continue;
                }
                let sess = { sess_clip.read().await.clone() };
                if let Some(session) = sess {
                    if session.post_clipboard(&text).await.is_ok() {
                        if let Ok(entries) = session.get_clipboard().await {
                            let dtos = map_clipboard_dtos(&entries);
                            let _ = app_clip.emit("clipboard://changed", dtos);
                        }
                    }
                }
            }
        });

        Ok(Self {
            session_slot,
            connection_state,
            identity_mutex: Arc::new(Mutex::new(())),
            settings,
            media_bridge,
            transfers,
            clipboard_bridge,
        })
    }

    pub fn load_identity_store(&self) -> Result<IdentityStore, String> {
        IdentityStore::load_or_create(IdentityStore::default_path(), false).map_err(|e| e.to_string())
    }

    pub async fn attach_session(&self, app: AppHandle, session: Session) {
        let hs = session.handshake_info().clone();
        let hs_dto = HandshakeDto {
            device_uuid: hs.device_uuid,
            device_name: hs.device_name,
            apk_version: hs.apk_version,
            apk_version_name: hs.apk_version_name,
            trust_type: hs.trust_type,
            transport: hs.transport.to_string(),
            transport_label: hs.transport_label,
            is_smartisan_device: hs.is_smartisan_device,
        };

        {
            *self.session_slot.write().await = Some(session.clone());
            let mut cs = self.connection_state.write().await;
            cs.status = "connected".to_owned();
            cs.handshake = Some(hs_dto.clone());
            cs.error = None;
            let _ = app.emit("connection://state", cs.clone());
        }

        let auto_sync = self.settings.read().await.clipboard_auto_sync;
        self.clipboard_bridge.set_enabled(auto_sync);

        let mut ev_rx = session.subscribe();
        let sess_slot = Arc::clone(&self.session_slot);
        let conn_state = Arc::clone(&self.connection_state);
        let cb_bridge = Arc::clone(&self.clipboard_bridge);
        let settings_ref = Arc::clone(&self.settings);

        tokio::spawn(async move {
            while let Ok(ev) = ev_rx.recv().await {
                match ev {
                    SessionEvent::ClipboardChanged(entries) => {
                        let dtos = map_clipboard_dtos(&entries);
                        if settings_ref.read().await.clipboard_auto_sync {
                            if let Some(latest) = entries.iter().max_by_key(|e| e.timestamp_ms) {
                                cb_bridge.write_to_linux(latest.text.clone());
                            }
                        }
                        let _ = app.emit("clipboard://changed", dtos);
                    }
                    SessionEvent::PhotoLibChanged { .. } => {
                        let _ = app.emit("library://changed", "photo");
                    }
                    SessionEvent::VideoLibChanged { .. } => {
                        let _ = app.emit("library://changed", "video");
                    }
                    SessionEvent::AudioLibChanged(_) => {
                        let _ = app.emit("library://changed", "audio");
                    }
                    SessionEvent::FolderChanged(_) | SessionEvent::FileChanged(_) => {
                        let _ = app.emit("folder://changed", ());
                    }
                    SessionEvent::QuitRequested | SessionEvent::Disconnected { .. } => {
                        let reason = match ev {
                            SessionEvent::Disconnected { reason } => Some(reason),
                            _ => None,
                        };
                        *sess_slot.write().await = None;
                        cb_bridge.set_enabled(false);
                        let mut cs = conn_state.write().await;
                        cs.status = "disconnected".to_owned();
                        cs.handshake = None;
                        cs.error = reason;
                        let _ = app.emit("connection://state", cs.clone());
                        break;
                    }
                    _ => {}
                }
            }
        });
    }
}

pub fn map_clipboard_dtos(entries: &[handshaker_proto::ClipboardEntry]) -> Vec<ClipboardItemDto> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    let mut dtos: Vec<_> = entries
        .iter()
        .map(|e| ClipboardItemDto {
            text: e.text.clone(),
            timestamp_ms: e.timestamp_ms,
            raw_gzip_b64: BASE64.encode(&e.raw_gzip),
        })
        .collect();
    dtos.sort_by_key(|b| std::cmp::Reverse(b.timestamp_ms));
    dtos
}
