use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use arboard::Clipboard;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;

/// Echo-suppression tracker avoiding feedback loops between Linux clipboard and Android clipboard.
#[derive(Debug, Default)]
pub struct ClipboardDeduplicator {
    last_local_hash: Option<[u8; 32]>,
    suppressed_from_remote_hash: Option<[u8; 32]>,
}

impl ClipboardDeduplicator {
    pub fn digest(text: &str) -> [u8; 32] {
        Sha256::digest(text.as_bytes()).into()
    }

    /// Seeds initial local clipboard hash without pushing on app startup.
    pub fn seed_local(&mut self, text: &str) {
        self.last_local_hash = Some(Self::digest(text));
    }

    /// Returns `true` when changed text read from Linux should be uploaded to Android.
    pub fn observe_local_change(&mut self, text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let hash = Self::digest(text);
        if self.last_local_hash == Some(hash) {
            return false;
        }
        self.last_local_hash = Some(hash);
        if self.suppressed_from_remote_hash == Some(hash) {
            // Clear suppression once Linux reflects the remote write so future repeats work.
            self.suppressed_from_remote_hash = None;
            return false;
        }
        true
    }

    /// Marks text received from Android so polling Linux clipboard won't bounce it back.
    pub fn record_remote_to_local(&mut self, text: &str) {
        let hash = Self::digest(text);
        self.suppressed_from_remote_hash = Some(hash);
        self.last_local_hash = Some(hash);
    }
}

pub enum ClipboardWorkerCmd {
    SetLocal(String),
    SetEnabled(bool),
}

pub struct ClipboardBridge {
    cmd_tx: mpsc::UnboundedSender<ClipboardWorkerCmd>,
    dedup: Arc<Mutex<ClipboardDeduplicator>>,
}

impl ClipboardBridge {
    pub fn start() -> (Self, mpsc::UnboundedReceiver<String>) {
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<ClipboardWorkerCmd>();
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<String>();
        let dedup = Arc::new(Mutex::new(ClipboardDeduplicator::default()));
        let dedup_worker = Arc::clone(&dedup);

        thread::Builder::new()
            .name("hs-clipboard".into())
            .spawn(move || {
                let mut cb: Option<Clipboard> = None;
                let mut enabled = false;

                loop {
                    while let Ok(cmd) = cmd_rx.try_recv() {
                        match cmd {
                            ClipboardWorkerCmd::SetLocal(text) => {
                                dedup_worker
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .record_remote_to_local(&text);
                                if cb.is_none() {
                                    cb = Clipboard::new().ok();
                                }
                                if let Some(c) = cb.as_mut() {
                                    if c.set_text(text).is_err() {
                                        cb = None;
                                    }
                                }
                            }
                            ClipboardWorkerCmd::SetEnabled(v) => {
                                let becoming_enabled = v && !enabled;
                                enabled = v;
                                if becoming_enabled {
                                    if cb.is_none() {
                                        cb = Clipboard::new().ok();
                                    }
                                    if let Some(c) = cb.as_mut() {
                                        if let Ok(cur) = c.get_text() {
                                            dedup_worker
                                                .lock()
                                                .unwrap_or_else(|e| e.into_inner())
                                                .seed_local(&cur);
                                        }
                                    }
                                }
                            }
                        }
                    }

                    if enabled {
                        if cb.is_none() {
                            cb = Clipboard::new().ok();
                        }
                        if let Some(c) = cb.as_mut() {
                            match c.get_text() {
                                Ok(text) => {
                                    let should_push = dedup_worker
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .observe_local_change(&text);
                                    if should_push && outbound_tx.send(text).is_err() {
                                        break;
                                    }
                                }
                                Err(arboard::Error::ContentNotAvailable) => {}
                                Err(_) => cb = None,
                            }
                        }
                    }

                    thread::sleep(Duration::from_millis(900));
                }
            })
            .ok();

        (Self { cmd_tx, dedup }, outbound_rx)
    }

    pub fn set_enabled(&self, enabled: bool) {
        let _ = self.cmd_tx.send(ClipboardWorkerCmd::SetEnabled(enabled));
    }

    pub fn write_to_linux(&self, text: String) {
        let _ = self.cmd_tx.send(ClipboardWorkerCmd::SetLocal(text));
    }

    pub fn note_sent_to_phone(&self, text: &str) {
        let mut g = self.dedup.lock().unwrap_or_else(|e| e.into_inner());
        g.last_local_hash = Some(ClipboardDeduplicator::digest(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deduplicator_prevents_echo_loop_on_remote_write() {
        let mut d = ClipboardDeduplicator::default();
        d.seed_local("initial");
        assert!(!d.observe_local_change("initial"));
        assert!(!d.observe_local_change(""));

        // Local user copies new text -> should push to phone once
        assert!(d.observe_local_change("from linux"));
        assert!(!d.observe_local_change("from linux"));

        // Phone pushes "from phone" -> applied to Linux -> next poll must NOT re-upload it
        d.record_remote_to_local("from phone");
        assert!(!d.observe_local_change("from phone"));

        // Next genuine local copy pushes cleanly
        assert!(d.observe_local_change("new note"));
    }
}
