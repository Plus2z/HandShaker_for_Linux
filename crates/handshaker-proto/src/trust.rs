use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::crypto::RsaIdentity;
use crate::error::{ProtoError, Result};

const DATA_DIR_NAME: &str = "handshaker-linux";
const STORE_FILE_NAME: &str = "trust_store.json";
const M0_PROBE_FILE_NAME: &str = "m0_probe_identity.json";

/// Persisted pairing entry for one trusted Android phone.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrustedDevice {
    pub device_uuid: String,
    pub device_name: String,
    pub derived_key_b64: String,
    pub paired_at_unix: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TrustStoreDisk {
    host_uuid: String,
    private_key_pem: String,
    #[serde(default)]
    trusted: HashMap<String, TrustedDevice>,
}

#[derive(Debug, Deserialize)]
struct M0ProbeIdentityDisk {
    host_uuid: String,
    private_key_pem: String,
    #[serde(default)]
    trusted: HashMap<String, String>,
}

/// Persistent client identity (`host_uuid`, RSA-1024 private key, and `derived_key` map).
#[derive(Clone, Debug)]
pub struct IdentityStore {
    path: PathBuf,
    host_uuid: String,
    rsa: RsaIdentity,
    trusted: HashMap<String, TrustedDevice>,
}

impl IdentityStore {
    /// Returns the default store path under `$XDG_DATA_HOME/handshaker-linux/trust_store.json`.
    pub fn default_path() -> PathBuf {
        let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".local/share"));
        base.join(DATA_DIR_NAME).join(STORE_FILE_NAME)
    }

    /// Loads existing `trust_store.json`, migrates M0 probe identity if present, or generates a fresh store.
    pub fn load_or_create(path: impl AsRef<Path>, fresh: bool) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !fresh && path.exists() {
            let raw = fs::read_to_string(&path)?;
            let disk: TrustStoreDisk = serde_json::from_str(&raw)
                .map_err(|e| ProtoError::Crypto(format!("Corrupt trust store {}: {e}", path.display())))?;
            let rsa = RsaIdentity::from_pkcs8_pem(&disk.private_key_pem)?;
            return Ok(Self {
                path,
                host_uuid: disk.host_uuid,
                rsa,
                trusted: disk.trusted,
            });
        }

        if !fresh {
            if let Some(parent) = path.parent() {
                let probe_path = parent.join(M0_PROBE_FILE_NAME);
                if probe_path.exists() {
                    if let Ok(migrated) = Self::try_migrate_m0(&probe_path, &path) {
                        return Ok(migrated);
                    }
                }
            }
        }

        let rsa = RsaIdentity::generate()?;
        let store = Self {
            path,
            host_uuid: Uuid::new_v4().to_string(),
            rsa,
            trusted: HashMap::new(),
        };
        store.save()?;
        Ok(store)
    }

    fn try_migrate_m0(probe_path: &Path, target_path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(probe_path)?;
        let m0: M0ProbeIdentityDisk = serde_json::from_str(&raw)
            .map_err(|e| ProtoError::Crypto(format!("Parse m0 probe store failed: {e}")))?;
        let rsa = RsaIdentity::from_pkcs8_pem(&m0.private_key_pem)?;
        let now = chrono::Utc::now().timestamp();
        let trusted = m0
            .trusted
            .into_iter()
            .map(|(device_uuid, derived_key_b64)| {
                let entry = TrustedDevice {
                    device_uuid: device_uuid.clone(),
                    device_name: "Migrated Android Device".to_owned(),
                    derived_key_b64,
                    paired_at_unix: now,
                };
                (device_uuid, entry)
            })
            .collect();
        let store = Self {
            path: target_path.to_path_buf(),
            host_uuid: m0.host_uuid,
            rsa,
            trusted,
        };
        store.save()?;
        Ok(store)
    }

    /// Creates an in-memory ephemeral identity store (mainly for tests).
    pub fn ephemeral(path: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_path_buf(),
            host_uuid: Uuid::new_v4().to_string(),
            rsa: RsaIdentity::generate()?,
            trusted: HashMap::new(),
        })
    }

    pub fn host_uuid(&self) -> &str {
        &self.host_uuid
    }

    pub fn rsa(&self) -> &RsaIdentity {
        &self.rsa
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn derived_key_for(&self, device_uuid: &str) -> Result<Option<Vec<u8>>> {
        let Some(entry) = self.trusted.get(device_uuid) else {
            return Ok(None);
        };
        let bytes = BASE64
            .decode(entry.derived_key_b64.as_bytes())
            .map_err(|e| ProtoError::Crypto(format!("Invalid stored derived_key base64: {e}")))?;
        Ok(Some(bytes))
    }

    pub fn trusted_devices(&self) -> Vec<TrustedDevice> {
        let mut items: Vec<_> = self.trusted.values().cloned().collect();
        items.sort_by(|a, b| a.device_name.cmp(&b.device_name));
        items
    }

    pub fn save_trusted_device(
        &mut self,
        device_uuid: impl Into<String>,
        device_name: impl Into<String>,
        derived_key: &[u8],
    ) -> Result<()> {
        let device_uuid = device_uuid.into();
        let entry = TrustedDevice {
            device_uuid: device_uuid.clone(),
            device_name: device_name.into(),
            derived_key_b64: BASE64.encode(derived_key),
            paired_at_unix: chrono::Utc::now().timestamp(),
        };
        self.trusted.insert(device_uuid, entry);
        self.save()
    }

    pub fn remove_trusted_device(&mut self, device_uuid: &str) -> Result<bool> {
        let removed = self.trusted.remove(device_uuid).is_some();
        if removed {
            self.save()?;
        }
        Ok(removed)
    }

    /// Returns the set of trusted `device_uuid` values currently stored.
    pub fn trusted_uuids(&self) -> std::collections::HashSet<String> {
        self.trusted.keys().cloned().collect()
    }

    /// Writes trust store atomically and enforces `0600` POSIX permissions.
    pub fn save(&self) -> Result<()> {
        let disk = TrustStoreDisk {
            host_uuid: self.host_uuid.clone(),
            private_key_pem: self.rsa.to_pkcs8_pem()?,
            trusted: self.trusted.clone(),
        };
        let json = serde_json::to_string_pretty(&disk)
            .map_err(|e| ProtoError::Crypto(format!("Serialize trust store failed: {e}")))?;
        write_private_file(&self.path, &json)
    }
}

pub(crate) fn write_private_file(path: &Path, content: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
    }
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, content)?;
    fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o600))?;
    fs::rename(&tmp_path, path)?;
    Ok(())
}
