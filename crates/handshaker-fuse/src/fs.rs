use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{FileExt, MetadataExt};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{
    Config as FuseConfig, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags,
    Generation, INodeNo, LockOwner, MountOption, OpenFlags, RenameFlags, ReplyAttr, ReplyCreate,
    ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request,
    TimeOrNow, WriteFlags,
};
use handshaker_proto::{ProtoError, Session};
use tokio::runtime::Handle;
use tracing::{debug, warn};
use uuid::Uuid;

const ATTR_TTL: Duration = Duration::from_secs(1);
const DIR_CACHE_TTL: Duration = Duration::from_secs(2);
const BLOCK_SIZE: u32 = 4096;

// POSIX open access mode / flags (Linux asm-generic/fcntl.h)
const O_ACCMODE_MASK: i32 = 0o3;
const O_WRONLY_FLAG: i32 = 0o1;
const O_RDWR_FLAG: i32 = 0o2;
const O_TRUNC_FLAG: i32 = 0o1000;
const O_APPEND_FLAG: i32 = 0o2000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub mtime_secs: u64,
}

#[derive(Clone, Debug, Default)]
pub struct FsStats {
    pub total_bytes: u64,
    pub free_bytes: u64,
}

#[derive(Debug)]
pub enum BackendError {
    NotFound,
    AlreadyExists,
    PermissionDenied,
    Io(String),
}

impl BackendError {
    pub fn to_errno(&self) -> Errno {
        match self {
            BackendError::NotFound => Errno::ENOENT,
            BackendError::AlreadyExists => Errno::EEXIST,
            BackendError::PermissionDenied => Errno::EACCES,
            BackendError::Io(msg) => {
                warn!("FUSE backend I/O error: {msg}");
                Errno::EIO
            }
        }
    }
}

pub trait RemoteFsBackend: Send + Sync + 'static {
    fn list_dir(&self, remote_path: &str) -> Result<Vec<RemoteEntry>, BackendError>;
    fn stat_fs(&self) -> Result<FsStats, BackendError>;
    fn read_range(&self, remote_path: &str, offset: u64, length: u64)
        -> Result<Vec<u8>, BackendError>;
    fn upload_file(&self, local_path: &Path, remote_path: &str) -> Result<(), BackendError>;
    fn create_folder(&self, remote_path: &str) -> Result<(), BackendError>;
    fn rename(&self, src_path: &str, dst_path: &str) -> Result<(), BackendError>;
    fn delete(&self, remote_path: &str, is_trash: bool) -> Result<(), BackendError>;
}

pub struct SessionBackend {
    session: Session,
    rt: Handle,
}

impl SessionBackend {
    pub fn new(session: Session, rt: Handle) -> Self {
        Self { session, rt }
    }

    fn map_proto_err(err: ProtoError) -> BackendError {
        match err {
            ProtoError::RemoteIo { code, message } => {
                let lower = message.to_ascii_lowercase();
                if lower.contains("exist") {
                    BackendError::AlreadyExists
                } else if lower.contains("not found") || lower.contains("no such") || code == Some(2) {
                    BackendError::NotFound
                } else if lower.contains("permission") || lower.contains("denied") {
                    BackendError::PermissionDenied
                } else {
                    BackendError::Io(format!("remote error {code:?}: {message}"))
                }
            }
            other => BackendError::Io(other.to_string()),
        }
    }
}

impl RemoteFsBackend for SessionBackend {
    fn list_dir(&self, remote_path: &str) -> Result<Vec<RemoteEntry>, BackendError> {
        let resp = self
            .rt
            .block_on(self.session.list_dir(remote_path, 1))
            .map_err(Self::map_proto_err)?;
        let out = resp
            .file
            .into_iter()
            .filter_map(|f| {
                let path = f.path?;
                let name = path
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or("")
                    .to_owned();
                if name.is_empty() {
                    return None;
                }
                Some(RemoteEntry {
                    name,
                    path,
                    is_dir: f.is_directory.unwrap_or(false),
                    size: f.file_size.unwrap_or(0),
                    mtime_secs: normalize_timestamp_secs(
                        f.modified_timestamp
                            .or(f.created_timestamp)
                            .unwrap_or(0),
                    ),
                })
            })
            .collect();
        Ok(out)
    }

    fn stat_fs(&self) -> Result<FsStats, BackendError> {
        let info = self
            .rt
            .block_on(self.session.get_device_info())
            .map_err(Self::map_proto_err)?;
        let total = info.disk_size.unwrap_or(0);
        let used = info.used_disk_size.unwrap_or(0);
        Ok(FsStats {
            total_bytes: total,
            free_bytes: total.saturating_sub(used),
        })
    }

    fn read_range(
        &self,
        remote_path: &str,
        offset: u64,
        length: u64,
    ) -> Result<Vec<u8>, BackendError> {
        if length == 0 {
            return Ok(Vec::new());
        }
        let mut buf = Vec::with_capacity(length.min(1024 * 1024) as usize);
        self.rt
            .block_on(
                self.session
                    .download_to_writer(remote_path, offset, length, &mut buf, |_, _| {}),
            )
            .map_err(Self::map_proto_err)?;
        Ok(buf)
    }

    fn upload_file(&self, local_path: &Path, remote_path: &str) -> Result<(), BackendError> {
        self.rt
            .block_on(self.session.upload_file(local_path, remote_path, |_, _| {}))
            .map(|_| ())
            .map_err(Self::map_proto_err)
    }

    fn create_folder(&self, remote_path: &str) -> Result<(), BackendError> {
        self.rt
            .block_on(self.session.create_folder(remote_path))
            .map_err(Self::map_proto_err)
    }

    fn rename(&self, src_path: &str, dst_path: &str) -> Result<(), BackendError> {
        self.rt
            .block_on(self.session.rename_file(src_path, dst_path))
            .map_err(Self::map_proto_err)
    }

    fn delete(&self, remote_path: &str, is_trash: bool) -> Result<(), BackendError> {
        let paths = vec![remote_path.to_owned()];
        self.rt
            .block_on(self.session.delete_files(&paths, is_trash))
            .map_err(Self::map_proto_err)
    }
}

#[derive(Clone, Debug)]
struct InodeEntry {
    ino: INodeNo,
    parent: INodeNo,
    name: String,
    remote_path: String,
    is_dir: bool,
    size: u64,
    mtime_secs: u64,
}

#[derive(Clone, Debug)]
struct CachedDir {
    children: Vec<(INodeNo, String, bool)>,
    fetched_at: std::time::Instant,
}

#[derive(Default)]
struct InodeTable {
    next_ino: u64,
    by_ino: HashMap<u64, InodeEntry>,
    by_path: HashMap<String, INodeNo>,
    dir_cache: HashMap<u64, CachedDir>,
}

impl InodeTable {
    fn new(root_remote_path: &str) -> Self {
        let clean_root = normalize_remote_path(root_remote_path);
        let mut by_ino = HashMap::new();
        let mut by_path = HashMap::new();
        by_ino.insert(
            INodeNo::ROOT.0,
            InodeEntry {
                ino: INodeNo::ROOT,
                parent: INodeNo::ROOT,
                name: String::from("/"),
                remote_path: clean_root.clone(),
                is_dir: true,
                size: 0,
                mtime_secs: 0,
            },
        );
        by_path.insert(clean_root, INodeNo::ROOT);
        Self {
            next_ino: 2,
            by_ino,
            by_path,
            dir_cache: HashMap::new(),
        }
    }

    fn upsert_child(&mut self, parent: INodeNo, entry: &RemoteEntry) -> INodeNo {
        let norm_path = normalize_remote_path(&entry.path);
        if let Some(&existing_ino) = self.by_path.get(&norm_path) {
            if let Some(node) = self.by_ino.get_mut(&existing_ino.0) {
                node.parent = parent;
                node.name = entry.name.clone();
                node.is_dir = entry.is_dir;
                node.size = entry.size;
                node.mtime_secs = entry.mtime_secs;
            }
            return existing_ino;
        }
        let ino = INodeNo(self.next_ino);
        self.next_ino += 1;
        self.by_ino.insert(
            ino.0,
            InodeEntry {
                ino,
                parent,
                name: entry.name.clone(),
                remote_path: norm_path.clone(),
                is_dir: entry.is_dir,
                size: entry.size,
                mtime_secs: entry.mtime_secs,
            },
        );
        self.by_path.insert(norm_path, ino);
        ino
    }

    fn remove_subtree(&mut self, path_prefix: &str) {
        let norm = normalize_remote_path(path_prefix);
        let slash_prefix = if norm == "/" {
            "/".to_owned()
        } else {
            format!("{norm}/")
        };
        let to_remove: Vec<(u64, String)> = self
            .by_ino
            .iter()
            .filter_map(|(&ino, entry)| {
                if ino != INodeNo::ROOT.0
                    && (entry.remote_path == norm || entry.remote_path.starts_with(&slash_prefix))
                {
                    Some((ino, entry.remote_path.clone()))
                } else {
                    None
                }
            })
            .collect();
        for (ino, path) in to_remove {
            self.by_ino.remove(&ino);
            self.by_path.remove(&path);
            self.dir_cache.remove(&ino);
        }
    }

    fn relocate(&mut self, old_path: &str, new_parent: INodeNo, new_name: &str, new_path: &str) {
        let old_norm = normalize_remote_path(old_path);
        let new_norm = normalize_remote_path(new_path);
        let Some(&ino) = self.by_path.get(&old_norm) else {
            return;
        };
        let old_prefix = format!("{old_norm}/");
        let new_prefix = format!("{new_norm}/");

        let descendants: Vec<(u64, String, String)> = self
            .by_ino
            .iter()
            .filter_map(|(&child_ino, entry)| {
                if child_ino != ino.0 && entry.remote_path.starts_with(&old_prefix) {
                    let suffix = &entry.remote_path[old_prefix.len()..];
                    Some((
                        child_ino,
                        entry.remote_path.clone(),
                        format!("{new_prefix}{suffix}"),
                    ))
                } else {
                    None
                }
            })
            .collect();

        self.by_path.remove(&old_norm);
        if let Some(node) = self.by_ino.get_mut(&ino.0) {
            node.parent = new_parent;
            node.name = new_name.to_owned();
            node.remote_path = new_norm.clone();
        }
        self.by_path.insert(new_norm, ino);

        for (child_ino, old_p, new_p) in descendants {
            self.by_path.remove(&old_p);
            if let Some(child_node) = self.by_ino.get_mut(&child_ino) {
                child_node.remote_path = new_p.clone();
            }
            self.by_path.insert(new_p, INodeNo(child_ino));
        }
    }
}

struct OpenFileState {
    ino: INodeNo,
    remote_path: String,
    writable: bool,
    append: bool,
    dirty: bool,
    hydrated: bool,
    remote_size: u64,
    staging_path: PathBuf,
    staging_file: File,
}

#[derive(Clone)]
pub struct HandShakerFsConfig {
    pub root_remote_path: String,
    pub read_only: bool,
    pub use_trash: bool,
}

pub struct HandShakerFs<B: RemoteFsBackend> {
    backend: Arc<B>,
    config: HandShakerFsConfig,
    uid: u32,
    gid: u32,
    staging_dir: PathBuf,
    inodes: Mutex<InodeTable>,
    next_fh: Mutex<u64>,
    open_files: Mutex<HashMap<u64, Arc<Mutex<OpenFileState>>>>,
}

pub fn build_mount_config(read_only: bool, allow_other: bool) -> FuseConfig {
    let mut cfg = FuseConfig::default();
    let mut opts = vec![
        MountOption::FSName("handshaker".to_owned()),
        MountOption::Subtype("handshaker".to_owned()),
        MountOption::NoDev,
        MountOption::NoSuid,
        MountOption::AutoUnmount,
    ];
    if read_only {
        opts.push(MountOption::RO);
    } else {
        opts.push(MountOption::RW);
    }
    if allow_other {
        opts.push(MountOption::CUSTOM("allow_other".to_owned()));
    }
    cfg.mount_options = opts;
    cfg
}

impl<B: RemoteFsBackend> HandShakerFs<B> {
    pub fn new(backend: Arc<B>, config: HandShakerFsConfig) -> anyhow::Result<Self> {
        let (uid, gid) = current_uid_gid();
        let staging_dir = std::env::temp_dir().join(format!("handshaker-fuse-{}", Uuid::new_v4()));
        fs::create_dir_all(&staging_dir)?;
        let table = InodeTable::new(&config.root_remote_path);
        Ok(Self {
            backend,
            config,
            uid,
            gid,
            staging_dir,
            inodes: Mutex::new(table),
            next_fh: Mutex::new(1),
            open_files: Mutex::new(HashMap::new()),
        })
    }

    fn alloc_fh(&self) -> u64 {
        let mut guard = self.next_fh.lock().unwrap_or_else(|e| e.into_inner());
        let id = *guard;
        *guard += 1;
        id
    }

    fn attr_for_entry(&self, entry: &InodeEntry) -> FileAttr {
        let perm = if entry.is_dir {
            if self.config.read_only { 0o555 } else { 0o755 }
        } else if self.config.read_only {
            0o444
        } else {
            0o644
        };
        let mtime = UNIX_EPOCH + Duration::from_secs(entry.mtime_secs);
        FileAttr {
            ino: entry.ino,
            size: entry.size,
            blocks: entry.size.div_ceil(512),
            atime: mtime,
            mtime,
            ctime: mtime,
            crtime: mtime,
            kind: if entry.is_dir {
                FileType::Directory
            } else {
                FileType::RegularFile
            },
            perm,
            nlink: if entry.is_dir { 2 } else { 1 },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: BLOCK_SIZE,
            flags: 0,
        }
    }

    fn node_snapshot(&self, ino: INodeNo) -> Option<InodeEntry> {
        self.inodes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .by_ino
            .get(&ino.0)
            .cloned()
    }

    fn refresh_dir_locked(&self, dir_ino: INodeNo) -> Result<Vec<(INodeNo, String, bool)>, Errno> {
        let (remote_path, cached) = {
            let guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
            let Some(node) = guard.by_ino.get(&dir_ino.0) else {
                return Err(Errno::ENOENT);
            };
            if !node.is_dir {
                return Err(Errno::ENOTDIR);
            }
            if let Some(hit) = guard.dir_cache.get(&dir_ino.0) {
                if hit.fetched_at.elapsed() < DIR_CACHE_TTL {
                    return Ok(hit.children.clone());
                }
            }
            (node.remote_path.clone(), None::<Vec<(INodeNo, String, bool)>>)
        };
        let _ = cached;

        let entries = self.backend.list_dir(&remote_path).map_err(|e| e.to_errno())?;
        let mut guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
        let mut children = Vec::with_capacity(entries.len());
        for entry in entries {
            let child_ino = guard.upsert_child(dir_ino, &entry);
            children.push((child_ino, entry.name, entry.is_dir));
        }
        guard.dir_cache.insert(
            dir_ino.0,
            CachedDir {
                children: children.clone(),
                fetched_at: std::time::Instant::now(),
            },
        );
        Ok(children)
    }

    fn hydrate_staging_if_needed(&self, st: &mut OpenFileState) -> Result<(), Errno> {
        if st.hydrated {
            return Ok(());
        }
        if st.remote_size > 0 {
            let data = self
                .backend
                .read_range(&st.remote_path, 0, st.remote_size)
                .map_err(|e| e.to_errno())?;
            st.staging_file.set_len(0).map_err(|_| Errno::EIO)?;
            st.staging_file.write_all_at(&data, 0).map_err(|_| Errno::EIO)?;
            st.staging_file.flush().map_err(|_| Errno::EIO)?;
        }
        st.hydrated = true;
        Ok(())
    }

    fn flush_open_state(&self, st: &mut OpenFileState) -> Result<(), Errno> {
        if !st.dirty {
            return Ok(());
        }
        st.staging_file.sync_data().map_err(|_| Errno::EIO)?;
        self.backend
            .upload_file(&st.staging_path, &st.remote_path)
            .map_err(|e| e.to_errno())?;
        let final_len = st.staging_file.metadata().map(|m| m.len()).unwrap_or(st.remote_size);
        st.remote_size = final_len;
        st.dirty = false;

        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
        let parent_ino = guard.by_ino.get(&st.ino.0).map(|n| n.parent);
        if let Some(node) = guard.by_ino.get_mut(&st.ino.0) {
            node.size = final_len;
            node.mtime_secs = now_secs;
        }
        if let Some(p) = parent_ino {
            guard.dir_cache.remove(&p.0);
        }
        Ok(())
    }

    fn open_staging_handle(&self, node: &InodeEntry, flags: i32, force_new_empty: bool) -> Result<u64, Errno> {
        let acc = flags & O_ACCMODE_MASK;
        let writable = acc == O_WRONLY_FLAG || acc == O_RDWR_FLAG || force_new_empty;
        if writable && self.config.read_only {
            return Err(Errno::EROFS);
        }

        let fh = self.alloc_fh();
        let staging_path = self.staging_dir.join(format!("fh-{fh}.part"));
        let staging_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&staging_path)
            .map_err(|_| Errno::EIO)?;

        let truncate = force_new_empty || (flags & O_TRUNC_FLAG) != 0;
        let hydrated = truncate || node.size == 0;
        let state = OpenFileState {
            ino: node.ino,
            remote_path: node.remote_path.clone(),
            writable,
            append: (flags & O_APPEND_FLAG) != 0,
            dirty: truncate && writable && force_new_empty,
            hydrated,
            remote_size: if truncate { 0 } else { node.size },
            staging_path,
            staging_file,
        };
        if truncate && writable && !force_new_empty && node.size > 0 {
            let mut st = state;
            st.dirty = true;
            self.open_files
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(fh, Arc::new(Mutex::new(st)));
        } else {
            self.open_files
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(fh, Arc::new(Mutex::new(state)));
        }
        Ok(fh)
    }
}

impl<B: RemoteFsBackend> Drop for HandShakerFs<B> {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.staging_dir);
    }
}

impl<B: RemoteFsBackend> Filesystem for HandShakerFs<B> {
    fn lookup(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(name_str) = name.to_str().filter(|s| is_valid_segment(s)) else {
            reply.error(Errno::ENOENT);
            return;
        };

        let Ok(children) = self.refresh_dir_locked(parent) else {
            reply.error(Errno::ENOENT);
            return;
        };
        let Some((child_ino, _, _)) = children.into_iter().find(|(_, n, _)| n == name_str) else {
            reply.error(Errno::ENOENT);
            return;
        };
        let Some(entry) = self.node_snapshot(child_ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        reply.entry(&ATTR_TTL, &self.attr_for_entry(&entry), Generation(0));
    }

    fn getattr(&self, _req: &Request, ino: INodeNo, fh: Option<FileHandle>, reply: ReplyAttr) {
        let Some(mut entry) = self.node_snapshot(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if let Some(handle) = fh {
            let open_arc = self
                .open_files
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&handle.0)
                .cloned();
            if let Some(arc) = open_arc {
                let st = arc.lock().unwrap_or_else(|e| e.into_inner());
                if st.dirty {
                    if let Ok(meta) = st.staging_file.metadata() {
                        entry.size = meta.len();
                    }
                }
            }
        }
        reply.attr(&ATTR_TTL, &self.attr_for_entry(&entry));
    }

    fn setattr(
        &self,
        _req: &Request,
        ino: INodeNo,
        _mode: Option<u32>,
        _uid: Option<u32>,
        _gid: Option<u32>,
        size: Option<u64>,
        _atime: Option<TimeOrNow>,
        _mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<FileHandle>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        _flags: Option<fuser::BsdFileFlags>,
        reply: ReplyAttr,
    ) {
        let Some(mut entry) = self.node_snapshot(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if self.config.read_only && size.is_some() {
            reply.error(Errno::EROFS);
            return;
        }
        if let Some(new_size) = size {
            if entry.is_dir {
                reply.error(Errno::EISDIR);
                return;
            }
            let open_arc = fh.and_then(|h| {
                self.open_files
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&h.0)
                    .cloned()
            });
            if let Some(arc) = open_arc {
                let mut st = arc.lock().unwrap_or_else(|e| e.into_inner());
                if new_size > 0 && self.hydrate_staging_if_needed(&mut st).is_err() {
                    reply.error(Errno::EIO);
                    return;
                }
                if st.staging_file.set_len(new_size).is_err() {
                    reply.error(Errno::EIO);
                    return;
                }
                st.hydrated = true;
                st.dirty = true;
                st.remote_size = new_size;
                entry.size = new_size;
            } else {
                // Direct truncate(2) without open handle: stage and upload immediately.
                let tmp = self.staging_dir.join(format!("trunc-{}.part", Uuid::new_v4()));
                let res = (|| -> Result<(), Errno> {
                    let mut f = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&tmp)
                        .map_err(|_| Errno::EIO)?;
                    if new_size > 0 && entry.size > 0 {
                        let copy_len = entry.size.min(new_size);
                        let bytes = self
                            .backend
                            .read_range(&entry.remote_path, 0, copy_len)
                            .map_err(|e| e.to_errno())?;
                        f.write_all(&bytes).map_err(|_| Errno::EIO)?;
                    }
                    f.set_len(new_size).map_err(|_| Errno::EIO)?;
                    f.sync_data().map_err(|_| Errno::EIO)?;
                    drop(f);
                    self.backend
                        .upload_file(&tmp, &entry.remote_path)
                        .map_err(|e| e.to_errno())
                })();
                let _ = fs::remove_file(&tmp);
                if let Err(err) = res {
                    reply.error(err);
                    return;
                }
                entry.size = new_size;
                let mut guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(node) = guard.by_ino.get_mut(&ino.0) {
                    node.size = new_size;
                }
                guard.dir_cache.remove(&entry.parent.0);
            }
        }
        reply.attr(&ATTR_TTL, &self.attr_for_entry(&entry));
    }

    fn readdir(
        &self,
        _req: &Request,
        ino: INodeNo,
        _fh: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let Some(node) = self.node_snapshot(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !node.is_dir {
            reply.error(Errno::ENOTDIR);
            return;
        }
        let Ok(children) = self.refresh_dir_locked(ino) else {
            reply.error(Errno::EIO);
            return;
        };

        let mut all_items: Vec<(INodeNo, FileType, String)> = Vec::with_capacity(children.len() + 2);
        all_items.push((ino, FileType::Directory, ".".to_owned()));
        all_items.push((node.parent, FileType::Directory, "..".to_owned()));
        for (c_ino, c_name, is_dir) in children {
            all_items.push((
                c_ino,
                if is_dir { FileType::Directory } else { FileType::RegularFile },
                c_name,
            ));
        }

        for (idx, (c_ino, kind, name)) in all_items.into_iter().enumerate().skip(offset as usize) {
            let next_offset = (idx + 1) as u64;
            if reply.add(c_ino, next_offset, kind, name) {
                break;
            }
        }
        reply.ok();
    }

    fn open(&self, _req: &Request, ino: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        let Some(node) = self.node_snapshot(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if node.is_dir {
            reply.error(Errno::EISDIR);
            return;
        }
        match self.open_staging_handle(&node, flags.0, false) {
            Ok(fh) => reply.opened(FileHandle(fh), FopenFlags::empty()),
            Err(errno) => reply.error(errno),
        }
    }

    fn create(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        if self.config.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let Some(name_str) = name.to_str().filter(|s| is_valid_segment(s)) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let Some(parent_node) = self.node_snapshot(parent) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !parent_node.is_dir {
            reply.error(Errno::ENOTDIR);
            return;
        }

        let remote_path = join_remote_path(&parent_node.remote_path, name_str);
        let entry_spec = RemoteEntry {
            name: name_str.to_owned(),
            path: remote_path,
            is_dir: false,
            size: 0,
            mtime_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let child_ino = {
            let mut guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
            guard.dir_cache.remove(&parent.0);
            guard.upsert_child(parent, &entry_spec)
        };
        let Some(node) = self.node_snapshot(child_ino) else {
            reply.error(Errno::EIO);
            return;
        };
        match self.open_staging_handle(&node, flags, true) {
            Ok(fh) => reply.created(
                &ATTR_TTL,
                &self.attr_for_entry(&node),
                Generation(0),
                FileHandle(fh),
                FopenFlags::empty(),
            ),
            Err(errno) => reply.error(errno),
        }
    }

    fn read(
        &self,
        _req: &Request,
        ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let open_arc = self
            .open_files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&fh.0)
            .cloned();
        if let Some(arc) = open_arc {
            let st = arc.lock().unwrap_or_else(|e| e.into_inner());
            if st.dirty || st.hydrated {
                let len = st.staging_file.metadata().map(|m| m.len()).unwrap_or(0);
                if offset >= len {
                    reply.data(&[]);
                    return;
                }
                let to_read = (size as u64).min(len - offset) as usize;
                let mut buf = vec![0u8; to_read];
                match st.staging_file.read_at(&mut buf, offset) {
                    Ok(n) => reply.data(&buf[..n]),
                    Err(_) => reply.error(Errno::EIO),
                }
                return;
            }
        }

        let Some(node) = self.node_snapshot(ino) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if node.is_dir {
            reply.error(Errno::EISDIR);
            return;
        }
        if offset >= node.size {
            reply.data(&[]);
            return;
        }
        let to_read = (size as u64).min(node.size - offset);
        match self.backend.read_range(&node.remote_path, offset, to_read) {
            Ok(data) => {
                let end = data.len().min(size as usize);
                reply.data(&data[..end]);
            }
            Err(err) => reply.error(err.to_errno()),
        }
    }

    fn write(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        offset: u64,
        data: &[u8],
        _write_flags: WriteFlags,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyWrite,
    ) {
        if self.config.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let open_arc = self
            .open_files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&fh.0)
            .cloned();
        let Some(arc) = open_arc else {
            reply.error(Errno::EBADF);
            return;
        };
        let mut st = arc.lock().unwrap_or_else(|e| e.into_inner());
        if !st.writable {
            reply.error(Errno::EACCES);
            return;
        }
        if let Err(errno) = self.hydrate_staging_if_needed(&mut st) {
            reply.error(errno);
            return;
        }
        let write_offset = if st.append {
            st.staging_file.metadata().map(|m| m.len()).unwrap_or(offset)
        } else {
            offset
        };
        if st.staging_file.write_all_at(data, write_offset).is_err() {
            reply.error(Errno::EIO);
            return;
        }
        st.dirty = true;
        reply.written(data.len() as u32);
    }

    fn flush(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _lock_owner: LockOwner,
        reply: ReplyEmpty,
    ) {
        let open_arc = self
            .open_files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&fh.0)
            .cloned();
        let Some(arc) = open_arc else {
            reply.ok();
            return;
        };
        let mut st = arc.lock().unwrap_or_else(|e| e.into_inner());
        match self.flush_open_state(&mut st) {
            Ok(()) => reply.ok(),
            Err(errno) => reply.error(errno),
        }
    }

    fn release(
        &self,
        _req: &Request,
        _ino: INodeNo,
        fh: FileHandle,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        let open_arc = self
            .open_files
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&fh.0);
        if let Some(arc) = open_arc {
            let mut st = arc.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(err) = self.flush_open_state(&mut st) {
                warn!("FUSE release flush failed for {}: {:?}", st.remote_path, err);
            }
            let _ = fs::remove_file(&st.staging_path);
        }
        reply.ok();
    }

    fn mkdir(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        _mode: u32,
        _umask: u32,
        reply: ReplyEntry,
    ) {
        if self.config.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let Some(name_str) = name.to_str().filter(|s| is_valid_segment(s)) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let Some(parent_node) = self.node_snapshot(parent) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if !parent_node.is_dir {
            reply.error(Errno::ENOTDIR);
            return;
        }
        let target_path = join_remote_path(&parent_node.remote_path, name_str);
        if let Err(err) = self.backend.create_folder(&target_path) {
            reply.error(err.to_errno());
            return;
        }
        let remote_entry = RemoteEntry {
            name: name_str.to_owned(),
            path: target_path,
            is_dir: true,
            size: 0,
            mtime_secs: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
        };
        let child_ino = {
            let mut guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
            guard.dir_cache.remove(&parent.0);
            guard.upsert_child(parent, &remote_entry)
        };
        let Some(node) = self.node_snapshot(child_ino) else {
            reply.error(Errno::EIO);
            return;
        };
        reply.entry(&ATTR_TTL, &self.attr_for_entry(&node), Generation(0));
    }

    fn unlink(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        if self.config.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let Some(name_str) = name.to_str().filter(|s| is_valid_segment(s)) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let Some(parent_node) = self.node_snapshot(parent) else {
            reply.error(Errno::ENOENT);
            return;
        };
        let target_path = join_remote_path(&parent_node.remote_path, name_str);
        if let Err(err) = self.backend.delete(&target_path, self.config.use_trash) {
            reply.error(err.to_errno());
            return;
        }
        let mut guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove_subtree(&target_path);
        guard.dir_cache.remove(&parent.0);
        reply.ok();
    }

    fn rmdir(&self, _req: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEmpty) {
        if self.config.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let Some(name_str) = name.to_str().filter(|s| is_valid_segment(s)) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let Some(parent_node) = self.node_snapshot(parent) else {
            reply.error(Errno::ENOENT);
            return;
        };
        let target_path = join_remote_path(&parent_node.remote_path, name_str);
        if let Ok(children) = self.backend.list_dir(&target_path) {
            if !children.is_empty() {
                reply.error(Errno::ENOTEMPTY);
                return;
            }
        }
        if let Err(err) = self.backend.delete(&target_path, self.config.use_trash) {
            reply.error(err.to_errno());
            return;
        }
        let mut guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
        guard.remove_subtree(&target_path);
        guard.dir_cache.remove(&parent.0);
        reply.ok();
    }

    fn rename(
        &self,
        _req: &Request,
        parent: INodeNo,
        name: &OsStr,
        newparent: INodeNo,
        newname: &OsStr,
        _flags: RenameFlags,
        reply: ReplyEmpty,
    ) {
        if self.config.read_only {
            reply.error(Errno::EROFS);
            return;
        }
        let (Some(old_name), Some(new_name)) = (
            name.to_str().filter(|s| is_valid_segment(s)),
            newname.to_str().filter(|s| is_valid_segment(s)),
        ) else {
            reply.error(Errno::EINVAL);
            return;
        };
        let (Some(src_parent), Some(dst_parent)) =
            (self.node_snapshot(parent), self.node_snapshot(newparent))
        else {
            reply.error(Errno::ENOENT);
            return;
        };
        let src_path = join_remote_path(&src_parent.remote_path, old_name);
        let dst_path = join_remote_path(&dst_parent.remote_path, new_name);
        debug!("FUSE rename {src_path} -> {dst_path}");
        if let Err(err) = self.backend.rename(&src_path, &dst_path) {
            reply.error(err.to_errno());
            return;
        }
        let mut guard = self.inodes.lock().unwrap_or_else(|e| e.into_inner());
        guard.relocate(&src_path, newparent, new_name, &dst_path);
        guard.dir_cache.remove(&parent.0);
        guard.dir_cache.remove(&newparent.0);
        reply.ok();
    }

    fn statfs(&self, _req: &Request, _ino: INodeNo, reply: ReplyStatfs) {
        let Ok(stats) = self.backend.stat_fs() else {
            reply.statfs(0, 0, 0, 0, 0, BLOCK_SIZE, 255, BLOCK_SIZE);
            return;
        };
        let bsize = u64::from(BLOCK_SIZE);
        let blocks = stats.total_bytes / bsize;
        let bfree = stats.free_bytes / bsize;
        reply.statfs(blocks, bfree, bfree, 0, 0, BLOCK_SIZE, 255, BLOCK_SIZE);
    }
}

pub fn normalize_remote_path(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub fn join_remote_path(parent: &str, name: &str) -> String {
    let clean_parent = normalize_remote_path(parent);
    if clean_parent == "/" {
        format!("/{name}")
    } else {
        format!("{clean_parent}/{name}")
    }
}

pub fn is_valid_segment(seg: &str) -> bool {
    !seg.is_empty() && seg != "." && seg != ".." && !seg.contains('/') && !seg.contains('\0')
}

pub fn normalize_timestamp_secs(raw: u64) -> u64 {
    if raw > 10_000_000_000 {
        raw / 1000
    } else {
        raw
    }
}

fn current_uid_gid() -> (u32, u32) {
    fs::metadata("/proc/self")
        .map(|m| (m.uid(), m.gid()))
        .unwrap_or((1000, 1000))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockBackend {
        files: Mutex<HashMap<String, (bool, Vec<u8>)>>,
    }

    impl MockBackend {
        fn new() -> Self {
            let mut map = HashMap::new();
            map.insert("/storage/emulated/0".to_owned(), (true, Vec::new()));
            map.insert("/storage/emulated/0/DCIM".to_owned(), (true, Vec::new()));
            map.insert(
                "/storage/emulated/0/hello.txt".to_owned(),
                (false, b"hello handshaker".to_vec()),
            );
            Self {
                files: Mutex::new(map),
            }
        }
    }

    impl RemoteFsBackend for MockBackend {
        fn list_dir(&self, remote_path: &str) -> Result<Vec<RemoteEntry>, BackendError> {
            let norm = normalize_remote_path(remote_path);
            let prefix = format!("{norm}/");
            let guard = self.files.lock().unwrap();
            let mut out = Vec::new();
            for (path, (is_dir, data)) in guard.iter() {
                if let Some(rest) = path.strip_prefix(&prefix) {
                    if !rest.is_empty() && !rest.contains('/') {
                        out.push(RemoteEntry {
                            name: rest.to_owned(),
                            path: path.clone(),
                            is_dir: *is_dir,
                            size: data.len() as u64,
                            mtime_secs: 1_700_000_000,
                        });
                    }
                }
            }
            out.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(out)
        }

        fn stat_fs(&self) -> Result<FsStats, BackendError> {
            Ok(FsStats {
                total_bytes: 64 * 1024 * 1024,
                free_bytes: 32 * 1024 * 1024,
            })
        }

        fn read_range(
            &self,
            remote_path: &str,
            offset: u64,
            length: u64,
        ) -> Result<Vec<u8>, BackendError> {
            let guard = self.files.lock().unwrap();
            let (is_dir, data) = guard.get(remote_path).ok_or(BackendError::NotFound)?;
            if *is_dir {
                return Err(BackendError::Io("cannot read directory range".to_owned()));
            }
            let start = (offset as usize).min(data.len());
            let end = start.saturating_add(length as usize).min(data.len());
            Ok(data[start..end].to_vec())
        }

        fn upload_file(&self, local_path: &Path, remote_path: &str) -> Result<(), BackendError> {
            let bytes = fs::read(local_path).map_err(|e| BackendError::Io(e.to_string()))?;
            self.files
                .lock()
                .unwrap()
                .insert(normalize_remote_path(remote_path), (false, bytes));
            Ok(())
        }

        fn create_folder(&self, remote_path: &str) -> Result<(), BackendError> {
            let norm = normalize_remote_path(remote_path);
            let mut guard = self.files.lock().unwrap();
            if guard.contains_key(&norm) {
                return Err(BackendError::AlreadyExists);
            }
            guard.insert(norm, (true, Vec::new()));
            Ok(())
        }

        fn rename(&self, src_path: &str, dst_path: &str) -> Result<(), BackendError> {
            let src = normalize_remote_path(src_path);
            let dst = normalize_remote_path(dst_path);
            let mut guard = self.files.lock().unwrap();
            let val = guard.remove(&src).ok_or(BackendError::NotFound)?;
            guard.insert(dst, val);
            Ok(())
        }

        fn delete(&self, remote_path: &str, _is_trash: bool) -> Result<(), BackendError> {
            let norm = normalize_remote_path(remote_path);
            self.files
                .lock()
                .unwrap()
                .remove(&norm)
                .map(|_| ())
                .ok_or(BackendError::NotFound)
        }
    }

    #[test]
    fn path_and_timestamp_helpers_normalize_cleanly() {
        assert_eq!(normalize_remote_path("/storage/emulated/0/"), "/storage/emulated/0");
        assert_eq!(join_remote_path("/storage/emulated/0/", "DCIM"), "/storage/emulated/0/DCIM");
        assert!(is_valid_segment("photo.jpg"));
        assert!(!is_valid_segment(".."));
        assert!(!is_valid_segment("a/b"));
        assert_eq!(normalize_timestamp_secs(1_700_000_000_000), 1_700_000_000);
        assert_eq!(normalize_timestamp_secs(1_700_000_000), 1_700_000_000);
    }

    #[test]
    fn inode_table_tracks_dir_children_relocation_and_removal() {
        let backend = Arc::new(MockBackend::new());
        let fs_impl = HandShakerFs::new(
            Arc::clone(&backend),
            HandShakerFsConfig {
                root_remote_path: "/storage/emulated/0".to_owned(),
                read_only: false,
                use_trash: false,
            },
        )
        .unwrap();

        let children = fs_impl.refresh_dir_locked(INodeNo::ROOT).unwrap();
        assert_eq!(children.len(), 2);
        let (_, hello_name, is_dir) = children.iter().find(|(_, n, _)| n == "hello.txt").unwrap();
        assert_eq!(hello_name, "hello.txt");
        assert!(!is_dir);

        let hello_ino = children.iter().find(|(_, n, _)| n == "hello.txt").unwrap().0;
        let hello_node = fs_impl.node_snapshot(hello_ino).unwrap();
        assert_eq!(hello_node.size, 16);

        let fh = fs_impl.open_staging_handle(&hello_node, O_RDWR_FLAG, false).unwrap();
        let arc = fs_impl.open_files.lock().unwrap().get(&fh).cloned().unwrap();
        {
            let mut st = arc.lock().unwrap();
            fs_impl.hydrate_staging_if_needed(&mut st).unwrap();
            st.staging_file.write_all_at(b"!", 16).unwrap();
            st.dirty = true;
            fs_impl.flush_open_state(&mut st).unwrap();
        }
        assert_eq!(
            backend.files.lock().unwrap().get("/storage/emulated/0/hello.txt").unwrap().1,
            b"hello handshaker!"
        );

        {
            let mut guard = fs_impl.inodes.lock().unwrap();
            guard.relocate(
                "/storage/emulated/0/hello.txt",
                INodeNo::ROOT,
                "renamed.txt",
                "/storage/emulated/0/renamed.txt",
            );
            assert!(guard.by_path.contains_key("/storage/emulated/0/renamed.txt"));
            guard.remove_subtree("/storage/emulated/0/DCIM");
            assert!(!guard.by_path.contains_key("/storage/emulated/0/DCIM"));
        }
    }
}
