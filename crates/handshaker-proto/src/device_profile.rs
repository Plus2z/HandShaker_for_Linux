use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ProtoError, Result};
use crate::proto::SspGetDeviceInfoResponse;
use crate::session::HandshakeInfo;
use crate::trust::{write_private_file, IdentityStore};

/// Cached hardware & app metadata learned from HandShaker sessions (`GetDeviceInfo` / `Handshake`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, Eq, PartialEq)]
pub struct DeviceProfile {
    pub device_uuid: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(default)]
    pub phone_model: Option<String>,
    #[serde(default)]
    pub product_brand: Option<String>,
    #[serde(default)]
    pub product_manufacturer: Option<String>,
    #[serde(default)]
    pub apk_version_name: Option<String>,
    #[serde(default)]
    pub usb_serial: Option<String>,
    #[serde(default)]
    pub updated_at_unix: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DeviceProfileDisk {
    #[serde(default)]
    profiles: BTreeMap<String, DeviceProfile>,
}

/// Persistent non-secret device metadata cache stored next to `trust_store.json`.
#[derive(Clone, Debug)]
pub struct DeviceProfileStore {
    path: PathBuf,
    profiles: BTreeMap<String, DeviceProfile>,
}

impl DeviceProfileStore {
    pub fn default_path() -> PathBuf {
        IdentityStore::default_path().with_file_name("device_profiles.json")
    }

    pub fn load(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref().to_path_buf();
        let profiles = fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<DeviceProfileDisk>(&raw).ok())
            .map(|disk| disk.profiles)
            .unwrap_or_default();
        Self { path, profiles }
    }

    pub fn load_default() -> Self {
        Self::load(Self::default_path())
    }

    pub fn get(&self, device_uuid: &str) -> Option<&DeviceProfile> {
        self.profiles.get(device_uuid)
    }

    pub fn find_by_usb_serial(&self, serial: &str) -> Option<&DeviceProfile> {
        let target = non_empty_trimmed(Some(serial))?;
        self.profiles
            .values()
            .find(|p| p.usb_serial.as_deref().map(str::trim) == Some(target))
    }

    /// Merges non-empty metadata fields into the cached profile for `device_uuid` and persists it.
    pub fn upsert(&mut self, incoming: DeviceProfile) -> Result<()> {
        let Some(uuid) = non_empty_trimmed(Some(&incoming.device_uuid)).map(str::to_owned) else {
            return Ok(());
        };
        let entry = self
            .profiles
            .entry(uuid.clone())
            .or_insert_with(|| DeviceProfile {
                device_uuid: uuid,
                ..Default::default()
            });

        if let Some(name) = non_empty_trimmed(Some(&incoming.device_name)) {
            entry.device_name = name.to_owned();
        }
        merge_opt(&mut entry.phone_model, incoming.phone_model);
        merge_opt(&mut entry.product_brand, incoming.product_brand);
        merge_opt(
            &mut entry.product_manufacturer,
            incoming.product_manufacturer,
        );
        merge_opt(&mut entry.apk_version_name, incoming.apk_version_name);
        merge_opt(&mut entry.usb_serial, incoming.usb_serial);
        entry.updated_at_unix = chrono::Utc::now().timestamp();
        self.save()
    }

    pub fn save_handshake(&mut self, hs: &HandshakeInfo) -> Result<()> {
        self.upsert(DeviceProfile {
            device_uuid: hs.device_uuid.clone(),
            device_name: hs.device_name.clone(),
            apk_version_name: non_empty_owned(Some(hs.apk_version_name.clone())),
            ..Default::default()
        })
    }

    pub fn save_device_info(
        &mut self,
        device_uuid: &str,
        info: &SspGetDeviceInfoResponse,
    ) -> Result<()> {
        let uuid = non_empty_trimmed(Some(device_uuid))
            .or_else(|| non_empty_trimmed(info.phone_id.as_deref()))
            .unwrap_or_default()
            .to_owned();
        self.upsert(DeviceProfile {
            device_uuid: uuid,
            device_name: non_empty_trimmed(info.phone_name.as_deref())
                .unwrap_or_default()
                .to_owned(),
            phone_model: non_empty_owned(info.phone_model.clone()),
            product_brand: non_empty_owned(info.product_brand.clone()),
            product_manufacturer: non_empty_owned(info.product_manufacturer.clone()),
            apk_version_name: non_empty_owned(info.apk_version_name.clone()),
            usb_serial: None,
            updated_at_unix: 0,
        })
    }

    pub fn save(&self) -> Result<()> {
        let disk = DeviceProfileDisk {
            profiles: self.profiles.clone(),
        };
        let json = serde_json::to_string_pretty(&disk)
            .map_err(|e| ProtoError::Crypto(format!("Serialize device profiles failed: {e}")))?;
        write_private_file(&self.path, &json)
    }
}

pub(crate) fn non_empty_trimmed(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

pub(crate) fn non_empty_owned(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

fn merge_opt(slot: &mut Option<String>, incoming: Option<String>) {
    if let Some(val) = non_empty_owned(incoming) {
        *slot = Some(val);
    }
}

/// Formats a concise non-redundant brand/model label (e.g. `vivo V2419A` or `OnePlus GM1917`).
pub fn format_brand_and_model(
    brand: Option<&str>,
    manufacturer: Option<&str>,
    model: Option<&str>,
) -> Option<String> {
    let vendor = non_empty_trimmed(brand).or_else(|| non_empty_trimmed(manufacturer));
    let model = non_empty_trimmed(model);

    match (vendor, model) {
        (Some(v), Some(m)) => {
            if m.to_ascii_lowercase().starts_with(&v.to_ascii_lowercase()) {
                Some(m.to_owned())
            } else {
                Some(format!("{v} {m}"))
            }
        }
        (Some(v), None) => Some(v.to_owned()),
        (None, Some(m)) => Some(m.to_owned()),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::TransportKind;
    use tempfile::tempdir;

    #[test]
    fn formats_brand_and_model_without_duplication() {
        assert_eq!(
            format_brand_and_model(Some("OnePlus"), Some("OnePlus"), Some("GM1917")).as_deref(),
            Some("OnePlus GM1917")
        );
        assert_eq!(
            format_brand_and_model(Some("vivo"), Some("vivo"), Some("vivo X200 Pro mini"))
                .as_deref(),
            Some("vivo X200 Pro mini")
        );
        assert_eq!(
            format_brand_and_model(None, Some("SAMSUNG"), Some("SM-G998B")).as_deref(),
            Some("SAMSUNG SM-G998B")
        );
    }

    #[test]
    fn merges_handshake_without_wiping_cached_hardware_model() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("device_profiles.json");
        let mut store = DeviceProfileStore::load(&path);
        let resp = SspGetDeviceInfoResponse {
            phone_name: Some("OnePlus 7Pro".to_owned()),
            phone_model: Some("GM1917".to_owned()),
            product_brand: Some("OnePlus".to_owned()),
            product_manufacturer: Some("OnePlus".to_owned()),
            apk_version_name: Some("2.5.0".to_owned()),
            phone_id: Some("uuid-1".to_owned()),
            ..Default::default()
        };
        store.save_device_info("uuid-1", &resp).unwrap();

        let hs = HandshakeInfo {
            device_uuid: "uuid-1".to_owned(),
            device_name: "OnePlus 7Pro Renamed".to_owned(),
            apk_version: "250".to_owned(),
            apk_version_name: "2.5.1".to_owned(),
            trust_type: 5,
            transport: TransportKind::WifiTcp,
            transport_label: "192.168.0.101:37831".to_owned(),
            is_smartisan_device: true,
        };
        store.save_handshake(&hs).unwrap();

        let reloaded = DeviceProfileStore::load(&path);
        let p = reloaded.get("uuid-1").unwrap();
        assert_eq!(p.device_name, "OnePlus 7Pro Renamed");
        assert_eq!(p.phone_model.as_deref(), Some("GM1917"));
        assert_eq!(p.product_brand.as_deref(), Some("OnePlus"));
        assert_eq!(p.apk_version_name.as_deref(), Some("2.5.1"));
    }
}
