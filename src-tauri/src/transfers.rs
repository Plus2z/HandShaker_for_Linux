use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use handshaker_proto::{ProtoError, Session};
use serde::Deserialize;
use tokio::sync::{mpsc, watch, RwLock, Semaphore};
use uuid::Uuid;

use crate::dto::TransferItemDto;

pub type EmitTransfersFn = Arc<dyn Fn(Vec<TransferItemDto>) + Send + Sync>;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadTarget {
    pub remote_path: String,
    pub is_directory: bool,
    #[serde(default)]
    pub file_size: u64,
}

#[derive(Clone, Debug)]
struct JobSpec {
    id: String,
    direction: TransferDir,
    local_path: PathBuf,
    remote_path: String,
    total_bytes: u64,
    cancel_rx: watch::Receiver<bool>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TransferDir {
    Download,
    Upload,
}

const MAX_EXPANDED_FILES: usize = 10_000;

/// Extracts and validates a single safe filename/folder segment from a remote path or filename.
pub fn sanitize_child_name(raw_path_or_name: &str) -> Result<String, String> {
    if raw_path_or_name.contains('\0') {
        return Err(format!("Unsafe path segment in {raw_path_or_name:?}"));
    }
    let trimmed = raw_path_or_name.trim_end_matches('/');
    let segment = trimmed.rsplit('/').next().unwrap_or("");
    if segment.is_empty()
        || segment == "."
        || segment == ".."
        || segment.contains('/')
        || segment.contains('\\')
        || segment.contains('\0')
    {
        return Err(format!("Unsafe path segment in {raw_path_or_name:?}"));
    }
    let mut comps = Path::new(segment).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(segment.to_owned()),
        _ => Err(format!("Unsafe path segment in {raw_path_or_name:?}")),
    }
}

pub struct TransferManager {
    items: Arc<Mutex<Vec<TransferItemDto>>>,
    cancels: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    job_tx: mpsc::UnboundedSender<JobSpec>,
    emit: EmitTransfersFn,
}

impl TransferManager {
    pub fn new(session_slot: Arc<RwLock<Option<Session>>>, emit: EmitTransfersFn) -> Self {
        let items: Arc<Mutex<Vec<TransferItemDto>>> = Arc::new(Mutex::new(Vec::new()));
        let cancels: Arc<Mutex<HashMap<String, watch::Sender<bool>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (job_tx, mut job_rx) = mpsc::unbounded_channel::<JobSpec>();

        let items_worker = Arc::clone(&items);
        let cancels_worker = Arc::clone(&cancels);
        let emit_worker = Arc::clone(&emit);

        tokio::spawn(async move {
            let sem = Arc::new(Semaphore::new(2));
            while let Some(job) = job_rx.recv().await {
                if *job.cancel_rx.borrow() {
                    update_item_status(&items_worker, &cancels_worker, &emit_worker, &job.id, "cancelled", None);
                    continue;
                }
                let Ok(permit) = sem.clone().acquire_owned().await else {
                    break;
                };
                let sess_slot = Arc::clone(&session_slot);
                let items_c = Arc::clone(&items_worker);
                let cancels_c = Arc::clone(&cancels_worker);
                let emit_c = Arc::clone(&emit_worker);
                tokio::spawn(async move {
                    let _permit = permit;
                    execute_job(job, sess_slot, items_c, cancels_c, emit_c).await;
                });
            }
        });

        Self {
            items,
            cancels,
            job_tx,
            emit,
        }
    }

    pub fn list(&self) -> Vec<TransferItemDto> {
        self.items.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub async fn enqueue_downloads(
        &self,
        session: &Session,
        targets: Vec<DownloadTarget>,
        local_dir: &Path,
    ) -> Result<usize, String> {
        let mut seen_targets = HashSet::new();
        let unique_targets: Vec<DownloadTarget> = targets
            .into_iter()
            .filter(|t| seen_targets.insert(t.remote_path.clone()))
            .collect();
        let expanded = expand_remote_downloads(session, &unique_targets, local_dir).await?;
        let mut count = 0;
        for (remote_path, local_path, size) in expanded {
            let file_name = sanitize_child_name(&remote_path)?;
            if self.push_job(TransferDir::Download, file_name, local_path, remote_path, size) {
                count += 1;
            }
        }
        if count > 0 {
            self.emit_snapshot();
        }
        Ok(count)
    }

    pub async fn enqueue_uploads(
        &self,
        session: &Session,
        local_paths: Vec<PathBuf>,
        remote_dir: &str,
    ) -> Result<usize, String> {
        let mut seen_paths = HashSet::new();
        let unique_paths: Vec<PathBuf> = local_paths
            .into_iter()
            .filter(|lp| seen_paths.insert(lp.clone()))
            .collect();
        let mut expanded = Vec::new();
        let clean_remote_base = remote_dir.trim_end_matches('/');
        if !unique_paths.is_empty() {
            ensure_remote_dir(session, clean_remote_base).await?;
        }
        for lp in unique_paths {
            expand_local_upload(Some(session), &lp, clean_remote_base, &mut expanded).await?;
        }
        let mut count = 0;
        for (local_path, remote_path, size) in expanded {
            let file_name = local_path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".to_owned());
            if self.push_job(TransferDir::Upload, file_name, local_path, remote_path, size) {
                count += 1;
            }
        }
        if count > 0 {
            self.emit_snapshot();
        }
        Ok(count)
    }

    pub fn cancel(&self, id: &str) -> bool {
        let sender = {
            let guard = self.cancels.lock().unwrap_or_else(|e| e.into_inner());
            guard.get(id).cloned()
        };
        if let Some(tx) = sender {
            let _ = tx.send(true);
            update_item_status(&self.items, &self.cancels, &self.emit, id, "cancelled", None);
            return true;
        }
        false
    }

    pub fn clear_finished(&self) {
        {
            let mut guard = self.items.lock().unwrap_or_else(|e| e.into_inner());
            guard.retain(|it| matches!(it.status.as_str(), "queued" | "active"));
        }
        self.emit_snapshot();
    }

    fn push_job(
        &self,
        direction: TransferDir,
        file_name: String,
        local_path: PathBuf,
        remote_path: String,
        total_bytes: u64,
    ) -> bool {
        let dir_label = match direction {
            TransferDir::Download => "download",
            TransferDir::Upload => "upload",
        };
        let local_str = local_path.to_string_lossy().into_owned();
        let id = Uuid::new_v4().to_string();
        let (cancel_tx, cancel_rx) = watch::channel(false);

        {
            let mut items = self.items.lock().unwrap_or_else(|e| e.into_inner());
            let already_active = items.iter().any(|it| {
                it.direction == dir_label
                    && it.local_path == local_str
                    && it.remote_path == remote_path
                    && matches!(it.status.as_str(), "queued" | "active")
            });
            if already_active {
                return false;
            }
            items.push(TransferItemDto {
                id: id.clone(),
                direction: dir_label.to_owned(),
                file_name,
                local_path: local_str,
                remote_path: remote_path.clone(),
                total_bytes,
                transferred_bytes: 0,
                status: "queued".to_owned(),
                error: None,
                created_at_ms: now_millis(),
            });
        }

        self.cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id.clone(), cancel_tx);

        let _ = self.job_tx.send(JobSpec {
            id,
            direction,
            local_path,
            remote_path,
            total_bytes,
            cancel_rx,
        });
        true
    }

    fn emit_snapshot(&self) {
        (self.emit)(self.list());
    }
}

async fn execute_job(
    job: JobSpec,
    session_slot: Arc<RwLock<Option<Session>>>,
    items: Arc<Mutex<Vec<TransferItemDto>>>,
    cancels: Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    emit: EmitTransfersFn,
) {
    if *job.cancel_rx.borrow() {
        update_item_status(&items, &cancels, &emit, &job.id, "cancelled", None);
        return;
    }

    let session = { session_slot.read().await.clone() };
    let Some(session) = session else {
        update_item_status(&items, &cancels, &emit, &job.id, "failed", Some("Not connected".into()));
        return;
    };

    update_item_status(&items, &cancels, &emit, &job.id, "active", None);
    let last_emit = Arc::new(Mutex::new(Instant::now() - duration_ms(500)));

    let res = match job.direction {
        TransferDir::Download => {
            if let Some(parent) = job.local_path.parent() {
                if let Err(e) = tokio::fs::create_dir_all(parent).await {
                    update_item_status(
                        &items,
                        &cancels,
                        &emit,
                        &job.id,
                        "failed",
                        Some(e.to_string()),
                    );
                    return;
                }
            }
            match tokio::fs::File::create(&job.local_path).await {
                Err(e) => Err(ProtoError::Io(e)),
                Ok(mut f) => {
                    let items_p = Arc::clone(&items);
                    let emit_p = Arc::clone(&emit);
                    let id_p = job.id.clone();
                    let last_p = Arc::clone(&last_emit);
                    session
                        .download_to_writer_cancellable(
                            &job.remote_path,
                            0,
                            0,
                            &mut f,
                            Some(job.cancel_rx.clone()),
                            move |done, total| {
                                update_progress(&items_p, &emit_p, &last_p, &id_p, done, total);
                            },
                        )
                        .await
                        .map(|hdr| hdr.range.and_then(|r| r.length).unwrap_or(job.total_bytes))
                }
            }
        }
        TransferDir::Upload => {
            let meta_res = tokio::fs::metadata(&job.local_path).await;
            match meta_res {
                Err(e) => Err(ProtoError::Io(e)),
                Ok(meta) => {
                    let size = meta.len();
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    match tokio::fs::File::open(&job.local_path).await {
                        Err(e) => Err(ProtoError::Io(e)),
                        Ok(mut f) => {
                            let items_p = Arc::clone(&items);
                            let emit_p = Arc::clone(&emit);
                            let id_p = job.id.clone();
                            let last_p = Arc::clone(&last_emit);
                            session
                                .upload_from_reader_cancellable(
                                    &mut f,
                                    size,
                                    &job.remote_path,
                                    mtime,
                                    Some(job.cancel_rx.clone()),
                                    move |done, total| {
                                        update_progress(&items_p, &emit_p, &last_p, &id_p, done, total);
                                    },
                                )
                                .await
                                .map(|_| size)
                        }
                    }
                }
            }
        }
    };

    match res {
        Ok(final_bytes) => {
            {
                let mut g = items.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(it) = g.iter_mut().find(|i| i.id == job.id) {
                    it.total_bytes = it.total_bytes.max(final_bytes);
                    it.transferred_bytes = it.total_bytes;
                }
            }
            update_item_status(&items, &cancels, &emit, &job.id, "completed", None);
        }
        Err(ProtoError::Cancelled) => {
            if job.direction == TransferDir::Download {
                let _ = tokio::fs::remove_file(&job.local_path).await;
            }
            update_item_status(&items, &cancels, &emit, &job.id, "cancelled", None);
        }
        Err(e) => {
            update_item_status(&items, &cancels, &emit, &job.id, "failed", Some(e.to_string()));
        }
    }
}

fn duration_ms(ms: u64) -> std::time::Duration {
    std::time::Duration::from_millis(ms)
}

fn update_progress(
    items: &Arc<Mutex<Vec<TransferItemDto>>>,
    emit: &EmitTransfersFn,
    last_emit: &Arc<Mutex<Instant>>,
    id: &str,
    done: u64,
    total: u64,
) {
    let snapshot_opt = {
        let mut g = items.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(it) = g.iter_mut().find(|i| i.id == id) {
            it.transferred_bytes = done;
            if total > 0 {
                it.total_bytes = total;
            }
        }
        let mut last = last_emit.lock().unwrap_or_else(|e| e.into_inner());
        if done >= total || last.elapsed() >= duration_ms(120) {
            *last = Instant::now();
            Some(g.clone())
        } else {
            None
        }
    };
    if let Some(snap) = snapshot_opt {
        emit(snap);
    }
}

fn update_item_status(
    items: &Arc<Mutex<Vec<TransferItemDto>>>,
    cancels: &Arc<Mutex<HashMap<String, watch::Sender<bool>>>>,
    emit: &EmitTransfersFn,
    id: &str,
    status: &str,
    error: Option<String>,
) {
    if matches!(status, "completed" | "failed" | "cancelled") {
        cancels
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(id);
    }
    let snap = {
        let mut g = items.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(it) = g.iter_mut().find(|i| i.id == id) {
            // Preserve user cancellation if async completion raced with cancel().
            if !(it.status == "cancelled" && status == "failed") {
                it.status = status.to_owned();
                it.error = error;
            }
        }
        g.clone()
    };
    emit(snap);
}

pub(crate) async fn expand_remote_downloads(
    session: &Session,
    targets: &[DownloadTarget],
    local_root: &Path,
) -> Result<Vec<(String, PathBuf, u64)>, String> {
    let mut files = Vec::new();
    let mut visited_dirs = HashSet::new();
    let mut stack: Vec<(String, PathBuf, bool, u64)> = Vec::with_capacity(targets.len());

    for t in targets {
        let name = sanitize_child_name(&t.remote_path)?;
        let target_local = local_root.join(&name);
        if !target_local.starts_with(local_root) || target_local == local_root {
            return Err(format!("Unsafe target path resolved outside download root: {}", t.remote_path));
        }
        stack.push((t.remote_path.clone(), target_local, t.is_directory, t.file_size));
    }

    while let Some((remote, local, is_dir, size)) = stack.pop() {
        if !is_dir {
            if files.len() >= MAX_EXPANDED_FILES {
                return Err(format!("Download exceeds maximum of {MAX_EXPANDED_FILES} files"));
            }
            files.push((remote, local, size));
            continue;
        }
        if !visited_dirs.insert(remote.clone()) {
            continue;
        }
        tokio::fs::create_dir_all(&local)
            .await
            .map_err(|e| format!("Failed creating local dir {}: {e}", local.display()))?;
        let resp = session
            .list_dir(&remote, 1)
            .await
            .map_err(|e| format!("Failed listing {remote}: {e}"))?;
        for entry in resp.file {
            let Some(child_remote) = entry.path else { continue };
            let child_name = sanitize_child_name(&child_remote)?;
            let child_local = local.join(&child_name);
            if !child_local.starts_with(local_root) || child_local == local_root {
                return Err(format!(
                    "Unsafe remote child path resolved outside download root: {child_remote}"
                ));
            }
            if files.len() + stack.len() >= MAX_EXPANDED_FILES {
                return Err(format!("Download exceeds maximum of {MAX_EXPANDED_FILES} files"));
            }
            stack.push((
                child_remote,
                child_local,
                entry.is_directory.unwrap_or(false),
                entry.file_size.unwrap_or(0),
            ));
        }
    }
    files.reverse();
    Ok(files)
}

async fn ensure_remote_dir(sess: &Session, remote_dir: &str) -> Result<(), String> {
    let clean = remote_dir.trim_end_matches('/');
    if clean.is_empty() {
        return Ok(());
    }
    match sess.file_exists(clean).await {
        Ok(true) => return Ok(()),
        Ok(false) => {}
        Err(e) => return Err(format!("Failed checking remote folder {clean}: {e}")),
    }
    match sess.create_folder(clean).await {
        Ok(()) => Ok(()),
        Err(e) => {
            if matches!(sess.file_exists(clean).await, Ok(true)) {
                Ok(())
            } else {
                Err(format!("Failed creating remote folder {clean}: {e}"))
            }
        }
    }
}

async fn expand_local_upload(
    session: Option<&Session>,
    root_local: &Path,
    remote_parent: &str,
    out: &mut Vec<(PathBuf, String, u64)>,
) -> Result<(), String> {
    let mut stack = vec![(root_local.to_path_buf(), remote_parent.to_owned())];
    while let Some((local_path, remote_dir)) = stack.pop() {
        let meta = tokio::fs::symlink_metadata(&local_path)
            .await
            .map_err(|e| format!("Failed reading {}: {e}", local_path.display()))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        let name = local_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or_else(|| format!("Invalid upload path: {}", local_path.display()))?;
        let target_remote = format!("{}/{}", remote_dir.trim_end_matches('/'), name);

        if meta.is_dir() {
            if let Some(sess) = session {
                ensure_remote_dir(sess, &target_remote).await?;
            }
            let mut rd = tokio::fs::read_dir(&local_path)
                .await
                .map_err(|e| format!("Failed reading dir {}: {e}", local_path.display()))?;
            while let Some(child) = rd.next_entry().await.map_err(|e| e.to_string())? {
                stack.push((child.path(), target_remote.clone()));
            }
        } else {
            if out.len() >= MAX_EXPANDED_FILES {
                return Err(format!("Upload exceeds maximum of {MAX_EXPANDED_FILES} files"));
            }
            out.push((local_path, target_remote, meta.len()));
        }
    }
    Ok(())
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queue_cancel_and_clear_finished_work() {
        let session_slot = Arc::new(RwLock::new(None));
        let emitted = Arc::new(Mutex::new(Vec::<Vec<TransferItemDto>>::new()));
        let emitted_c = Arc::clone(&emitted);
        let mgr = TransferManager::new(
            session_slot,
            Arc::new(move |snap| {
                emitted_c.lock().unwrap().push(snap);
            }),
        );

        mgr.push_job(
            TransferDir::Upload,
            "notes.txt".into(),
            PathBuf::from("/tmp/missing-notes.txt"),
            "/storage/emulated/0/notes.txt".into(),
            42,
        );
        let list = mgr.list();
        assert_eq!(list.len(), 1);
        let id = list[0].id.clone();

        // Wait briefly for worker to see Not connected -> failed
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(mgr.list()[0].status, "failed");

        mgr.clear_finished();
        assert!(mgr.list().is_empty());
        assert!(!mgr.cancel(&id));
    }

    #[test]
    fn sanitize_child_name_rejects_traversal_and_special_segments() {
        for bad in [
            "..",
            "/storage/emulated/0/..",
            "/storage/emulated/0/.",
            "foo\\bar",
            "bad\0file.txt",
            "",
        ] {
            assert!(sanitize_child_name(bad).is_err(), "expected Err for {bad:?}");
        }
        assert_eq!(
            sanitize_child_name("/storage/emulated/0/DCIM/Camera/IMG_001.jpg").unwrap(),
            "IMG_001.jpg"
        );
    }

    #[tokio::test]
    async fn expand_local_upload_skips_symlinks() {
        let tmp = tempfile::TempDir::new().unwrap();
        let dir_path = tmp.path().join("folder");
        std::fs::create_dir_all(&dir_path).unwrap();
        let symlink_path = dir_path.join("loop_to_parent");
        #[cfg(unix)]
        std::os::unix::fs::symlink(tmp.path(), &symlink_path).unwrap();

        let mut out = Vec::new();
        expand_local_upload(None, &symlink_path, "/storage/emulated/0", &mut out)
            .await
            .unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn push_job_deduplicates_active_transfers_and_allows_retry_after_finish() {
        let session_slot = Arc::new(RwLock::new(None));
        let mgr = TransferManager::new(session_slot, Arc::new(|_| {}));

        assert!(mgr.push_job(
            TransferDir::Upload,
            "doc.pdf".into(),
            PathBuf::from("/tmp/nonexistent-doc.pdf"),
            "/storage/emulated/0/doc.pdf".into(),
            128,
        ));
        assert!(!mgr.push_job(
            TransferDir::Upload,
            "doc.pdf".into(),
            PathBuf::from("/tmp/nonexistent-doc.pdf"),
            "/storage/emulated/0/doc.pdf".into(),
            128,
        ));
        assert_eq!(mgr.list().len(), 1);

        // Once the first attempt finishes (fails because no session is connected), re-queueing succeeds.
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        assert_eq!(mgr.list()[0].status, "failed");

        assert!(mgr.push_job(
            TransferDir::Upload,
            "doc.pdf".into(),
            PathBuf::from("/tmp/nonexistent-doc.pdf"),
            "/storage/emulated/0/doc.pdf".into(),
            128,
        ));
        assert_eq!(mgr.list().len(), 2);
    }
}
