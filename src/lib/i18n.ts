import type { LanguagePreference } from '../types/api';

export type ResolvedLanguage = 'en' | 'zh-CN';

export function resolveLanguage(
  pref?: LanguagePreference | string | null,
  browserLanguages?: readonly string[],
): ResolvedLanguage {
  if (pref === 'zh-CN' || pref === 'zh') return 'zh-CN';
  if (pref === 'en') return 'en';

  const candidates =
    browserLanguages ??
    (typeof navigator !== 'undefined'
      ? navigator.languages?.length
        ? navigator.languages
        : [navigator.language]
      : []);

  for (const tag of candidates) {
    if (typeof tag === 'string' && tag.trim().toLowerCase().startsWith('zh')) {
      return 'zh-CN';
    }
  }
  return 'en';
}

export interface I18nMessages {
  common: {
    cancel: string;
    refresh: string;
    syncing: string;
    download: string;
    delete: string;
    closeDialog: string;
  };
  app: {
    initializing: string;
    dismissStatusNotice: string;
    dismissError: string;
    copiedClipboardSnippet: string;
    openedStreamIn: (player: string) => string;
  };
  titleBar: {
    appTitle: string;
    waitingTrust: string;
    notConnected: string;
    minimizeWindow: string;
    toggleMaximizeWindow: string;
    closeWindow: string;
  };
  sidebar: {
    primaryNavigation: string;
    connectDevice: string;
    tabs: {
      files: string;
      photos: string;
      videos: string;
      music: string;
      clipboard: string;
      settings: string;
    };
    transfers: string;
    disconnect: string;
  };
  storage: {
    ariaLabel: string;
    heading: string;
    battery: (pct: number) => string;
    usedOf: (used: string, total: string) => string;
    freeOf: (free: string, total: string) => string;
    segments: {
      media: string;
      audio: string;
      downloads: string;
      apps: string;
      cache: string;
      other: string;
    };
  };
  welcome: {
    title: string;
    subtitle: string;
    scanning: string;
    scanDevices: string;
    trustTitle: string;
    trustDesc: string;
    wifiHeading: (count: number) => string;
    noWifiDevices: string;
    pairedBadge: string;
    lockedBadge: string;
    connect: string;
    phoneHostAria: string;
    phoneHostPlaceholder: string;
    tcpPortAria: string;
    directConnect: string;
    usbHeading: (count: number) => string;
    usbSwitchConsent: string;
    noUsbDevices: string;
    usbMeta: (bus: number, address: number, ready: boolean) => string;
    usbFallbackName: (vidHex: string, pidHex: string) => string;
    connectUsb: string;
    trustedHeading: (count: number) => string;
    noTrustedDevices: string;
    pairedAt: (date: string) => string;
    unpair: string;
  };
  dropZone: {
    overlay: (targetDir: string) => string;
  };
  files: {
    phoneStorage: string;
    up: string;
    upTitle: string;
    breadcrumbsAria: string;
    filterAria: string;
    filterPlaceholder: string;
    refreshFolderTitle: string;
    newFolder: string;
    uploadFiles: string;
    uploadFolder: string;
    downloadCount: (count: number) => string;
    rename: string;
    delete: string;
    tableCaption: (path: string) => string;
    selectAllAria: string;
    colName: string;
    colSize: string;
    colModified: string;
    loadingDir: string;
    noMatchingFiles: string;
    emptyFolder: string;
    selectEntryAria: (name: string) => string;
    folderType: string;
    newFolderTitle: string;
    folderNamePlaceholder: string;
    create: string;
    renameTitle: string;
    save: string;
    deleteConfirmTitle: (count: number) => string;
    deleteConfirmDesc: string;
    deleteMoreItems: (count: number) => string;
    deletePermCheckbox: string;
    deletePermAction: string;
    moveToTrash: string;
    deleteTrashTargetHint: (trashDir: string) => string;
  };
  photos: {
    allPhotos: (count: number) => string;
    albumsMode: (count: number) => string;
    backToAlbums: string;
    folderPhotoCount: (count: number) => string;
    openAlbumAria: (name: string, count: number) => string;
    emptyAlbums: string;
    loading: string;
    empty: string;
    previewAria: (title: string) => string;
    previewTitleFallback: string;
    prev: string;
    next: string;
  };
  mediaSelect: {
    multiSelect: string;
    exitMultiSelect: string;
    selectedCount: (selected: number, total: number) => string;
    selectAll: string;
    deselectAll: string;
    selectAllAria: string;
    selectItemAria: (name: string) => string;
    downloadCount: (count: number) => string;
    copyToFolder: string;
    deleteCount: (count: number) => string;
    copiedToFolderNotice: (count: number, dir: string) => string;
    selectionBarAria: string;
  };
  videos: {
    heading: (count: number) => string;
    loading: string;
    empty: string;
    playAria: (name: string) => string;
    downloadAria: (name: string) => string;
    playerTitleFallback: string;
    inAppUnavailable: string;
    missingGstreamer: (missing: string[]) => string;
    checkingGstreamer: string;
    openInSystemPlayer: string;
    downloadVideo: string;
  };
  music: {
    heading: (tracks: number, albums: number) => string;
    tableCaption: string;
    colTitle: string;
    colArtist: string;
    colDuration: string;
    colSize: string;
    colActions: string;
    loading: string;
    empty: string;
    playAria: (title: string) => string;
    downloadAria: (title: string) => string;
    missingGstreamerAudio: (missing: string[]) => string;
    audioFallback: string;
    openInSystemPlayer: string;
    closePlayer: string;
  };
  clipboard: {
    heading: (count: number) => string;
    autoSync: string;
    refreshing: string;
    clearAll: string;
    sendAria: string;
    sendPlaceholder: string;
    pushToPhone: string;
    empty: string;
    copy: string;
    deleteAria: string;
    deleteConfirmTitle: string;
    deleteConfirmAction: string;
    clearConfirmTitle: string;
    clearConfirmDesc: string;
    clearHistory: string;
  };
  transfers: {
    queueAria: string;
    heading: (count: number) => string;
    clearTitle: string;
    clear: string;
    closeAria: string;
    empty: string;
    cancelAria: (fileName: string) => string;
    status: {
      queued: string;
      active: string;
      completed: string;
      failed: string;
      cancelled: string;
    };
  };
  settings: {
    title: string;
    windowAppearance: string;
    languageLabel: string;
    languageDesc: string;
    languageOptions: Record<LanguagePreference, string>;
    useSystemDecorations: string;
    tilingWmDesc: string;
    floatingWmDesc: string;
    colorTheme: string;
    colorThemeDesc: string;
    themes: {
      system: string;
      light: string;
      dark: string;
    };
    transfersClipboard: string;
    downloadDir: string;
    chooseFolder: string;
    trashSection: string;
    openTrash: string;
    trashDir: string;
    trashDirInvalid: string;
    clipboardAutoSync: string;
    clipboardAutoSyncDesc: string;
    usbPermissions: string;
    usbPermissionsDesc: string;
  };
  trash: {
    title: string;
    backToSettings: string;
    dirLabel: (dir: string) => string;
    filterPlaceholder: string;
    selectedCount: (count: number) => string;
    restore: string;
    restoreSelected: string;
    deletePermanently: string;
    emptyTrash: string;
    connectToRestore: string;
    emptyTitle: string;
    emptyHint: string;
    noMatches: string;
    columns: {
      selectAll: string;
      name: string;
      originalPath: string;
      sourceDevice: string;
      deletedAt: string;
      size: string;
    };
    unknownPath: string;
    unknownDevice: string;
    currentDeviceBadge: string;
    otherDeviceBadge: string;
    confirmDeleteTitle: string;
    confirmDeleteBody: (count: number) => string;
    confirmEmptyTitle: string;
    confirmEmptyBody: (count: number) => string;
    restoredNotice: (count: number) => string;
    deletedNotice: (count: number) => string;
    emptiedNotice: (count: number) => string;
    restoreWrongDeviceError: (name: string, sourcePhone: string) => string;
    restoreWrongDeviceGeneric: string;
    restoreMissingOriginError: (name: string) => string;
    restoreMissingOriginGeneric: string;
  };
}

const en: I18nMessages = {
  common: {
    cancel: 'Cancel',
    refresh: 'Refresh',
    syncing: 'Syncing…',
    download: 'Download',
    delete: 'Delete',
    closeDialog: 'Close dialog',
  },
  app: {
    initializing: 'Initializing HandShaker…',
    dismissStatusNotice: 'Dismiss status notice',
    dismissError: 'Dismiss error',
    copiedClipboardSnippet: 'Copied snippet to Linux clipboard',
    openedStreamIn: (player) => `Opened stream in ${player}`,
  },
  titleBar: {
    appTitle: 'HandShaker for Linux',
    waitingTrust: 'Waiting for phone trust approval…',
    notConnected: 'Not connected',
    minimizeWindow: 'Minimize window',
    toggleMaximizeWindow: 'Toggle maximize window',
    closeWindow: 'Close window',
  },
  sidebar: {
    primaryNavigation: 'Primary navigation',
    connectDevice: 'Connect Device',
    tabs: {
      files: 'Files',
      photos: 'Photos',
      videos: 'Videos',
      music: 'Music',
      clipboard: 'Clipboard',
      settings: 'Settings',
    },
    transfers: 'Transfers',
    disconnect: 'Disconnect',
  },
  storage: {
    ariaLabel: 'Phone storage usage',
    heading: 'Storage',
    battery: (pct) => `${pct}% bat`,
    usedOf: (used, total) => `${used} used of ${total}`,
    freeOf: (free, total) => `${free} free / ${total}`,
    segments: {
      media: 'Media',
      audio: 'Audio',
      downloads: 'Downloads',
      apps: 'Apps',
      cache: 'Cache',
      other: 'Other',
    },
  },
  welcome: {
    title: 'Connect Your Android Phone',
    subtitle: 'Open HandShaker on your phone over Wi-Fi (`_handshaker_ssp._tcp`) or USB cable.',
    scanning: 'Scanning…',
    scanDevices: 'Scan Devices',
    trustTitle: 'Waiting for Trust confirmation on your phone',
    trustDesc: 'Tap "Trust Always" or "Allow" on the HandShaker prompt shown on your phone screen.',
    wifiHeading: (count) => `Wi-Fi mDNS Devices (${count})`,
    noWifiDevices:
      'No mDNS devices discovered yet. Ensure both devices are on the same LAN and HandShaker is running on Android.',
    pairedBadge: 'Paired',
    lockedBadge: 'Locked',
    connect: 'Connect',
    phoneHostAria: 'Phone IP or hostname',
    phoneHostPlaceholder: 'e.g. 192.168.1.42',
    tcpPortAria: 'TCP port',
    directConnect: 'Direct',
    usbHeading: (count) => `USB AOA Candidates (${count})`,
    usbSwitchConsent:
      'Allow sending AOA mode-switch (`51/52/53`) if phone is not already `18d1:2d00..2d05` (interrupts ADB until replug)',
    noUsbDevices:
      'No Android USB / AOA device detected. Check USB cable & `packaging/udev/70-handshaker.rules`.',
    usbMeta: (bus, address, ready) =>
      `Bus ${bus}:${address} • ${ready ? 'AOA Ready' : 'Pre-switch USB'}`,
    usbFallbackName: (vidHex, pidHex) => `USB ${vidHex}:${pidHex}`,
    connectUsb: 'Connect USB',
    trustedHeading: (count) => `Trusted Paired Phones (${count})`,
    noTrustedDevices:
      'No paired devices saved yet (`trust_store.json`). Pair once via "Trust Always" to skip future confirmation prompts.',
    pairedAt: (date) => `Paired ${date}`,
    unpair: 'Unpair',
  },
  dropZone: {
    overlay: (targetDir) => `Drop Linux files or folders here to upload to ${targetDir}`,
  },
  files: {
    phoneStorage: 'Phone Storage',
    up: 'Up',
    upTitle: 'Go up one folder',
    breadcrumbsAria: 'Folder breadcrumbs',
    filterAria: 'Filter files in folder',
    filterPlaceholder: 'Filter...',
    refreshFolderTitle: 'Refresh folder',
    newFolder: 'New Folder',
    uploadFiles: 'Upload Files',
    uploadFolder: 'Upload Folder',
    downloadCount: (count) => `Download (${count})`,
    rename: 'Rename',
    delete: 'Delete',
    tableCaption: (path) => `Android files in ${path}`,
    selectAllAria: 'Select all entries',
    colName: 'Name',
    colSize: 'Size',
    colModified: 'Modified',
    loadingDir: 'Loading directory…',
    noMatchingFiles: 'No matching files.',
    emptyFolder: 'This folder is empty.',
    selectEntryAria: (name) => `Select ${name}`,
    folderType: 'Folder',
    newFolderTitle: 'Create New Folder',
    folderNamePlaceholder: 'Folder name',
    create: 'Create',
    renameTitle: 'Rename Item',
    save: 'Save',
    deleteConfirmTitle: (count) => `Delete ${count} item(s)?`,
    deleteConfirmDesc:
      'Move selected items to HandShaker trash on Linux (files/ + .trashinfo), or delete permanently from the phone.',
    deleteMoreItems: (count) => `…and ${count} more item(s)`,
    deletePermCheckbox: 'Delete permanently instead of moving to trash',
    deletePermAction: 'Delete Permanently',
    moveToTrash: 'Move to Trash',
    deleteTrashTargetHint: (trashDir) =>
      `Items will be moved to "${trashDir}" on Linux with .trashinfo metadata before removal from phone.`,
  },
  photos: {
    allPhotos: (count) => `All Photos (${count})`,
    albumsMode: (count) => `Albums (${count})`,
    backToAlbums: 'Back to Albums',
    folderPhotoCount: (count) => `${count} photo(s)`,
    openAlbumAria: (name, count) => `Open album ${name} (${count} photos)`,
    emptyAlbums: 'No photo folders found on phone.',
    loading: 'Loading photos…',
    empty: 'No photos found in this album.',
    previewAria: (title) => `Preview ${title}`,
    previewTitleFallback: 'Photo Preview',
    prev: 'Prev',
    next: 'Next',
  },
  mediaSelect: {
    multiSelect: 'Multi-select',
    exitMultiSelect: 'Done',
    selectedCount: (selected, total) => `${selected} of ${total} selected`,
    selectAll: 'Select All',
    deselectAll: 'Deselect All',
    selectAllAria: 'Select all items',
    selectItemAria: (name) => `Select ${name}`,
    downloadCount: (count) => `Download (${count})`,
    copyToFolder: 'Copy to…',
    deleteCount: (count) => `Delete (${count})`,
    copiedToFolderNotice: (count, dir) => `Queued ${count} file(s) to copy to ${dir}`,
    selectionBarAria: 'Multi-select actions',
  },
  videos: {
    heading: (count) => `Videos (${count})`,
    loading: 'Loading videos…',
    empty: 'No videos found on phone.',
    playAria: (name) => `Play ${name}`,
    downloadAria: (name) => `Download ${name}`,
    playerTitleFallback: 'Video Player',
    inAppUnavailable: 'In-app video playback unavailable',
    missingGstreamer: (missing) =>
      `In-app playback is disabled to prevent WebKitGTK crash because GStreamer element(s) are missing: ${missing.join(', ')}. Install gst-plugins-good (and gst-libav for extra codecs), or use Open in System Player / Download.`,
    checkingGstreamer:
      'Checking GStreamer playback support. Install gst-plugins-good for embedded playback, or open in a system player.',
    openInSystemPlayer: 'Open in System Player',
    downloadVideo: 'Download Video',
  },
  music: {
    heading: (tracks, albums) => `Music (${tracks} tracks • ${albums} albums)`,
    tableCaption: 'Android music tracks',
    colTitle: 'Title',
    colArtist: 'Artist',
    colDuration: 'Duration',
    colSize: 'Size',
    colActions: 'Actions',
    loading: 'Loading audio library…',
    empty: 'No music files found on phone.',
    playAria: (title) => `Play ${title}`,
    downloadAria: (title) => `Download ${title}`,
    missingGstreamerAudio: (missing) =>
      `In-app audio disabled to prevent WebKitGTK crash because GStreamer element(s) are missing: ${missing.join(', ')}. Open externally or download.`,
    audioFallback:
      'In-app audio disabled (missing GStreamer gst-plugins-good). Open externally or download.',
    openInSystemPlayer: 'Open in System Player',
    closePlayer: 'Close player',
  },
  clipboard: {
    heading: (count) => `Clipboard Sync (${count})`,
    autoSync: 'Two-way auto sync (Wayland / X11)',
    refreshing: 'Refreshing…',
    clearAll: 'Clear All',
    sendAria: 'Send clipboard note to phone',
    sendPlaceholder: 'Write or paste text to push immediately to Android HandShaker…',
    pushToPhone: 'Push to Phone',
    empty:
      'Clipboard history is empty. Copy text on Linux (with auto-sync enabled) or push a note above.',
    copy: 'Copy',
    deleteAria: 'Delete clipboard note',
    deleteConfirmTitle: 'Delete Clipboard Note?',
    deleteConfirmAction: 'Delete',
    clearConfirmTitle: 'Clear Phone Clipboard History?',
    clearConfirmDesc:
      'This deletes all synced clipboard notes stored by HandShaker on your phone.',
    clearHistory: 'Clear History',
  },
  transfers: {
    queueAria: 'Transfer queue',
    heading: (count) => `Transfers (${count})`,
    clearTitle: 'Clear completed, failed, and cancelled items',
    clear: 'Clear',
    closeAria: 'Close transfer queue',
    empty: 'No active or recent transfers.',
    cancelAria: (fileName) => `Cancel ${fileName}`,
    status: {
      queued: 'queued',
      active: 'active',
      completed: 'completed',
      failed: 'failed',
      cancelled: 'cancelled',
    },
  },
  settings: {
    title: 'Settings',
    windowAppearance: 'Window & Appearance',
    languageLabel: 'Language',
    languageDesc: 'Follow desktop system locale or switch between Simplified Chinese and English.',
    languageOptions: {
      system: 'System',
      'zh-CN': '简体中文',
      en: 'English',
    },
    useSystemDecorations: 'Use system window titlebar (OS decorations)',
    tilingWmDesc:
      'Tiling compositor detected (Hyprland/Sway/i3): integrated header bar recommended to avoid double borders.',
    floatingWmDesc:
      'Switch live between the integrated HandShaker bar and native desktop window decorations.',
    colorTheme: 'Color theme',
    colorThemeDesc: 'Follows desktop system preference or forces light / dark appearance.',
    themes: {
      system: 'System',
      light: 'Light',
      dark: 'Dark',
    },
    transfersClipboard: 'Transfers & Clipboard',
    downloadDir: 'Default download folder',
    chooseFolder: 'Choose…',
    trashSection: 'Trash',
    openTrash: 'Trash',
    trashDir: 'Linux trash directory',
    trashDirInvalid:
      'Invalid Linux trash directory: choose a dedicated subfolder rather than /, /home, or your home directory.',
    clipboardAutoSync: 'Bidirectional clipboard auto-sync',
    clipboardAutoSyncDesc:
      'Uses `arboard` (`wayland-data-control` on Hyprland/Wayland) with echo suppression.',
    usbPermissions: 'USB AOA Linux Permissions',
    usbPermissionsDesc:
      'To connect over USB cable without root, install the included udev rule once:',
  },
  trash: {
    title: 'Trash',
    backToSettings: 'Back to Settings',
    dirLabel: (dir) => `Directory: ${dir}`,
    filterPlaceholder: 'Filter trash items…',
    selectedCount: (count) => `${count} selected`,
    restore: 'Restore',
    restoreSelected: 'Restore Selected',
    deletePermanently: 'Delete Permanently',
    emptyTrash: 'Empty Trash',
    connectToRestore: 'Connect phone to restore files back to their original paths.',
    emptyTitle: 'Trash is empty',
    emptyHint: 'Files moved to Trash from your phone will appear here.',
    noMatches: 'No matching items in Trash',
    columns: {
      selectAll: 'Select all items in Trash',
      name: 'Name',
      originalPath: 'Original Phone Path',
      sourceDevice: 'Source Phone',
      deletedAt: 'Deleted At',
      size: 'Size',
    },
    unknownPath: 'Unknown original path (missing metadata)',
    unknownDevice: 'Unknown phone',
    currentDeviceBadge: 'This phone',
    otherDeviceBadge: 'Other phone',
    confirmDeleteTitle: 'Delete permanently?',
    confirmDeleteBody: (count) =>
      `Permanently remove ${count} selected item(s) from Linux Trash? This cannot be undone.`,
    confirmEmptyTitle: 'Empty Trash?',
    confirmEmptyBody: (count) =>
      `Permanently remove all ${count} item(s) from Linux Trash? This cannot be undone.`,
    restoredNotice: (count) => `Restored ${count} item(s) to phone`,
    deletedNotice: (count) => `Permanently deleted ${count} item(s)`,
    emptiedNotice: (count) => `Emptied Trash (${count} item(s) removed)`,
    restoreWrongDeviceError: (name, sourcePhone) =>
      `Cannot restore "${name}": this file belongs to "${sourcePhone}", not the currently connected phone.`,
    restoreWrongDeviceGeneric:
      'Selected file does not belong to the currently connected phone and cannot be restored.',
    restoreMissingOriginError: (name) =>
      `Cannot restore "${name}": source phone metadata is missing, so its origin cannot be verified.`,
    restoreMissingOriginGeneric:
      'Source phone metadata is missing, so the original phone cannot be verified for restore.',
  },
};

const zhCN: I18nMessages = {
  common: {
    cancel: '取消',
    refresh: '刷新',
    syncing: '同步中…',
    download: '下载',
    delete: '删除',
    closeDialog: '关闭弹窗',
  },
  app: {
    initializing: '正在初始化 HandShaker…',
    dismissStatusNotice: '关闭状态提示',
    dismissError: '关闭错误提示',
    copiedClipboardSnippet: '已复制到 Linux 剪贴板',
    openedStreamIn: (player) => `已在 ${player} 中打开播放`,
  },
  titleBar: {
    appTitle: 'HandShaker for Linux',
    waitingTrust: '等待手机端信任授权…',
    notConnected: '未连接',
    minimizeWindow: '最小化窗口',
    toggleMaximizeWindow: '切换最大化窗口',
    closeWindow: '关闭窗口',
  },
  sidebar: {
    primaryNavigation: '主导航',
    connectDevice: '连接设备',
    tabs: {
      files: '文件',
      photos: '照片',
      videos: '视频',
      music: '音乐',
      clipboard: '剪贴板',
      settings: '设置',
    },
    transfers: '传输队列',
    disconnect: '断开连接',
  },
  storage: {
    ariaLabel: '手机存储空间占用',
    heading: '存储空间',
    battery: (pct) => `电量 ${pct}%`,
    usedOf: (used, total) => `已用 ${used} / 共 ${total}`,
    freeOf: (free, total) => `可用 ${free} / 共 ${total}`,
    segments: {
      media: '媒体',
      audio: '音频',
      downloads: '下载',
      apps: '应用',
      cache: '缓存',
      other: '其他',
    },
  },
  welcome: {
    title: '连接你的 Android 手机',
    subtitle: '在手机端打开 HandShaker，通过 Wi-Fi（_handshaker_ssp._tcp）或 USB 数据线连接。',
    scanning: '正在扫描…',
    scanDevices: '扫描设备',
    trustTitle: '等待手机端信任确认',
    trustDesc: '请在手机屏幕弹出的 HandShaker 授权提示中点击“始终信任”或“允许”。',
    wifiHeading: (count) => `Wi-Fi 局域网设备 (${count})`,
    noWifiDevices: '暂未发现局域网设备。请确保手机与电脑处于同一局域网，且手机端已打开 HandShaker。',
    pairedBadge: '已配对',
    lockedBadge: '已锁屏',
    connect: '连接',
    phoneHostAria: '手机 IP 或主机名',
    phoneHostPlaceholder: '例如 192.168.1.42',
    tcpPortAria: 'TCP 端口',
    directConnect: '直连',
    usbHeading: (count) => `USB AOA 设备 (${count})`,
    usbSwitchConsent:
      '若手机尚未处于 18d1:2d00..2d05 模式，允许发送 AOA 模式切换指令（51/52/53，重插前会中断 ADB）',
    noUsbDevices:
      '未检测到 Android USB / AOA 设备，请检查数据线与 udev 规则（packaging/udev/70-handshaker.rules）。',
    usbMeta: (bus, address, ready) =>
      `总线 ${bus}:${address} · ${ready ? 'AOA 就绪' : '待切换 USB'}`,
    usbFallbackName: (vidHex, pidHex) => `USB ${vidHex}:${pidHex}`,
    connectUsb: 'USB 连接',
    trustedHeading: (count) => `已信任的配对手机 (${count})`,
    noTrustedDevices:
      '暂无已保存的配对设备（trust_store.json）。首次配对选择“始终信任”后可免确认直连。',
    pairedAt: (date) => `配对于 ${date}`,
    unpair: '取消配对',
  },
  dropZone: {
    overlay: (targetDir) => `松开即可将 Linux 文件或文件夹上传至 ${targetDir}`,
  },
  files: {
    phoneStorage: '手机存储',
    up: '上级',
    upTitle: '返回上一级文件夹',
    breadcrumbsAria: '当前文件夹路径',
    filterAria: '筛选当前文件夹中的文件',
    filterPlaceholder: '筛选文件名…',
    refreshFolderTitle: '刷新文件夹',
    newFolder: '新建文件夹',
    uploadFiles: '上传文件',
    uploadFolder: '上传文件夹',
    downloadCount: (count) => `下载 (${count})`,
    rename: '重命名',
    delete: '删除',
    tableCaption: (path) => `${path} 中的手机文件`,
    selectAllAria: '全选当前列表',
    colName: '名称',
    colSize: '大小',
    colModified: '修改时间',
    loadingDir: '正在加载目录…',
    noMatchingFiles: '没有匹配的文件。',
    emptyFolder: '当前文件夹为空。',
    selectEntryAria: (name) => `选择 ${name}`,
    folderType: '文件夹',
    newFolderTitle: '新建文件夹',
    folderNamePlaceholder: '请输入文件夹名称',
    create: '创建',
    renameTitle: '重命名',
    save: '保存',
    deleteConfirmTitle: (count) => `确定删除选中的 ${count} 项吗？`,
    deleteConfirmDesc: '将所选项目移至 Linux 端 HandShaker 回收站，或直接从手机永久删除。',
    deleteMoreItems: (count) => `…以及其余 ${count} 项`,
    deletePermCheckbox: '直接永久删除（不放入回收站）',
    deletePermAction: '永久删除',
    moveToTrash: '移至回收站',
    deleteTrashTargetHint: (trashDir) =>
      `将先移至 Linux 目录「${trashDir}」（记录 .trashinfo 元数据），随后从手机删除原文件。`,
  },
  photos: {
    allPhotos: (count) => `全部照片 (${count})`,
    albumsMode: (count) => `相册 (${count})`,
    backToAlbums: '返回相册',
    folderPhotoCount: (count) => `${count} 张`,
    openAlbumAria: (name, count) => `打开相册 ${name}（${count} 张照片）`,
    emptyAlbums: '手机上暂无相册文件夹。',
    loading: '正在加载照片…',
    empty: '当前相册中暂无照片。',
    previewAria: (title) => `预览 ${title}`,
    previewTitleFallback: '照片预览',
    prev: '上一张',
    next: '下一张',
  },
  mediaSelect: {
    multiSelect: '多选',
    exitMultiSelect: '完成',
    selectedCount: (selected, total) => `已选择 ${selected} / ${total} 项`,
    selectAll: '全选',
    deselectAll: '取消全选',
    selectAllAria: '全选当前项目',
    selectItemAria: (name) => `选择 ${name}`,
    downloadCount: (count) => `下载 (${count})`,
    copyToFolder: '复制到…',
    deleteCount: (count) => `删除 (${count})`,
    copiedToFolderNotice: (count, dir) => `已将 ${count} 个文件加入复制队列，目标目录：${dir}`,
    selectionBarAria: '多选操作栏',
  },
  videos: {
    heading: (count) => `视频 (${count})`,
    loading: '正在加载视频…',
    empty: '手机上暂无视频。',
    playAria: (name) => `播放 ${name}`,
    downloadAria: (name) => `下载 ${name}`,
    playerTitleFallback: '视频播放器',
    inAppUnavailable: '应用内视频播放暂不可用',
    missingGstreamer: (missing) =>
      `为防止 WebKitGTK 崩溃，已禁用应用内播放（缺少 GStreamer 组件：${missing.join(', ')}）。请安装 gst-plugins-good（需要更多解码器可一并安装 gst-libav），或使用系统播放器打开 / 下载。`,
    checkingGstreamer:
      '正在检测 GStreamer 播放支持。请安装 gst-plugins-good 以启用内嵌播放，或使用系统播放器打开。',
    openInSystemPlayer: '在系统播放器中打开',
    downloadVideo: '下载视频',
  },
  music: {
    heading: (tracks, albums) => `音乐（${tracks} 首歌曲 · ${albums} 张专辑）`,
    tableCaption: '手机音乐列表',
    colTitle: '标题',
    colArtist: '艺术家',
    colDuration: '时长',
    colSize: '大小',
    colActions: '操作',
    loading: '正在加载音乐库…',
    empty: '手机上暂无音乐文件。',
    playAria: (title) => `播放 ${title}`,
    downloadAria: (title) => `下载 ${title}`,
    missingGstreamerAudio: (missing) =>
      `为防止 WebKitGTK 崩溃，已禁用应用内音频播放（缺少 GStreamer 组件：${missing.join(', ')}）。请使用系统播放器打开或下载。`,
    audioFallback: '应用内音频播放已禁用（缺少 GStreamer gst-plugins-good），请外部打开或下载。',
    openInSystemPlayer: '在系统播放器中打开',
    closePlayer: '关闭播放器',
  },
  clipboard: {
    heading: (count) => `剪贴板同步 (${count})`,
    autoSync: '双向自动同步（Wayland / X11）',
    refreshing: '刷新中…',
    clearAll: '全部清空',
    sendAria: '发送剪贴板文本到手机',
    sendPlaceholder: '输入或粘贴文本，立即推送到手机端 HandShaker…',
    pushToPhone: '发送到手机',
    empty: '剪贴板历史为空。开启自动同步后在 Linux 中复制文本，或在上方输入内容发送。',
    copy: '复制',
    deleteAria: '删除此条剪贴板记录',
    deleteConfirmTitle: '确定删除这条剪贴板记录吗？',
    deleteConfirmAction: '确认删除',
    clearConfirmTitle: '清空手机剪贴板历史？',
    clearConfirmDesc: '此操作会删除手机端 HandShaker 中保存的所有已同步剪贴板记录。',
    clearHistory: '确认清空',
  },
  transfers: {
    queueAria: '传输队列',
    heading: (count) => `传输队列 (${count})`,
    clearTitle: '清除已完成、失败和已取消的任务',
    clear: '清理',
    closeAria: '关闭传输队列',
    empty: '暂无进行中或最近的传输任务。',
    cancelAria: (fileName) => `取消 ${fileName}`,
    status: {
      queued: '排队中',
      active: '传输中',
      completed: '已完成',
      failed: '失败',
      cancelled: '已取消',
    },
  },
  settings: {
    title: '设置',
    windowAppearance: '窗口与外观',
    languageLabel: '界面语言',
    languageDesc: '跟随桌面系统语言，或在简体中文与 English 之间切换。',
    languageOptions: {
      system: '跟随系统',
      'zh-CN': '简体中文',
      en: 'English',
    },
    useSystemDecorations: '使用系统窗口标题栏（原生窗口边框）',
    tilingWmDesc:
      '检测到平铺式窗口管理器（Hyprland/Sway/i3）：建议使用应用集成标题栏以避免双层边框。',
    floatingWmDesc: '实时切换 HandShaker 集成标题栏与系统原生窗口装饰。',
    colorTheme: '主题配色',
    colorThemeDesc: '跟随桌面系统深浅色偏好，或固定使用浅色 / 深色外观。',
    themes: {
      system: '跟随系统',
      light: '浅色',
      dark: '深色',
    },
    transfersClipboard: '传输与剪贴板',
    downloadDir: '默认下载目录',
    chooseFolder: '选择目录…',
    trashSection: '回收站',
    openTrash: '回收站',
    trashDir: 'Linux 端回收站目录',
    trashDirInvalid: '回收站目录无效：请选择专用子目录，不能为根目录或用户主目录本身。',
    clipboardAutoSync: '双向剪贴板自动同步',
    clipboardAutoSyncDesc:
      '基于 arboard（在 Hyprland/Wayland 下支持 wayland-data-control）自动同步并抑制回环。',
    usbPermissions: 'Linux USB AOA 权限配置',
    usbPermissionsDesc: '如需免 root 权限通过 USB 数据线连接手机，请执行一次以下命令安装内置 udev 规则：',
  },
  trash: {
    title: '回收站',
    backToSettings: '返回设置',
    dirLabel: (dir) => `目录：${dir}`,
    filterPlaceholder: '筛选回收站项目…',
    selectedCount: (count) => `已选 ${count} 项`,
    restore: '恢复',
    restoreSelected: '恢复选中',
    deletePermanently: '彻底删除',
    emptyTrash: '清空回收站',
    connectToRestore: '请先连接手机，才可将回收站文件恢复至手机原路径。',
    emptyTitle: '回收站为空',
    emptyHint: '从手机删除并移至回收站的文件和文件夹会显示在这里。',
    noMatches: '没有匹配的回收站项目',
    columns: {
      selectAll: '全选回收站项目',
      name: '名称',
      originalPath: '手机原路径',
      sourceDevice: '来源手机',
      deletedAt: '删除时间',
      size: '大小',
    },
    unknownPath: '未知原路径（缺少元数据）',
    unknownDevice: '未知手机',
    currentDeviceBadge: '当前手机',
    otherDeviceBadge: '非当前手机',
    confirmDeleteTitle: '彻底删除所选项目？',
    confirmDeleteBody: (count) =>
      `将从 Linux 回收站永久删除所选的 ${count} 项，此操作不可撤销。`,
    confirmEmptyTitle: '确认清空回收站？',
    confirmEmptyBody: (count) =>
      `将从 Linux 回收站永久删除全部 ${count} 项，此操作不可撤销。`,
    restoredNotice: (count) => `已恢复 ${count} 项到手机`,
    deletedNotice: (count) => `已彻底删除 ${count} 项`,
    emptiedNotice: (count) => `已清空回收站（共移除 ${count} 项）`,
    restoreWrongDeviceError: (name, sourcePhone) =>
      `文件「${name}」来自手机「${sourcePhone}」，不是从当前连接的手机删除的，无法恢复。`,
    restoreWrongDeviceGeneric: '文件不是从当前连接的手机删除的，无法恢复。',
    restoreMissingOriginError: (name) =>
      `文件「${name}」缺少来源手机信息，无法确认来自当前手机，无法恢复。`,
    restoreMissingOriginGeneric: '文件缺少来源手机信息，无法确认来自当前手机，无法恢复。',
  },
};

const MESSAGES: Record<ResolvedLanguage, I18nMessages> = {
  en,
  'zh-CN': zhCN,
};

export function getMessages(lang: ResolvedLanguage): I18nMessages {
  return MESSAGES[lang];
}
