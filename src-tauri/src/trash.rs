use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use chrono::Local;
use handshaker_proto::{HandshakeInfo, Session};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::dto::TrashItemDto;
use crate::settings::normalize_trash_dir;
use crate::transfers::{expand_remote_downloads, sanitize_child_name, DownloadTarget};

const FILES_SUBDIR: &str = "files";
const INFO_SUBDIR: &str = "info";
const TRASHINFO_EXT: &str = ".trashinfo";
const DEFAULT_RESTORE_ROOT: &str = "/storage/emulated/0";
const KEY_DEVICE_UUID: &str = "X-HandShaker-Device-UUID=";
const KEY_DEVICE_NAME: &str = "X-HandShaker-Device-Name=";
const KEY_DEVICE_MODEL: &str = "X-HandShaker-Device-Model=";

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TrashDeviceOrigin {
    pub device_uuid: Option<String>,
    pub device_name: Option<String>,
    pub device_model: Option<String>,
}

impl TrashDeviceOrigin {
    pub fn format_label(&self) -> Option<String> {
        let name = clean_non_empty(self.device_name.as_deref());
        let model = clean_non_empty(self.device_model.as_deref());
        match (name, model) {
            (Some(n), Some(m)) if !n.eq_ignore_ascii_case(&m) => Some(format!("{n} ({m})")),
            (Some(n), _) => Some(n),
            (None, Some(m)) => Some(m),
            (None, None) => clean_non_empty(self.device_uuid.as_deref()),
        }
    }
}

pub fn origin_from_handshake(hs: &HandshakeInfo) -> TrashDeviceOrigin {
    TrashDeviceOrigin {
        device_uuid: clean_non_empty(Some(&hs.device_uuid)),
        device_name: clean_non_empty(Some(&hs.device_name)),
        device_model: None,
    }
}

fn clean_non_empty(raw: Option<&str>) -> Option<String> {
    raw.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RemoteMeta {
    is_directory: bool,
    file_size: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct TrashInfoMeta {
    original_path: Option<String>,
    deletion_date: Option<String>,
    device_uuid: Option<String>,
    device_name: Option<String>,
    device_model: Option<String>,
}

impl TrashInfoMeta {
    fn device_origin(&self) -> TrashDeviceOrigin {
        TrashDeviceOrigin {
            device_uuid: self.device_uuid.clone(),
            device_name: self.device_name.clone(),
            device_model: self.device_model.clone(),
        }
    }
}

struct PreparedRestoreItem {
    local_entry: PathBuf,
    info_path: PathBuf,
    target_remote: String,
    is_dir: bool,
}

/// Resolves and validates the Linux trash root directory (expanding `~/.HandShakerTrash`).
pub fn resolve_trash_root(raw_dir: &str) -> Result<PathBuf, String> {
    let normalized = normalize_trash_dir(raw_dir)
        .ok_or_else(|| format!("Invalid Linux trash directory: {raw_dir:?}"))?;
    Ok(PathBuf::from(normalized))
}

/// Lists items currently stored inside `<trash_dir>/files` along with `.trashinfo` metadata.
pub async fn list_trash_items(trash_dir: &str) -> Result<Vec<TrashItemDto>, String> {
    let trash_root = resolve_trash_root(trash_dir)?;
    let files_dir = trash_root.join(FILES_SUBDIR);
    let info_dir = trash_root.join(INFO_SUBDIR);
    if !files_dir.exists() {
        return Ok(Vec::new());
    }

    let mut rd = tokio::fs::read_dir(&files_dir)
        .await
        .map_err(|e| format!("Failed reading trash dir {}: {e}", files_dir.display()))?;
    let mut items = Vec::new();

    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| format!("Failed scanning trash dir {}: {e}", files_dir.display()))?
    {
        let entry_name = entry.file_name().to_string_lossy().into_owned();
        if entry_name.is_empty() || entry_name.starts_with(".staging-") {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = tokio::fs::symlink_metadata(&path).await else {
            continue;
        };
        if meta.file_type().is_symlink() {
            continue;
        }

        let is_directory = meta.is_dir();
        let size_bytes = if is_directory {
            compute_dir_size(&path).await
        } else {
            meta.len()
        };
        let modified_timestamp_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let info_path = info_dir.join(format!("{entry_name}{TRASHINFO_EXT}"));
        let parsed = match tokio::fs::read_to_string(&info_path).await {
            Ok(raw) => parse_trashinfo(&raw),
            Err(_) => TrashInfoMeta::default(),
        };

        items.push(TrashItemDto {
            entry_name,
            is_directory,
            size_bytes,
            original_path: parsed.original_path,
            deletion_date: parsed.deletion_date,
            device_uuid: parsed.device_uuid,
            device_name: parsed.device_name,
            device_model: parsed.device_model,
            modified_timestamp_ms,
        });
    }

    items.sort_by(|a, b| {
        b.deletion_date
            .cmp(&a.deletion_date)
            .then_with(|| b.modified_timestamp_ms.cmp(&a.modified_timestamp_ms))
            .then_with(|| a.entry_name.cmp(&b.entry_name))
    });
    Ok(items)
}

/// Restores selected items from Linux trash back to their original phone paths after verifying
/// that every item originated from the currently connected phone, removing each local copy +
/// `.trashinfo` only after phone upload succeeds.
pub async fn restore_trash_items(
    session: &Session,
    trash_dir: &str,
    entry_names: &[String],
    current_origin: Option<&TrashDeviceOrigin>,
    fallback_root: Option<&str>,
) -> Result<Vec<String>, String> {
    let resolved_origin = current_origin
        .cloned()
        .unwrap_or_else(|| origin_from_handshake(session.handshake_info()));
    let unique_entries = sanitize_unique_entry_names(entry_names)?;
    if unique_entries.is_empty() {
        return Ok(Vec::new());
    }

    let trash_root = resolve_trash_root(trash_dir)?;
    let plan = prepare_restore_plan(&trash_root, &unique_entries, &resolved_origin, fallback_root)
        .await?;

    let mut restored_paths = Vec::with_capacity(plan.len());

    for item in plan {
        if session
            .file_exists(&item.target_remote)
            .await
            .map_err(|e| format!("Failed checking target path on phone {}: {e}", item.target_remote))?
        {
            return Err(format!("Target already exists on phone: {}", item.target_remote));
        }

        if let Some((parent_remote, _)) = item.target_remote.trim_end_matches('/').rsplit_once('/') {
            if !parent_remote.is_empty() {
                ensure_remote_dir_chain(session, parent_remote).await?;
            }
        }

        upload_entry_to_phone(session, &item.local_entry, &item.target_remote, item.is_dir).await?;

        remove_entry_path(&item.local_entry).await;
        let _ = tokio::fs::remove_file(&item.info_path).await;
        restored_paths.push(item.target_remote);
    }

    Ok(restored_paths)
}

async fn prepare_restore_plan(
    trash_root: &Path,
    unique_entries: &[String],
    current_origin: &TrashDeviceOrigin,
    fallback_root: Option<&str>,
) -> Result<Vec<PreparedRestoreItem>, String> {
    let files_dir = trash_root.join(FILES_SUBDIR);
    let info_dir = trash_root.join(INFO_SUBDIR);
    let fallback_clean = fallback_root
        .map(str::trim)
        .filter(|s| s.starts_with('/'))
        .unwrap_or(DEFAULT_RESTORE_ROOT)
        .trim_end_matches('/');

    let mut plan = Vec::with_capacity(unique_entries.len());

    for entry_name in unique_entries {
        let local_entry = files_dir.join(entry_name);
        let meta = tokio::fs::symlink_metadata(&local_entry)
            .await
            .map_err(|_| format!("Trash item not found: {entry_name}"))?;
        if meta.file_type().is_symlink() {
            return Err(format!("Refusing to restore symlink: {entry_name}"));
        }

        let info_path = info_dir.join(format!("{entry_name}{TRASHINFO_EXT}"));
        let parsed = match tokio::fs::read_to_string(&info_path).await {
            Ok(raw) => parse_trashinfo(&raw),
            Err(_) => TrashInfoMeta::default(),
        };

        verify_restore_device(entry_name, &parsed, current_origin)?;

        let target_remote = parsed
            .original_path
            .filter(|p| is_safe_remote_restore_path(p))
            .unwrap_or_else(|| format!("{fallback_clean}/{entry_name}"));

        plan.push(PreparedRestoreItem {
            local_entry,
            info_path,
            target_remote,
            is_dir: meta.is_dir(),
        });
    }

    Ok(plan)
}

fn verify_restore_device(
    entry_name: &str,
    parsed: &TrashInfoMeta,
    current_origin: &TrashDeviceOrigin,
) -> Result<(), String> {
    let recorded_uuid = clean_non_empty(parsed.device_uuid.as_deref());
    let current_uuid = clean_non_empty(current_origin.device_uuid.as_deref());

    let Some(expected_uuid) = recorded_uuid else {
        if let Some(source_label) = parsed.device_origin().format_label() {
            return Err(format!(
                "Device mismatch for \"{entry_name}\": deleted from {source_label}, cannot verify or restore to the currently connected phone."
            ));
        }
        return Err(format!(
            "Missing source phone metadata for \"{entry_name}\": cannot verify that this file was deleted from the currently connected phone."
        ));
    };

    let Some(actual_uuid) = current_uuid else {
        return Err(format!(
            "Cannot verify currently connected phone identity to restore \"{entry_name}\"."
        ));
    };

    if expected_uuid != actual_uuid {
        let source_label = parsed
            .device_origin()
            .format_label()
            .unwrap_or(expected_uuid);
        return Err(format!(
            "Device mismatch for \"{entry_name}\": deleted from {source_label}, cannot restore to the currently connected phone."
        ));
    }

    Ok(())
}

/// Permanently deletes specific entries (and their `.trashinfo`) from the Linux trash directory.
pub async fn delete_trash_items(trash_dir: &str, entry_names: &[String]) -> Result<usize, String> {
    let unique_entries = sanitize_unique_entry_names(entry_names)?;
    if unique_entries.is_empty() {
        return Ok(0);
    }

    let trash_root = resolve_trash_root(trash_dir)?;
    let files_dir = trash_root.join(FILES_SUBDIR);
    let info_dir = trash_root.join(INFO_SUBDIR);

    let mut removed = 0;
    for entry_name in unique_entries {
        let local_entry = files_dir.join(&entry_name);
        let info_path = info_dir.join(format!("{entry_name}{TRASHINFO_EXT}"));
        if local_entry.exists() {
            remove_entry_path(&local_entry).await;
            removed += 1;
        }
        if info_path.exists() {
            let _ = tokio::fs::remove_file(&info_path).await;
        }
    }
    Ok(removed)
}

/// Empties all items and metadata inside the Linux trash directory.
pub async fn empty_trash(trash_dir: &str) -> Result<usize, String> {
    let trash_root = resolve_trash_root(trash_dir)?;
    let files_dir = trash_root.join(FILES_SUBDIR);
    let info_dir = trash_root.join(INFO_SUBDIR);

    let mut cleared = 0;
    if files_dir.exists() {
        let mut rd = tokio::fs::read_dir(&files_dir)
            .await
            .map_err(|e| format!("Failed reading {}: {e}", files_dir.display()))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| format!("Failed enumerating {}: {e}", files_dir.display()))?
        {
            remove_entry_path(&entry.path()).await;
            cleared += 1;
        }
    }

    if info_dir.exists() {
        let mut rd = tokio::fs::read_dir(&info_dir)
            .await
            .map_err(|e| format!("Failed reading {}: {e}", info_dir.display()))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| format!("Failed enumerating {}: {e}", info_dir.display()))?
        {
            remove_entry_path(&entry.path()).await;
        }
    }

    if trash_root.exists() {
        if let Ok(mut rd) = tokio::fs::read_dir(&trash_root).await {
            while let Ok(Some(entry)) = rd.next_entry().await {
                if entry.file_name().to_string_lossy().starts_with(".staging-") {
                    remove_entry_path(&entry.path()).await;
                }
            }
        }
    }

    Ok(cleared)
}

/// Moves remote files/directories on the phone to the Linux trash directory
/// (`<trash_root>/files/<entry>` + `<trash_root>/info/<entry>.trashinfo`), recording the source
/// phone identity before permanently removing each safely backed-up item from the phone.
pub async fn move_remote_paths_to_linux_trash(
    session: &Session,
    paths: &[String],
    trash_dir: &str,
    origin: Option<&TrashDeviceOrigin>,
) -> Result<(), String> {
    let resolved_origin = origin
        .cloned()
        .unwrap_or_else(|| origin_from_handshake(session.handshake_info()));
    let mut seen = HashSet::new();
    let unique_paths: Vec<String> = paths
        .iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty() && seen.insert(p.clone()))
        .collect();
    if unique_paths.is_empty() {
        return Ok(());
    }

    let trash_root = resolve_trash_root(trash_dir)?;
    let files_dir = trash_root.join(FILES_SUBDIR);
    let info_dir = trash_root.join(INFO_SUBDIR);
    tokio::fs::create_dir_all(&files_dir)
        .await
        .map_err(|e| format!("Failed creating trash files dir {}: {e}", files_dir.display()))?;
    tokio::fs::create_dir_all(&info_dir)
        .await
        .map_err(|e| format!("Failed creating trash info dir {}: {e}", info_dir.display()))?;

    let metas = resolve_remote_metadata(session, &unique_paths).await?;

    for (remote_path, meta) in unique_paths.into_iter().zip(metas) {
        move_one_item_to_trash(
            session,
            &remote_path,
            meta,
            &resolved_origin,
            &trash_root,
            &files_dir,
            &info_dir,
        )
        .await?;
    }
    Ok(())
}

async fn resolve_remote_metadata(
    session: &Session,
    paths: &[String],
) -> Result<Vec<RemoteMeta>, String> {
    let mut parent_cache: HashMap<String, HashMap<String, RemoteMeta>> = HashMap::new();
    let mut out = Vec::with_capacity(paths.len());

    for remote_path in paths {
        let trimmed = remote_path.trim_end_matches('/');
        let parent = match trimmed.rsplit_once('/') {
            Some(("", _)) => "/".to_owned(),
            Some((p, _)) => p.to_owned(),
            None => "/".to_owned(),
        };

        if !parent_cache.contains_key(&parent) {
            let mut entries = HashMap::new();
            if let Ok(resp) = session.list_dir(&parent, 1).await {
                for f in resp.file {
                    if let Some(p) = f.path {
                        let key = p.trim_end_matches('/').to_owned();
                        entries.insert(
                            key,
                            RemoteMeta {
                                is_directory: f.is_directory.unwrap_or(false),
                                file_size: f.file_size.unwrap_or(0),
                            },
                        );
                    }
                }
            }
            parent_cache.insert(parent.clone(), entries);
        }

        if let Some(meta) = parent_cache
            .get(&parent)
            .and_then(|map| map.get(trimmed))
            .cloned()
        {
            out.push(meta);
        } else {
            out.push(RemoteMeta {
                is_directory: false,
                file_size: 0,
            });
        }
    }

    Ok(out)
}

async fn move_one_item_to_trash(
    session: &Session,
    remote_path: &str,
    meta: RemoteMeta,
    origin: &TrashDeviceOrigin,
    trash_root: &Path,
    files_dir: &Path,
    info_dir: &Path,
) -> Result<(), String> {
    let base_name = sanitize_child_name(remote_path)?;
    let entry_name = allocate_entry_name(files_dir, info_dir, &base_name, meta.is_directory)?;

    let stage_dir = trash_root.join(format!(".staging-{}", Uuid::new_v4()));
    tokio::fs::create_dir_all(&stage_dir)
        .await
        .map_err(|e| format!("Failed creating staging dir {}: {e}", stage_dir.display()))?;

    let stage_result = async {
        let staged_entry = if meta.is_directory {
            let target = DownloadTarget {
                remote_path: remote_path.to_owned(),
                is_directory: true,
                file_size: meta.file_size,
            };
            let expanded = expand_remote_downloads(session, &[target], &stage_dir).await?;
            for (child_remote, child_local, _) in expanded {
                download_remote_file(session, &child_remote, &child_local).await?;
            }
            stage_dir.join(&base_name)
        } else {
            let staged_file = stage_dir.join(&entry_name);
            download_remote_file(session, remote_path, &staged_file).await?;
            staged_file
        };

        let info_path = info_dir.join(format!("{entry_name}{TRASHINFO_EXT}"));
        let trashinfo = format_trashinfo(
            remote_path,
            &Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            origin,
        );
        tokio::fs::write(&info_path, trashinfo.as_bytes())
            .await
            .map_err(|e| format!("Failed writing trash metadata {}: {e}", info_path.display()))?;

        let final_dst = files_dir.join(&entry_name);
        if let Err(e) = tokio::fs::rename(&staged_entry, &final_dst).await {
            let _ = tokio::fs::remove_file(&info_path).await;
            return Err(format!(
                "Failed moving {} into trash {}: {e}",
                staged_entry.display(),
                final_dst.display()
            ));
        }

        Ok::<(PathBuf, PathBuf), String>((final_dst, info_path))
    }
    .await;

    let _ = tokio::fs::remove_dir_all(&stage_dir).await;

    let (final_dst, info_path) = stage_result?;

    if let Err(e) = session.delete_files(&[remote_path.to_owned()], false).await {
        // Keep move atomic: if the phone failed to remove the original, roll back local trash copy.
        remove_entry_path(&final_dst).await;
        let _ = tokio::fs::remove_file(&info_path).await;
        return Err(format!(
            "Saved trash backup, but failed removing {remote_path} from phone: {e}"
        ));
    }

    Ok(())
}

async fn ensure_remote_dir_chain(session: &Session, remote_dir: &str) -> Result<(), String> {
    let clean = remote_dir.trim_end_matches('/');
    if clean.is_empty() || clean == "/" {
        return Ok(());
    }
    let mut missing = Vec::new();
    let mut cur = clean.to_owned();
    loop {
        match session.file_exists(&cur).await {
            Ok(true) => break,
            Ok(false) => {
                missing.push(cur.clone());
                match cur.rsplit_once('/') {
                    Some((parent, _)) if !parent.is_empty() => cur = parent.to_owned(),
                    _ => break,
                }
            }
            Err(e) => return Err(format!("Failed checking remote dir {cur}: {e}")),
        }
    }
    for dir in missing.into_iter().rev() {
        ensure_remote_dir_single(session, &dir).await?;
    }
    Ok(())
}

async fn ensure_remote_dir_single(session: &Session, remote_dir: &str) -> Result<(), String> {
    match session.file_exists(remote_dir).await {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => return Err(format!("Failed checking remote folder {remote_dir}: {e}")),
    }
    match session.create_folder(remote_dir).await {
        Ok(()) => Ok(()),
        Err(e) => {
            if matches!(session.file_exists(remote_dir).await, Ok(true)) {
                Ok(())
            } else {
                Err(format!("Failed creating remote folder {remote_dir}: {e}"))
            }
        }
    }
}

async fn upload_entry_to_phone(
    session: &Session,
    local_entry: &Path,
    remote_target: &str,
    is_dir: bool,
) -> Result<(), String> {
    if !is_dir {
        return upload_local_file_to_phone(session, local_entry, remote_target).await;
    }

    ensure_remote_dir_single(session, remote_target).await?;
    let mut stack = vec![(local_entry.to_path_buf(), remote_target.to_owned())];
    while let Some((cur_local, cur_remote)) = stack.pop() {
        let mut rd = tokio::fs::read_dir(&cur_local)
            .await
            .map_err(|e| format!("Failed reading {}: {e}", cur_local.display()))?;
        while let Some(child) = rd
            .next_entry()
            .await
            .map_err(|e| format!("Failed enumerating {}: {e}", cur_local.display()))?
        {
            let child_path = child.path();
            let meta = tokio::fs::symlink_metadata(&child_path)
                .await
                .map_err(|e| format!("Failed reading metadata for {}: {e}", child_path.display()))?;
            if meta.file_type().is_symlink() {
                continue;
            }
            let child_name_raw = child.file_name().to_string_lossy().into_owned();
            let child_name = sanitize_child_name(&child_name_raw)?;
            let child_remote = format!("{}/{}", cur_remote.trim_end_matches('/'), child_name);
            if meta.is_dir() {
                ensure_remote_dir_single(session, &child_remote).await?;
                stack.push((child_path, child_remote));
            } else {
                upload_local_file_to_phone(session, &child_path, &child_remote).await?;
            }
        }
    }
    Ok(())
}

async fn upload_local_file_to_phone(
    session: &Session,
    local_path: &Path,
    remote_path: &str,
) -> Result<(), String> {
    let meta = tokio::fs::metadata(local_path)
        .await
        .map_err(|e| format!("Failed reading {}: {e}", local_path.display()))?;
    let size = meta.len();
    let mtime_secs = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut file = tokio::fs::File::open(local_path)
        .await
        .map_err(|e| format!("Failed opening {}: {e}", local_path.display()))?;
    session
        .upload_from_reader(&mut file, size, remote_path, mtime_secs, |_, _| {})
        .await
        .map_err(|e| format!("Failed restoring {remote_path}: {e}"))?;
    Ok(())
}

async fn download_remote_file(
    session: &Session,
    remote_path: &str,
    local_path: &Path,
) -> Result<(), String> {
    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed creating parent dir {}: {e}", parent.display()))?;
    }
    let mut file = tokio::fs::File::create(local_path)
        .await
        .map_err(|e| format!("Failed creating trash file {}: {e}", local_path.display()))?;
    if let Err(e) = session
        .download_to_writer(remote_path, 0, 0, &mut file, |_, _| {})
        .await
    {
        drop(file);
        let _ = tokio::fs::remove_file(local_path).await;
        return Err(format!("Failed copying {remote_path} to Linux trash: {e}"));
    }
    file.flush()
        .await
        .map_err(|e| format!("Failed flushing trash file {}: {e}", local_path.display()))?;
    file.sync_all()
        .await
        .map_err(|e| format!("Failed syncing trash file {}: {e}", local_path.display()))?;
    Ok(())
}

async fn compute_dir_size(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let p = entry.path();
            let Ok(meta) = tokio::fs::symlink_metadata(&p).await else {
                continue;
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.is_dir() {
                stack.push(p);
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

async fn remove_entry_path(path: &Path) {
    if let Ok(meta) = tokio::fs::symlink_metadata(path).await {
        if meta.is_dir() {
            let _ = tokio::fs::remove_dir_all(path).await;
        } else {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

fn sanitize_unique_entry_names(entry_names: &[String]) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut out = Vec::with_capacity(entry_names.len());
    for raw in entry_names {
        let clean = sanitize_child_name(raw)?;
        if raw.contains('/') || clean != raw.as_str() {
            return Err(format!("Unsafe trash entry name: {raw:?}"));
        }
        if seen.insert(clean.clone()) {
            out.push(clean);
        }
    }
    Ok(out)
}

fn is_safe_remote_restore_path(path: &str) -> bool {
    let trimmed = path.trim();
    if !trimmed.starts_with('/') || trimmed == "/" || trimmed.contains('\\') || trimmed.contains('\0')
    {
        return false;
    }
    !trimmed.split('/').any(|seg| seg == "." || seg == "..")
}

fn allocate_entry_name(
    files_dir: &Path,
    info_dir: &Path,
    base_name: &str,
    is_dir: bool,
) -> Result<String, String> {
    let exists = |name: &str| {
        files_dir.join(name).exists() || info_dir.join(format!("{name}{TRASHINFO_EXT}")).exists()
    };
    if !exists(base_name) {
        return Ok(base_name.to_owned());
    }

    let (stem, ext) = if !is_dir {
        match base_name.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() && !e.is_empty() => (s, format!(".{e}")),
            _ => (base_name, String::new()),
        }
    } else {
        (base_name, String::new())
    };

    let stamp = Local::now().format("%Y%m%d%H%M%S").to_string();
    for idx in 1..10_000 {
        let candidate = format!("{stem}_{stamp}_{idx}{ext}");
        if !exists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!("Too many trash name collisions for {base_name}"))
}

fn format_trashinfo(original_path: &str, deletion_date: &str, origin: &TrashDeviceOrigin) -> String {
    let mut out = format!(
        "[Trash Info]\nPath={}\nDeletionDate={deletion_date}\n",
        encode_xdg_path(original_path)
    );
    if let Some(uuid) = clean_non_empty(origin.device_uuid.as_deref()) {
        out.push_str(KEY_DEVICE_UUID);
        out.push_str(&encode_trashinfo_meta_value(&uuid));
        out.push('\n');
    }
    if let Some(name) = clean_non_empty(origin.device_name.as_deref()) {
        out.push_str(KEY_DEVICE_NAME);
        out.push_str(&encode_trashinfo_meta_value(&name));
        out.push('\n');
    }
    if let Some(model) = clean_non_empty(origin.device_model.as_deref()) {
        out.push_str(KEY_DEVICE_MODEL);
        out.push_str(&encode_trashinfo_meta_value(&model));
        out.push('\n');
    }
    out
}

fn parse_trashinfo(raw: &str) -> TrashInfoMeta {
    let mut meta = TrashInfoMeta::default();
    let mut in_header = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('[') {
            in_header = trimmed == "[Trash Info]";
            continue;
        }
        if !in_header {
            continue;
        }
        if let Some(val) = trimmed.strip_prefix("Path=") {
            meta.original_path = decode_xdg_path(val.trim());
        } else if let Some(val) = trimmed.strip_prefix("DeletionDate=") {
            let d = val.trim();
            if !d.is_empty() {
                meta.deletion_date = Some(d.to_owned());
            }
        } else if let Some(val) = trimmed.strip_prefix(KEY_DEVICE_UUID) {
            meta.device_uuid = decode_trashinfo_meta_value(val);
        } else if let Some(val) = trimmed.strip_prefix(KEY_DEVICE_NAME) {
            meta.device_name = decode_trashinfo_meta_value(val);
        } else if let Some(val) = trimmed.strip_prefix(KEY_DEVICE_MODEL) {
            meta.device_model = decode_trashinfo_meta_value(val);
        }
    }
    meta
}

fn encode_trashinfo_meta_value(raw: &str) -> String {
    let trimmed = raw.trim();
    let mut out = String::with_capacity(trimmed.len());
    for ch in trimmed.chars() {
        match ch {
            '%' => out.push_str("%25"),
            '\n' => out.push_str("%0A"),
            '\r' => out.push_str("%0D"),
            '\0' => out.push_str("%00"),
            _ => out.push(ch),
        }
    }
    out
}

fn decode_trashinfo_meta_value(raw: &str) -> Option<String> {
    decode_xdg_path(raw.trim()).and_then(|s| clean_non_empty(Some(&s)))
}

fn encode_xdg_path(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for byte in raw.as_bytes() {
        let b = *byte;
        let is_unreserved = b.is_ascii_alphanumeric()
            || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~');
        if is_unreserved {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(char::from_digit((b >> 4) as u32, 16).unwrap().to_ascii_uppercase());
            out.push(char::from_digit((b & 0xF) as u32, 16).unwrap().to_ascii_uppercase());
        }
    }
    out
}

fn decode_xdg_path(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hi = (bytes[i + 1] as char).to_digit(16)?;
            let lo = (bytes[i + 2] as char).to_digit(16)?;
            decoded.push(((hi << 4) | lo) as u8);
            i += 3;
        } else {
            decoded.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(decoded).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trashinfo_xdg_path_roundtrip_handles_spaces_unicode_and_slashes() {
        let original = "/storage/emulated/0/DCIM/Camera/照片 1#a%.jpg";
        let encoded = encode_xdg_path(original);
        assert_eq!(
            encoded,
            "/storage/emulated/0/DCIM/Camera/%E7%85%A7%E7%89%87%201%23a%25.jpg"
        );
        assert_eq!(decode_xdg_path(&encoded).as_deref(), Some(original));

        let origin = TrashDeviceOrigin {
            device_uuid: Some("uuid-vivo-1".to_owned()),
            device_name: Some("小明的 vivo 100%\nPhone".to_owned()),
            device_model: Some("vivo V2419A".to_owned()),
        };
        let info = format_trashinfo(
            "/storage/emulated/0/foo bar.txt",
            "2026-11-15T13:40:00",
            &origin,
        );
        assert_eq!(
            info,
            "[Trash Info]\nPath=/storage/emulated/0/foo%20bar.txt\nDeletionDate=2026-11-15T13:40:00\nX-HandShaker-Device-UUID=uuid-vivo-1\nX-HandShaker-Device-Name=小明的 vivo 100%25%0APhone\nX-HandShaker-Device-Model=vivo V2419A\n"
        );
        let parsed = parse_trashinfo(&info);
        assert_eq!(
            parsed.original_path.as_deref(),
            Some("/storage/emulated/0/foo bar.txt")
        );
        assert_eq!(parsed.deletion_date.as_deref(), Some("2026-11-15T13:40:00"));
        assert_eq!(parsed.device_uuid.as_deref(), Some("uuid-vivo-1"));
        assert_eq!(
            parsed.device_name.as_deref(),
            Some("小明的 vivo 100%\nPhone")
        );
        assert_eq!(parsed.device_model.as_deref(), Some("vivo V2419A"));
    }

    #[test]
    fn allocate_entry_name_avoids_collisions_in_files_and_info() {
        let dir = tempfile::tempdir().unwrap();
        let files_dir = dir.path().join("files");
        let info_dir = dir.path().join("info");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::create_dir_all(&info_dir).unwrap();

        assert_eq!(
            allocate_entry_name(&files_dir, &info_dir, "photo.jpg", false).unwrap(),
            "photo.jpg"
        );
        std::fs::write(files_dir.join("photo.jpg"), b"x").unwrap();
        let next = allocate_entry_name(&files_dir, &info_dir, "photo.jpg", false).unwrap();
        assert!(next.starts_with("photo_"));
        assert!(next.ends_with(".jpg"));

        // Collision when only .trashinfo exists should also be avoided
        std::fs::write(info_dir.join("album.trashinfo"), b"[Trash Info]\n").unwrap();
        let album_next = allocate_entry_name(&files_dir, &info_dir, "album", true).unwrap();
        assert!(album_next.starts_with("album_"));
        assert_ne!(album_next, "album");
    }

    #[test]
    fn resolve_trash_root_expands_tilde_and_rejects_unsafe_paths() {
        let resolved = resolve_trash_root("~/.HandShakerTrash").unwrap();
        assert!(resolved.is_absolute());
        assert!(resolved.ends_with(".HandShakerTrash"));

        assert!(resolve_trash_root("../escape").is_err());
        assert!(resolve_trash_root("~").is_err());
        assert!(resolve_trash_root("/").is_err());
    }

    #[tokio::test]
    async fn list_delete_and_empty_trash_manage_files_dirs_and_sidecars() {
        let root = tempfile::tempdir().unwrap();
        let root_str = root.path().to_string_lossy().into_owned();
        let files_dir = root.path().join("files");
        let info_dir = root.path().join("info");
        std::fs::create_dir_all(files_dir.join("Album")).unwrap();
        std::fs::create_dir_all(&info_dir).unwrap();

        let vivo_origin = TrashDeviceOrigin {
            device_uuid: Some("uuid-vivo".to_owned()),
            device_name: Some("vivo X200 Pro mini".to_owned()),
            device_model: Some("vivo V2419A".to_owned()),
        };

        std::fs::write(files_dir.join("note.txt"), b"hello").unwrap();
        std::fs::write(
            info_dir.join("note.txt.trashinfo"),
            format_trashinfo(
                "/storage/emulated/0/Download/note.txt",
                "2026-11-15T10:00:00",
                &vivo_origin,
            ),
        )
        .unwrap();

        std::fs::write(files_dir.join("Album/a.bin"), vec![1u8; 42]).unwrap();
        std::fs::write(
            info_dir.join("Album.trashinfo"),
            format_trashinfo(
                "/storage/emulated/0/Pictures/Album",
                "2026-11-15T11:00:00",
                &vivo_origin,
            ),
        )
        .unwrap();

        let items = list_trash_items(&root_str).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].entry_name, "Album");
        assert!(items[0].is_directory);
        assert_eq!(items[0].size_bytes, 42);
        assert_eq!(
            items[0].original_path.as_deref(),
            Some("/storage/emulated/0/Pictures/Album")
        );
        assert_eq!(items[0].device_uuid.as_deref(), Some("uuid-vivo"));
        assert_eq!(items[0].device_name.as_deref(), Some("vivo X200 Pro mini"));
        assert_eq!(items[0].device_model.as_deref(), Some("vivo V2419A"));
        assert_eq!(items[1].entry_name, "note.txt");
        assert_eq!(items[1].size_bytes, 5);

        assert!(delete_trash_items(&root_str, &["../etc/passwd".to_owned()])
            .await
            .is_err());

        let deleted = delete_trash_items(&root_str, &["note.txt".to_owned()])
            .await
            .unwrap();
        assert_eq!(deleted, 1);
        assert!(!files_dir.join("note.txt").exists());
        assert!(!info_dir.join("note.txt.trashinfo").exists());

        let cleared = empty_trash(&root_str).await.unwrap();
        assert_eq!(cleared, 1);
        assert!(list_trash_items(&root_str).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn restore_preflight_requires_matching_origin_phone() {
        let root = tempfile::tempdir().unwrap();
        let files_dir = root.path().join("files");
        let info_dir = root.path().join("info");
        std::fs::create_dir_all(&files_dir).unwrap();
        std::fs::create_dir_all(&info_dir).unwrap();

        let vivo_origin = TrashDeviceOrigin {
            device_uuid: Some("uuid-vivo".to_owned()),
            device_name: Some("vivo X200 Pro mini".to_owned()),
            device_model: Some("vivo V2419A".to_owned()),
        };
        let oneplus_origin = TrashDeviceOrigin {
            device_uuid: Some("uuid-oneplus".to_owned()),
            device_name: Some("OnePlus 7Pro".to_owned()),
            device_model: Some("OnePlus GM1917".to_owned()),
        };

        std::fs::write(files_dir.join("from_vivo.jpg"), b"img").unwrap();
        std::fs::write(
            info_dir.join("from_vivo.jpg.trashinfo"),
            format_trashinfo(
                "/storage/emulated/0/DCIM/from_vivo.jpg",
                "2026-11-15T10:00:00",
                &vivo_origin,
            ),
        )
        .unwrap();

        std::fs::write(files_dir.join("from_oneplus.jpg"), b"img2").unwrap();
        std::fs::write(
            info_dir.join("from_oneplus.jpg.trashinfo"),
            format_trashinfo(
                "/storage/emulated/0/DCIM/from_oneplus.jpg",
                "2026-11-15T10:01:00",
                &oneplus_origin,
            ),
        )
        .unwrap();

        std::fs::write(files_dir.join("legacy.txt"), b"old").unwrap();
        std::fs::write(
            info_dir.join("legacy.txt.trashinfo"),
            format_trashinfo(
                "/storage/emulated/0/Download/legacy.txt",
                "2026-11-15T09:00:00",
                &TrashDeviceOrigin::default(),
            ),
        )
        .unwrap();

        // Matching device passes preflight cleanly
        let ok_plan = prepare_restore_plan(
            root.path(),
            &["from_vivo.jpg".to_owned()],
            &vivo_origin,
            None,
        )
        .await
        .unwrap();
        assert_eq!(ok_plan.len(), 1);
        assert_eq!(ok_plan[0].target_remote, "/storage/emulated/0/DCIM/from_vivo.jpg");

        // Different phone with same model/name or different UUID is rejected with clear message
        let err = prepare_restore_plan(
            root.path(),
            &["from_oneplus.jpg".to_owned()],
            &vivo_origin,
            None,
        )
        .await
        .err()
        .unwrap();
        assert!(err.contains("Device mismatch"));
        assert!(err.contains("OnePlus 7Pro (OnePlus GM1917)"));

        // Mixed batch fails preflight atomically before restoring any file
        let mixed_err = prepare_restore_plan(
            root.path(),
            &["from_vivo.jpg".to_owned(), "from_oneplus.jpg".to_owned()],
            &vivo_origin,
            None,
        )
        .await
        .err()
        .unwrap();
        assert!(mixed_err.contains("Device mismatch"));

        // Missing device origin metadata cannot be verified as original phone
        let missing_err = prepare_restore_plan(
            root.path(),
            &["legacy.txt".to_owned()],
            &vivo_origin,
            None,
        )
        .await
        .err()
        .unwrap();
        assert!(missing_err.contains("Missing source phone metadata"));

        // Unknown current device identity also refuses restore
        let no_cur_err = prepare_restore_plan(
            root.path(),
            &["from_vivo.jpg".to_owned()],
            &TrashDeviceOrigin::default(),
            None,
        )
        .await
        .err()
        .unwrap();
        assert!(no_cur_err.contains("Cannot verify currently connected phone"));
    }
}
