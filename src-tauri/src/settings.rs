use std::fs;
use std::path::{Component, PathBuf};

use crate::dto::{default_language, default_trash_dir, AppSettings, AppSettingsPatch};

const CONFIG_DIR_NAME: &str = "handshaker-linux";
const SETTINGS_FILE_NAME: &str = "settings.json";

pub fn is_valid_language(lang: &str) -> bool {
    matches!(lang, "system" | "en" | "zh-CN")
}

pub fn normalize_trash_dir(raw: &str) -> Option<String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.contains('\\') || trimmed.contains('\0') {
        return None;
    }
    let home = dirs::home_dir();
    let expanded = if trimmed == "~" {
        return None;
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        let rest_clean = rest.trim_start_matches('/');
        if rest_clean.is_empty() {
            return None;
        }
        home.as_ref()?.join(rest_clean)
    } else if trimmed == ".HandShakerTrash" {
        home.as_ref()?.join(".HandShakerTrash")
    } else if trimmed.starts_with('/') {
        PathBuf::from(trimmed)
    } else {
        return None;
    };

    if expanded
        .components()
        .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return None;
    }
    if let Some(ref h) = home {
        if expanded == *h {
            return None;
        }
    }
    let norm = expanded.to_string_lossy().trim_end_matches('/').to_owned();
    if norm.is_empty()
        || matches!(
            norm.as_str(),
            "/" | "/home" | "/root" | "/mnt" | "/media" | "/storage" | "/storage/emulated" | "/storage/emulated/0" | "/sdcard"
        )
    {
        return None;
    }
    Some(norm)
}

pub fn detect_tiling_wm() -> bool {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
        || std::env::var_os("SWAYSOCK").is_some()
        || std::env::var_os("I3SOCK").is_some()
    {
        return true;
    }
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(desktop.as_str(), "hyprland" | "sway" | "i3" | "river" | "niri")
}

pub fn default_download_dir() -> String {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(|| PathBuf::from("."))
        .to_string_lossy()
        .into_owned()
}

pub fn settings_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(".config"))
        .join(CONFIG_DIR_NAME)
        .join(SETTINGS_FILE_NAME)
}

pub fn load_settings() -> AppSettings {
    let is_tiling_wm = detect_tiling_wm();
    let path = settings_path();
    if let Ok(raw) = fs::read_to_string(&path) {
        if let Ok(mut parsed) = serde_json::from_str::<AppSettings>(&raw) {
            parsed.is_tiling_wm = is_tiling_wm;
            if parsed.download_dir.is_empty() {
                parsed.download_dir = default_download_dir();
            }
            let raw_trash = parsed.trash_dir.trim();
            if raw_trash.starts_with("/storage/emulated")
                || raw_trash == "/sdcard"
                || raw_trash.starts_with("/sdcard/")
            {
                parsed.trash_dir = default_trash_dir();
            } else {
                parsed.trash_dir = normalize_trash_dir(&parsed.trash_dir)
                    .unwrap_or_else(default_trash_dir);
            }
            if !is_valid_language(&parsed.language) {
                parsed.language = default_language();
            }
            return parsed;
        }
    }
    AppSettings {
        // Default off on Linux tiling WMs / Hyprland; users on floating DEs can toggle live.
        use_system_decorations: false,
        theme: "system".to_owned(),
        language: default_language(),
        download_dir: default_download_dir(),
        trash_dir: default_trash_dir(),
        clipboard_auto_sync: true,
        is_tiling_wm,
    }
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed creating config dir: {e}"))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed serializing settings: {e}"))?;
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, json).map_err(|e| format!("Failed writing temp settings: {e}"))?;
    fs::rename(&tmp_path, &path).map_err(|e| format!("Failed replacing settings file: {e}"))
}

pub fn apply_settings_patch(current: &mut AppSettings, patch: AppSettingsPatch) {
    if let Some(dec) = patch.use_system_decorations {
        current.use_system_decorations = dec;
    }
    if let Some(theme) = patch.theme {
        if matches!(theme.as_str(), "system" | "light" | "dark") {
            current.theme = theme;
        }
    }
    if let Some(lang) = patch.language {
        if is_valid_language(&lang) {
            current.language = lang;
        }
    }
    if let Some(dir) = patch.download_dir {
        if !dir.trim().is_empty() {
            current.download_dir = dir;
        }
    }
    if let Some(trash_dir) = patch.trash_dir {
        if let Some(normalized) = normalize_trash_dir(&trash_dir) {
            current.trash_dir = normalized;
        }
    }
    if let Some(sync) = patch.clipboard_auto_sync {
        current.clipboard_auto_sync = sync;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_settings_json_without_language_defaults_to_system() {
        let legacy = r#"{
            "useSystemDecorations": true,
            "theme": "dark",
            "downloadDir": "/tmp/dl",
            "clipboardAutoSync": false,
            "isTilingWm": false
        }"#;
        let parsed: AppSettings = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.language, "system");
        assert_eq!(parsed.trash_dir, default_trash_dir());
        assert_eq!(parsed.theme, "dark");
        assert!(parsed.use_system_decorations);
        assert!(!parsed.clipboard_auto_sync);
    }

    #[test]
    fn apply_settings_patch_validates_language_and_trash_dir() {
        let mut settings = AppSettings {
            use_system_decorations: false,
            theme: "system".to_owned(),
            language: "system".to_owned(),
            download_dir: "/tmp".to_owned(),
            trash_dir: default_trash_dir(),
            clipboard_auto_sync: true,
            is_tiling_wm: false,
        };
        apply_settings_patch(
            &mut settings,
            AppSettingsPatch {
                language: Some("zh-CN".to_owned()),
                trash_dir: Some("  /tmp/MyHandShakerTrash/  ".to_owned()),
                ..Default::default()
            },
        );
        assert_eq!(settings.language, "zh-CN");
        assert_eq!(settings.trash_dir, "/tmp/MyHandShakerTrash");

        apply_settings_patch(
            &mut settings,
            AppSettingsPatch {
                language: Some("invalid-locale".to_owned()),
                trash_dir: Some("../escape".to_owned()),
                ..Default::default()
            },
        );
        assert_eq!(settings.language, "zh-CN");
        assert_eq!(settings.trash_dir, "/tmp/MyHandShakerTrash");
        assert!(normalize_trash_dir("~").is_none());
        assert!(normalize_trash_dir("/home").is_none());
        assert!(normalize_trash_dir("/").is_none());
        assert!(normalize_trash_dir("~/.HandShakerTrash")
            .unwrap()
            .ends_with("/.HandShakerTrash"));
    }
}
