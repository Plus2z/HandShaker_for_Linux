import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { open } from '@tauri-apps/plugin-dialog';
import type {
  AppSettings,
  AppSettingsPatch,
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
  VideoLibrary,
} from '../types/api';

export const ipc = {
  getSettings: () => invoke<AppSettings>('get_settings'),
  updateSettings: (patch: AppSettingsPatch) => invoke<AppSettings>('update_settings', { patch }),
  windowMinimize: () => invoke<void>('window_minimize'),
  windowToggleMaximize: () => invoke<boolean>('window_toggle_maximize'),
  windowClose: () => invoke<void>('window_close'),

  scanDevices: (wifiTimeoutMs = 2800) =>
    invoke<DiscoverySnapshot>('scan_devices', { wifiTimeoutMs }),
  listTrustedDevices: () => invoke<TrustedDevice[]>('list_trusted_devices'),
  removeTrustedDevice: (deviceUuid: string) =>
    invoke<boolean>('remove_trusted_device', { deviceUuid }),
  getConnectionState: () => invoke<ConnectionState>('get_connection_state'),
  connectWifi: (host: string, port: number) =>
    invoke<ConnectionState>('connect_wifi', { host, port }),
  connectUsb: (bus: number | null, address: number | null, allowSwitch: boolean) =>
    invoke<ConnectionState>('connect_usb', { bus, address, allowSwitch }),
  disconnectSession: () => invoke<void>('disconnect_session'),

  getDeviceInfo: () => invoke<DeviceInfo>('get_device_info'),
  listDir: (path: string, maxdepth = 1) => invoke<FileEntry[]>('list_dir', { path, maxdepth }),
  createFolder: (path: string) => invoke<void>('create_folder', { path }),
  renameFile: (srcPath: string, dstPath: string) =>
    invoke<void>('rename_file', { srcPath, dstPath }),
  deleteFiles: (paths: string[], isTrash = true) =>
    invoke<void>('delete_files', { paths, isTrash }),
  listTrashItems: () => invoke<TrashItem[]>('list_trash_items'),
  restoreTrashItems: (entryNames: string[]) =>
    invoke<string[]>('restore_trash_items', { entryNames }),
  deleteTrashItems: (entryNames: string[]) =>
    invoke<number>('delete_trash_items', { entryNames }),
  emptyTrash: () => invoke<number>('empty_trash'),

  listTransfers: () => invoke<TransferItem[]>('list_transfers'),
  enqueueDownload: (targets: DownloadTarget[], localDir?: string) =>
    invoke<number>('enqueue_download', { targets, localDir: localDir ?? null }),
  enqueueUpload: (localPaths: string[], remoteDir: string) =>
    invoke<number>('enqueue_upload', { localPaths, remoteDir }),
  cancelTransfer: (id: string) => invoke<boolean>('cancel_transfer', { id }),
  clearFinishedTransfers: () => invoke<void>('clear_finished_transfers'),

  getPhotoLibrary: () => invoke<PhotoLibrary>('get_photo_library'),
  getVideoLibrary: () => invoke<VideoLibrary>('get_video_library'),
  getAudioLibrary: () => invoke<AudioLibrary>('get_audio_library'),
  getMediaPlaybackSupport: () => invoke<MediaPlaybackSupport>('get_media_playback_support'),
  openMediaExternally: (url: string) => invoke<string>('open_media_externally', { url }),

  getClipboardHistory: () => invoke<ClipboardItem[]>('get_clipboard_history'),
  postClipboardText: (text: string) => invoke<void>('post_clipboard_text', { text }),
  clearClipboardHistory: () => invoke<void>('clear_clipboard_history'),
  deleteClipboardItem: (timestampMs: number, rawGzipB64: string) =>
    invoke<void>('delete_clipboard_item', { timestampMs, rawGzipB64 }),
  copyToLinuxClipboard: (text: string) => invoke<void>('copy_to_linux_clipboard', { text }),

  pickLocalPaths: async (directory: boolean, multiple = true): Promise<string[]> => {
    const selected = await open({ directory, multiple });
    if (!selected) return [];
    return Array.isArray(selected) ? selected : [selected];
  },

  listenConnection: (cb: (s: ConnectionState) => void): Promise<UnlistenFn> =>
    listen<ConnectionState>('connection://state', (ev) => cb(ev.payload)),
  listenTransfers: (cb: (items: TransferItem[]) => void): Promise<UnlistenFn> =>
    listen<TransferItem[]>('transfer://updated', (ev) => cb(ev.payload)),
  listenClipboard: (cb: (items: ClipboardItem[]) => void): Promise<UnlistenFn> =>
    listen<ClipboardItem[]>('clipboard://changed', (ev) => cb(ev.payload)),
  listenLibraryChange: (cb: (kind: string) => void): Promise<UnlistenFn> =>
    listen<string>('library://changed', (ev) => cb(ev.payload)),
  listenFolderChange: (cb: () => void): Promise<UnlistenFn> =>
    listen('folder://changed', () => cb()),
};
