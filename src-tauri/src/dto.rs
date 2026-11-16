use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub fn default_language() -> String {
    "system".to_owned()
}

pub fn default_trash_dir() -> String {
    dirs::home_dir()
        .map(|h| h.join(".HandShakerTrash"))
        .unwrap_or_else(|| PathBuf::from(".HandShakerTrash"))
        .to_string_lossy()
        .into_owned()
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub use_system_decorations: bool,
    pub theme: String, // "system" | "light" | "dark"
    #[serde(default = "default_language")]
    pub language: String, // "system" | "en" | "zh-CN"
    pub download_dir: String,
    #[serde(default = "default_trash_dir")]
    pub trash_dir: String,
    pub clipboard_auto_sync: bool,
    pub is_tiling_wm: bool,
}

#[derive(Clone, Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AppSettingsPatch {
    pub use_system_decorations: Option<bool>,
    pub theme: Option<String>,
    pub language: Option<String>,
    pub download_dir: Option<String>,
    pub trash_dir: Option<String>,
    pub clipboard_auto_sync: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WifiCandidateDto {
    pub service_name: String,
    pub display_name: String,
    pub host: String,
    pub port: u16,
    pub addresses: Vec<String>,
    pub hostname: Option<String>,
    pub device_uuid: Option<String>,
    pub device_name: Option<String>,
    pub phone_model: Option<String>,
    pub product_brand: Option<String>,
    pub product_manufacturer: Option<String>,
    pub apk_version_name: Option<String>,
    pub is_trusted: bool,
    pub is_locked: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbCandidateDto {
    pub bus: u8,
    pub address: u8,
    pub vendor_id: u16,
    pub product_id: u16,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial: Option<String>,
    pub already_aoa: bool,
    pub interface_count: u8,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoverySnapshotDto {
    pub wifi: Vec<WifiCandidateDto>,
    pub usb: Vec<UsbCandidateDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedDeviceDto {
    pub device_uuid: String,
    pub device_name: String,
    pub paired_at_unix: i64,
    pub phone_model: Option<String>,
    pub product_brand: Option<String>,
    pub product_manufacturer: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HandshakeDto {
    pub device_uuid: String,
    pub device_name: String,
    pub apk_version: String,
    pub apk_version_name: String,
    pub trust_type: i32,
    pub transport: String,
    pub transport_label: String,
    pub is_smartisan_device: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStateDto {
    pub status: String, // "disconnected" | "connecting" | "trust_waiting" | "connected"
    pub handshake: Option<HandshakeDto>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfoDto {
    pub phone_name: String,
    pub phone_model: String,
    pub product_brand: String,
    pub product_manufacturer: String,
    pub apk_version_name: String,
    pub root_path: String,
    pub external_storage_path: Option<String>,
    pub battery_percentage: u32,
    pub disk_size: u64,
    pub used_disk_size: u64,
    pub free_disk_size: u64,
    pub pic_video_size: u64,
    pub audio_size: u64,
    pub download_size: u64,
    pub app_size: u64,
    pub cache_size: u64,
    pub other_size: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntryDto {
    pub name: String,
    pub path: String,
    pub is_directory: bool,
    pub file_size: u64,
    pub created_timestamp: u64,
    pub modified_timestamp: u64,
    pub is_trash: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TransferItemDto {
    pub id: String,
    pub direction: String, // "download" | "upload"
    pub file_name: String,
    pub local_path: String,
    pub remote_path: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub status: String, // "queued" | "active" | "completed" | "failed" | "cancelled"
    pub error: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaBridgeInfoDto {
    pub base_url: String,
    pub token: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoAlbumDto {
    pub album_id: u64,
    pub album_name: String,
    pub album_path: String,
    pub cover_thumb_url: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageItemDto {
    pub media_id: u64,
    pub album_id: u64,
    pub album_name: String,
    pub path: String,
    pub title: String,
    pub mime_type: String,
    pub file_size: u64,
    pub width: u32,
    pub height: u32,
    pub orientation: u32,
    pub date_taken: u64,
    pub modified_timestamp: u64,
    pub starred: bool,
    pub thumb_url: String,
    pub stream_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoLibraryDto {
    pub camera_album_id: Option<u64>,
    pub albums: Vec<PhotoAlbumDto>,
    pub images: Vec<ImageItemDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAlbumDto {
    pub album_id: u64,
    pub album_name: String,
    pub album_path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoItemDto {
    pub media_id: u64,
    pub album_id: u64,
    pub path: String,
    pub name: String,
    pub mime_type: String,
    pub file_size: u64,
    pub width: u32,
    pub height: u32,
    pub duration_secs: f64,
    pub modified_timestamp: u32,
    pub thumb_url: String,
    pub stream_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoLibraryDto {
    pub albums: Vec<VideoAlbumDto>,
    pub videos: Vec<VideoItemDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioAlbumDto {
    pub album_id: u64,
    pub album_name: String,
    pub album_path: String,
    pub artist: String,
    pub year: u32,
    pub thumb_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioItemDto {
    pub media_id: u64,
    pub album_id: u64,
    pub path: String,
    pub title: String,
    pub artist: String,
    pub mime_type: String,
    pub file_size: u64,
    pub duration_secs: f64,
    pub bitrate: u32,
    pub sample_rate: f64,
    pub year: u32,
    pub track: u32,
    pub stream_url: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioLibraryDto {
    pub albums: Vec<AudioAlbumDto>,
    pub tracks: Vec<AudioItemDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardItemDto {
    pub text: String,
    pub timestamp_ms: i64,
    pub raw_gzip_b64: String,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MediaPlaybackSupportDto {
    pub embedded_audio: bool,
    pub embedded_video: bool,
    pub missing_elements: Vec<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TrashItemDto {
    pub entry_name: String,
    pub is_directory: bool,
    pub size_bytes: u64,
    pub original_path: Option<String>,
    pub deletion_date: Option<String>,
    pub device_uuid: Option<String>,
    pub device_name: Option<String>,
    pub device_model: Option<String>,
    pub modified_timestamp_ms: i64,
}
