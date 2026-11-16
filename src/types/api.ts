export type ThemePreference = 'system' | 'light' | 'dark';
export type LanguagePreference = 'system' | 'en' | 'zh-CN';

export interface AppSettings {
  useSystemDecorations: boolean;
  theme: ThemePreference;
  language: LanguagePreference;
  downloadDir: string;
  trashDir: string;
  clipboardAutoSync: boolean;
  isTilingWm: boolean;
}

export interface AppSettingsPatch {
  useSystemDecorations?: boolean;
  theme?: ThemePreference;
  language?: LanguagePreference;
  downloadDir?: string;
  trashDir?: string;
  clipboardAutoSync?: boolean;
}

export interface WifiCandidate {
  serviceName: string;
  displayName: string;
  host: string;
  port: number;
  addresses: string[];
  hostname: string | null;
  deviceUuid: string | null;
  deviceName: string | null;
  phoneModel: string | null;
  productBrand: string | null;
  productManufacturer: string | null;
  apkVersionName: string | null;
  isTrusted: boolean;
  isLocked: boolean;
}

export interface UsbCandidate {
  bus: number;
  address: number;
  vendorId: number;
  productId: number;
  manufacturer: string | null;
  product: string | null;
  serial: string | null;
  alreadyAoa: boolean;
  interfaceCount: number;
}

export interface DiscoverySnapshot {
  wifi: WifiCandidate[];
  usb: UsbCandidate[];
}

export interface TrustedDevice {
  deviceUuid: string;
  deviceName: string;
  pairedAtUnix: number;
  phoneModel: string | null;
  productBrand: string | null;
  productManufacturer: string | null;
}

export interface HandshakeInfo {
  deviceUuid: string;
  deviceName: string;
  apkVersion: string;
  apkVersionName: string;
  trustType: number;
  transport: string;
  transportLabel: string;
  isSmartisanDevice: boolean;
}

export interface ConnectionState {
  status: 'disconnected' | 'connecting' | 'trust_waiting' | 'connected';
  handshake: HandshakeInfo | null;
  error: string | null;
}

export interface DeviceInfo {
  phoneName: string;
  phoneModel: string;
  productBrand: string;
  productManufacturer: string;
  apkVersionName: string;
  rootPath: string;
  externalStoragePath: string | null;
  batteryPercentage: number;
  diskSize: number;
  usedDiskSize: number;
  freeDiskSize: number;
  picVideoSize: number;
  audioSize: number;
  downloadSize: number;
  appSize: number;
  cacheSize: number;
  otherSize: number;
}

export interface FileEntry {
  name: string;
  path: string;
  isDirectory: boolean;
  fileSize: number;
  createdTimestamp: number;
  modifiedTimestamp: number;
  isTrash: boolean;
}

export interface DownloadTarget {
  remotePath: string;
  isDirectory: boolean;
  fileSize: number;
}

export interface TransferItem {
  id: string;
  direction: 'download' | 'upload';
  fileName: string;
  localPath: string;
  remotePath: string;
  totalBytes: number;
  transferredBytes: number;
  status: 'queued' | 'active' | 'completed' | 'failed' | 'cancelled';
  error: string | null;
  createdAtMs: number;
}

export interface PhotoAlbum {
  albumId: number;
  albumName: string;
  albumPath: string;
  coverThumbUrl: string | null;
}

export interface ImageItem {
  mediaId: number;
  albumId: number;
  albumName: string;
  path: string;
  title: string;
  mimeType: string;
  fileSize: number;
  width: number;
  height: number;
  orientation: number;
  dateTaken: number;
  modifiedTimestamp: number;
  starred: boolean;
  thumbUrl: string;
  streamUrl: string;
}

export interface PhotoLibrary {
  cameraAlbumId: number | null;
  albums: PhotoAlbum[];
  images: ImageItem[];
}

export interface VideoAlbum {
  albumId: number;
  albumName: string;
  albumPath: string;
}

export interface VideoItem {
  mediaId: number;
  albumId: number;
  path: string;
  name: string;
  mimeType: string;
  fileSize: number;
  width: number;
  height: number;
  durationSecs: number;
  modifiedTimestamp: number;
  thumbUrl: string;
  streamUrl: string;
}

export interface VideoLibrary {
  albums: VideoAlbum[];
  videos: VideoItem[];
}

export interface AudioAlbum {
  albumId: number;
  albumName: string;
  albumPath: string;
  artist: string;
  year: number;
  thumbUrl: string;
}

export interface AudioItem {
  mediaId: number;
  albumId: number;
  path: string;
  title: string;
  artist: string;
  mimeType: string;
  fileSize: number;
  durationSecs: number;
  bitrate: number;
  sampleRate: number;
  year: number;
  track: number;
  streamUrl: string;
}

export interface AudioLibrary {
  albums: AudioAlbum[];
  tracks: AudioItem[];
}

export interface ClipboardItem {
  text: string;
  timestampMs: number;
  rawGzipB64: string;
}

export interface MediaPlaybackSupport {
  embeddedAudio: boolean;
  embeddedVideo: boolean;
  missingElements: string[];
  reason: string | null;
}

export interface TrashItem {
  entryName: string;
  isDirectory: boolean;
  sizeBytes: number;
  originalPath: string | null;
  deletionDate: string | null;
  deviceUuid: string | null;
  deviceName: string | null;
  deviceModel: string | null;
  modifiedTimestampMs: number;
}
