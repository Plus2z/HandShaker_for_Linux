import { create } from 'zustand';
import { formatTrashSourcePhone, nameCollator } from '../lib/format';
import { getMessages, resolveLanguage } from '../lib/i18n';
import { ipc } from '../lib/ipc';
import type {
  AppSettings,
  AppSettingsPatch,
  AudioItem,
  AudioLibrary,
  ClipboardItem,
  ConnectionState,
  DeviceInfo,
  DiscoverySnapshot,
  DownloadTarget,
  FileEntry,
  MediaPlaybackSupport,
  PhotoLibrary,
  TransferItem,
  TrashItem,
  TrustedDevice,
  VideoItem,
  VideoLibrary,
} from '../types/api';

export type NavTab =
  | 'files'
  | 'photos'
  | 'videos'
  | 'music'
  | 'clipboard'
  | 'settings'
  | 'trash';
export type SortKey = 'name' | 'size' | 'modified';
export type SortDir = 'asc' | 'desc';

interface AppStore {
  initialized: boolean;
  tab: NavTab;
  settings: AppSettings;
  connection: ConnectionState;
  discovery: DiscoverySnapshot;
  scanning: boolean;
  trustedDevices: TrustedDevice[];
  deviceInfo: DeviceInfo | null;

  currentPath: string;
  files: FileEntry[];
  filesLoading: boolean;
  selectedPaths: string[];
  sortKey: SortKey;
  sortDir: SortDir;
  fileSearch: string;

  transfers: TransferItem[];
  transferDrawerOpen: boolean;

  photoLibrary: PhotoLibrary | null;
  selectedAlbumId: number | null;
  quickLookIndex: number | null;
  photosLoading: boolean;

  videoLibrary: VideoLibrary | null;
  activeVideo: VideoItem | null;
  videosLoading: boolean;

  audioLibrary: AudioLibrary | null;
  activeAudio: AudioItem | null;
  audioLoading: boolean;
  mediaSupport: MediaPlaybackSupport | null;

  clipboard: ClipboardItem[];
  clipboardLoading: boolean;
  trashItems: TrashItem[];
  trashLoading: boolean;
  trashBusy: boolean;
  toastError: string | null;
  statusNotice: string | null;

  setTab: (tab: NavTab) => void;
  setToastError: (msg: string | null) => void;
  setStatusNotice: (msg: string | null) => void;
  setTransferDrawerOpen: (open: boolean) => void;
  setFileSearch: (q: string) => void;
  setSort: (key: SortKey) => void;
  toggleSelectPath: (path: string, multi: boolean) => void;
  selectAllFiles: (checked: boolean) => void;
  setSelectedAlbumId: (id: number | null) => void;
  setQuickLookIndex: (idx: number | null) => void;
  setActiveVideo: (video: VideoItem | null) => void;
  setActiveAudio: (track: AudioItem | null) => void;

  bootstrap: () => Promise<() => void>;
  refreshScan: () => Promise<void>;
  refreshTrustedDevices: () => Promise<void>;
  unpairTrustedDevice: (uuid: string) => Promise<void>;
  updateSettings: (patch: AppSettingsPatch) => Promise<void>;

  connectWifi: (host: string, port: number) => Promise<void>;
  connectUsb: (bus: number | null, address: number | null, allowSwitch: boolean) => Promise<void>;
  disconnect: () => Promise<void>;

  loadDeviceInfo: () => Promise<void>;
  openDir: (path: string) => Promise<void>;
  createFolder: (path: string) => Promise<void>;
  renameFile: (srcPath: string, dstPath: string) => Promise<void>;
  deleteFiles: (paths: string[], isTrash: boolean) => Promise<boolean>;
  loadTrash: () => Promise<void>;
  restoreTrash: (entryNames: string[]) => Promise<boolean>;
  deleteTrash: (entryNames: string[]) => Promise<boolean>;
  emptyTrash: () => Promise<boolean>;

  enqueueDownloads: (targets: DownloadTarget[], localDir?: string) => Promise<void>;
  enqueueUploads: (localPaths: string[], remoteDir?: string) => Promise<void>;
  cancelTransfer: (id: string) => Promise<void>;
  clearFinishedTransfers: () => Promise<void>;

  loadPhotos: () => Promise<void>;
  loadVideos: () => Promise<void>;
  loadAudio: () => Promise<void>;
  loadClipboard: () => Promise<void>;
  pushClipboard: (text: string) => Promise<void>;
  clearClipboard: () => Promise<void>;
  deleteClipboard: (item: ClipboardItem) => Promise<void>;
  copyClipboardToLinux: (text: string) => Promise<void>;
  openMediaExternally: (url: string) => Promise<void>;
}

let bootstrapEpoch = 0;

const DEFAULT_SETTINGS: AppSettings = {
  useSystemDecorations: false,
  theme: 'system',
  language: 'system',
  downloadDir: '',
  trashDir: '~/.HandShakerTrash',
  clipboardAutoSync: true,
  isTilingWm: false,
};

export function sortFileEntries(
  entries: FileEntry[],
  sortKey: SortKey,
  sortDir: SortDir,
  search: string,
): FileEntry[] {
  const q = search.trim().toLowerCase();
  const filtered = q ? entries.filter((e) => e.name.toLowerCase().includes(q)) : [...entries];
  const factor = sortDir === 'asc' ? 1 : -1;

  filtered.sort((a, b) => {
    if (a.isDirectory !== b.isDirectory) {
      return a.isDirectory ? -1 : 1;
    }
    if (sortKey === 'size') {
      const diff = a.fileSize - b.fileSize;
      return diff !== 0 ? diff * factor : nameCollator.compare(a.name, b.name);
    }
    if (sortKey === 'modified') {
      const diff = a.modifiedTimestamp - b.modifiedTimestamp;
      return diff !== 0 ? diff * factor : nameCollator.compare(a.name, b.name);
    }
    return nameCollator.compare(a.name, b.name) * factor;
  });
  return filtered;
}

export const useAppStore = create<AppStore>((set, get) => ({
  initialized: false,
  tab: 'files',
  settings: DEFAULT_SETTINGS,
  connection: { status: 'disconnected', handshake: null, error: null },
  discovery: { wifi: [], usb: [] },
  scanning: false,
  trustedDevices: [],
  deviceInfo: null,

  currentPath: '/storage/emulated/0',
  files: [],
  filesLoading: false,
  selectedPaths: [],
  sortKey: 'name',
  sortDir: 'asc',
  fileSearch: '',

  transfers: [],
  transferDrawerOpen: false,

  photoLibrary: null,
  selectedAlbumId: null,
  quickLookIndex: null,
  photosLoading: false,

  videoLibrary: null,
  activeVideo: null,
  videosLoading: false,

  audioLibrary: null,
  activeAudio: null,
  audioLoading: false,
  mediaSupport: null,

  clipboard: [],
  clipboardLoading: false,
  trashItems: [],
  trashLoading: false,
  trashBusy: false,
  toastError: null,
  statusNotice: null,

  setTab: (tab) => {
    set({ tab });
    const conn = get().connection.status;
    if (conn !== 'connected') return;
    if (tab === 'photos' && !get().photoLibrary) void get().loadPhotos();
    if (tab === 'videos' && !get().videoLibrary) void get().loadVideos();
    if (tab === 'music' && !get().audioLibrary) void get().loadAudio();
    if (tab === 'clipboard') void get().loadClipboard();
  },

  setToastError: (toastError) => set({ toastError }),
  setStatusNotice: (statusNotice) => set({ statusNotice }),
  setTransferDrawerOpen: (transferDrawerOpen) => set({ transferDrawerOpen }),
  setFileSearch: (fileSearch) => set({ fileSearch }),

  setSort: (key) => {
    const { sortKey, sortDir } = get();
    const nextDir: SortDir = sortKey === key && sortDir === 'asc' ? 'desc' : 'asc';
    set({ sortKey: key, sortDir: nextDir });
  },

  toggleSelectPath: (path, multi) => {
    const cur = get().selectedPaths;
    if (!multi) {
      set({ selectedPaths: [path] });
      return;
    }
    if (cur.includes(path)) {
      set({ selectedPaths: cur.filter((p) => p !== path) });
    } else {
      set({ selectedPaths: [...cur, path] });
    }
  },

  selectAllFiles: (checked) => {
    if (!checked) {
      set({ selectedPaths: [] });
      return;
    }
    const visible = sortFileEntries(
      get().files,
      get().sortKey,
      get().sortDir,
      get().fileSearch,
    );
    set({ selectedPaths: visible.map((f) => f.path) });
  },

  setSelectedAlbumId: (selectedAlbumId) => set({ selectedAlbumId, quickLookIndex: null }),
  setQuickLookIndex: (quickLookIndex) => set({ quickLookIndex }),
  setActiveVideo: (activeVideo) => set({ activeVideo }),
  setActiveAudio: (activeAudio) => set({ activeAudio }),

  bootstrap: async () => {
    const epoch = ++bootstrapEpoch;
    try {
      const [settings, connection, transfers, trustedDevices, mediaSupport] = await Promise.all([
        ipc.getSettings(),
        ipc.getConnectionState(),
        ipc.listTransfers(),
        ipc.listTrustedDevices(),
        ipc.getMediaPlaybackSupport().catch(() => null),
      ]);
      if (epoch !== bootstrapEpoch) {
        return () => {};
      }
      set({ initialized: true, settings, connection, transfers, trustedDevices, mediaSupport });
      if (connection.status === 'connected') {
        await get().loadDeviceInfo();
      } else {
        void get().refreshScan();
      }
    } catch (e) {
      if (epoch !== bootstrapEpoch) {
        return () => {};
      }
      set({ initialized: true, toastError: String(e) });
    }

    const unsubs = await Promise.all([
      ipc.listenConnection((connection) => {
        const prev = get().connection.status;
        set({ connection });
        if (connection.error) {
          set({ toastError: connection.error });
        }
        if (connection.status === 'connected' && prev !== 'connected') {
          void get().loadDeviceInfo();
          void get().refreshTrustedDevices();
        }
        if (connection.status === 'disconnected') {
          set({
            deviceInfo: null,
            files: [],
            selectedPaths: [],
            photoLibrary: null,
            selectedAlbumId: null,
            videoLibrary: null,
            audioLibrary: null,
            clipboard: [],
            quickLookIndex: null,
            activeVideo: null,
            activeAudio: null,
          });
        }
      }),
      ipc.listenTransfers((transfers) => set({ transfers })),
      ipc.listenClipboard((clipboard) => set({ clipboard })),
      ipc.listenLibraryChange((kind) => {
        if (kind === 'photo') void get().loadPhotos();
        if (kind === 'video') void get().loadVideos();
        if (kind === 'audio') void get().loadAudio();
      }),
      ipc.listenFolderChange(() => {
        if (get().connection.status === 'connected') {
          void get().openDir(get().currentPath);
        }
      }),
    ]);

    if (epoch !== bootstrapEpoch) {
      for (const u of unsubs) u();
      return () => {};
    }

    return () => {
      if (epoch === bootstrapEpoch) {
        bootstrapEpoch++;
      }
      for (const u of unsubs) u();
    };
  },

  refreshScan: async () => {
    set({ scanning: true });
    try {
      const discovery = await ipc.scanDevices(2800);
      set({ discovery });
    } catch (e) {
      set({ toastError: String(e) });
    } finally {
      set({ scanning: false });
    }
  },

  refreshTrustedDevices: async () => {
    try {
      const trustedDevices = await ipc.listTrustedDevices();
      set({ trustedDevices });
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  unpairTrustedDevice: async (uuid) => {
    try {
      await ipc.removeTrustedDevice(uuid);
      await get().refreshTrustedDevices();
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  updateSettings: async (patch) => {
    try {
      const settings = await ipc.updateSettings(patch);
      set({ settings });
    } catch (e) {
      const msg = String(e);
      if (msg.includes('Invalid trash directory path')) {
        const t = getMessages(resolveLanguage(get().settings.language));
        set({ toastError: t.settings.trashDirInvalid });
      } else {
        set({ toastError: msg });
      }
    }
  },

  connectWifi: async (host, port) => {
    set({ toastError: null });
    try {
      const connection = await ipc.connectWifi(host, port);
      set({ connection });
      await get().loadDeviceInfo();
      await get().refreshTrustedDevices();
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  connectUsb: async (bus, address, allowSwitch) => {
    set({ toastError: null });
    try {
      const connection = await ipc.connectUsb(bus, address, allowSwitch);
      set({ connection });
      await get().loadDeviceInfo();
      await get().refreshTrustedDevices();
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  disconnect: async () => {
    try {
      await ipc.disconnectSession();
      void get().refreshScan();
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  loadDeviceInfo: async () => {
    try {
      const deviceInfo = await ipc.getDeviceInfo();
      const root = deviceInfo.rootPath || '/storage/emulated/0';
      set({ deviceInfo, currentPath: root });
      await get().openDir(root);
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  openDir: async (path) => {
    set({ filesLoading: true });
    try {
      const files = await ipc.listDir(path, 1);
      set({ currentPath: path, files, selectedPaths: [] });
    } catch (e) {
      set({ toastError: String(e) });
    } finally {
      set({ filesLoading: false });
    }
  },

  createFolder: async (path) => {
    try {
      await ipc.createFolder(path);
      await get().openDir(get().currentPath);
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  renameFile: async (srcPath, dstPath) => {
    try {
      await ipc.renameFile(srcPath, dstPath);
      await get().openDir(get().currentPath);
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  deleteFiles: async (paths, isTrash) => {
    const uniquePaths = Array.from(new Set(paths)).filter(Boolean);
    if (uniquePaths.length === 0) return false;
    try {
      await ipc.deleteFiles(uniquePaths, isTrash);
      const deletedSet = new Set(uniquePaths);
      const {
        tab,
        connection,
        currentPath,
        files,
        selectedPaths,
        photoLibrary,
        selectedAlbumId,
        quickLookIndex,
        videoLibrary,
        activeVideo,
        audioLibrary,
        activeAudio,
      } = get();

      const fileHit = files.some((f) => deletedSet.has(f.path));
      let nextPhotoLibrary = photoLibrary;
      let nextQuickLookIndex = quickLookIndex;
      if (photoLibrary) {
        const nextImages = photoLibrary.images.filter((img) => !deletedSet.has(img.path));
        if (nextImages.length !== photoLibrary.images.length) {
          nextPhotoLibrary = { ...photoLibrary, images: nextImages };
          if (quickLookIndex !== null) {
            const visibleAfter =
              selectedAlbumId === null
                ? nextImages
                : nextImages.filter((img) => img.albumId === selectedAlbumId);
            nextQuickLookIndex =
              visibleAfter.length === 0 ? null : Math.min(quickLookIndex, visibleAfter.length - 1);
          }
        }
      }

      let nextVideoLibrary = videoLibrary;
      let nextActiveVideo = activeVideo;
      if (videoLibrary) {
        const nextVideos = videoLibrary.videos.filter((v) => !deletedSet.has(v.path));
        if (nextVideos.length !== videoLibrary.videos.length) {
          nextVideoLibrary = { ...videoLibrary, videos: nextVideos };
          if (activeVideo && deletedSet.has(activeVideo.path)) {
            nextActiveVideo = null;
          }
        }
      }

      let nextAudioLibrary = audioLibrary;
      let nextActiveAudio = activeAudio;
      if (audioLibrary) {
        const nextTracks = audioLibrary.tracks.filter((tr) => !deletedSet.has(tr.path));
        if (nextTracks.length !== audioLibrary.tracks.length) {
          nextAudioLibrary = { ...audioLibrary, tracks: nextTracks };
          if (activeAudio && deletedSet.has(activeAudio.path)) {
            nextActiveAudio = null;
          }
        }
      }

      set({
        files: files.filter((f) => !deletedSet.has(f.path)),
        selectedPaths: selectedPaths.filter((p) => !deletedSet.has(p)),
        photoLibrary: nextPhotoLibrary,
        quickLookIndex: nextQuickLookIndex,
        videoLibrary: nextVideoLibrary,
        activeVideo: nextActiveVideo,
        audioLibrary: nextAudioLibrary,
        activeAudio: nextActiveAudio,
      });

      if (connection.status === 'connected' && (tab === 'files' || fileHit)) {
        await get().openDir(currentPath);
      }
      return true;
    } catch (e) {
      set({ toastError: String(e) });
      return false;
    }
  },

  enqueueDownloads: async (targets, localDir) => {
    try {
      const count = await ipc.enqueueDownload(targets, localDir);
      if (count > 0) set({ transferDrawerOpen: true });
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  enqueueUploads: async (localPaths, remoteDir) => {
    try {
      const dir = remoteDir ?? get().currentPath;
      const count = await ipc.enqueueUpload(localPaths, dir);
      if (count > 0) set({ transferDrawerOpen: true });
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  cancelTransfer: async (id) => {
    try {
      await ipc.cancelTransfer(id);
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  clearFinishedTransfers: async () => {
    try {
      await ipc.clearFinishedTransfers();
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  loadPhotos: async () => {
    set({ photosLoading: true });
    try {
      const photoLibrary = await ipc.getPhotoLibrary();
      set({ photoLibrary });
    } catch (e) {
      set({ toastError: String(e) });
    } finally {
      set({ photosLoading: false });
    }
  },

  loadVideos: async () => {
    set({ videosLoading: true });
    try {
      const videoLibrary = await ipc.getVideoLibrary();
      set({ videoLibrary });
    } catch (e) {
      set({ toastError: String(e) });
    } finally {
      set({ videosLoading: false });
    }
  },

  loadAudio: async () => {
    set({ audioLoading: true });
    try {
      const audioLibrary = await ipc.getAudioLibrary();
      set({ audioLibrary });
    } catch (e) {
      set({ toastError: String(e) });
    } finally {
      set({ audioLoading: false });
    }
  },

  loadClipboard: async () => {
    set({ clipboardLoading: true });
    try {
      const clipboard = await ipc.getClipboardHistory();
      set({ clipboard });
    } catch (e) {
      set({ toastError: String(e) });
    } finally {
      set({ clipboardLoading: false });
    }
  },

  pushClipboard: async (text) => {
    if (!text.trim()) return;
    try {
      await ipc.postClipboardText(text);
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  clearClipboard: async () => {
    try {
      await ipc.clearClipboardHistory();
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  deleteClipboard: async (item) => {
    try {
      await ipc.deleteClipboardItem(item.timestampMs, item.rawGzipB64);
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  copyClipboardToLinux: async (text) => {
    try {
      await ipc.copyToLinuxClipboard(text);
      const t = getMessages(resolveLanguage(get().settings.language));
      set({ statusNotice: t.app.copiedClipboardSnippet });
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  openMediaExternally: async (url) => {
    try {
      const player = await ipc.openMediaExternally(url);
      const t = getMessages(resolveLanguage(get().settings.language));
      set({ statusNotice: t.app.openedStreamIn(player) });
    } catch (e) {
      set({ toastError: String(e) });
    }
  },

  loadTrash: async () => {
    set({ trashLoading: true });
    try {
      const trashItems = await ipc.listTrashItems();
      set({ trashItems });
    } catch (e) {
      set({ toastError: String(e) });
    } finally {
      set({ trashLoading: false });
    }
  },

  restoreTrash: async (entryNames) => {
    if (entryNames.length === 0) return true;
    const state = get();
    const t = getMessages(resolveLanguage(state.settings.language));
    const currentUuid = state.connection.handshake?.deviceUuid?.trim() || '';
    if (state.connection.status === 'connected' && currentUuid) {
      const byName = new Map(state.trashItems.map((it) => [it.entryName, it]));
      for (const name of entryNames) {
        const item = byName.get(name);
        if (!item) continue;
        const itemUuid = item.deviceUuid?.trim() || '';
        if (!itemUuid) {
          set({ toastError: t.trash.restoreMissingOriginError(name) });
          return false;
        }
        if (itemUuid !== currentUuid) {
          const sourceLabel = formatTrashSourcePhone(item) || itemUuid;
          set({ toastError: t.trash.restoreWrongDeviceError(name, sourceLabel) });
          return false;
        }
      }
    }

    set({ trashBusy: true, toastError: null });
    try {
      const restoredPaths = await ipc.restoreTrashItems(entryNames);
      const trashItems = await ipc.listTrashItems();
      set({
        trashItems,
        statusNotice: t.trash.restoredNotice(restoredPaths.length),
      });
      const currentPath = get().currentPath;
      const prefix = currentPath === '/' ? '/' : `${currentPath}/`;
      if (
        get().connection.status === 'connected' &&
        restoredPaths.some((p) => p.startsWith(prefix))
      ) {
        void get().openDir(currentPath);
      }
      return true;
    } catch (e) {
      const msg = String(e);
      if (msg.includes('Missing source phone metadata')) {
        set({ toastError: t.trash.restoreMissingOriginGeneric });
      } else if (msg.includes('Device mismatch')) {
        set({ toastError: t.trash.restoreWrongDeviceGeneric });
      } else {
        set({ toastError: msg });
      }
      return false;
    } finally {
      set({ trashBusy: false });
    }
  },

  deleteTrash: async (entryNames) => {
    if (entryNames.length === 0) return true;
    set({ trashBusy: true });
    try {
      const deleted = await ipc.deleteTrashItems(entryNames);
      const trashItems = await ipc.listTrashItems();
      const t = getMessages(resolveLanguage(get().settings.language));
      set({
        trashItems,
        statusNotice: t.trash.deletedNotice(deleted),
      });
      return true;
    } catch (e) {
      set({ toastError: String(e) });
      return false;
    } finally {
      set({ trashBusy: false });
    }
  },

  emptyTrash: async () => {
    set({ trashBusy: true });
    try {
      const deleted = await ipc.emptyTrash();
      const t = getMessages(resolveLanguage(get().settings.language));
      set({
        trashItems: [],
        statusNotice: t.trash.emptiedNotice(deleted),
      });
      return true;
    } catch (e) {
      set({ toastError: String(e) });
      return false;
    } finally {
      set({ trashBusy: false });
    }
  },
}));
