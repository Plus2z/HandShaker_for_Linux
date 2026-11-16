import { StrictMode } from 'react';
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  formatBytes,
  formatDuration,
  parentRemotePath,
  splitFilenameExt,
} from '../lib/format';
import { resolveLanguage } from '../lib/i18n';
import { sortFileEntries, useAppStore } from '../store/useAppStore';

const webviewMock = vi.hoisted(() => ({
  dragHandlers: new Set<(ev: { payload: unknown }) => void>(),
  deferredResolvers: [] as Array<() => void>,
  deferNext: false,
  onDragDropEvent: vi.fn((cb: (ev: { payload: unknown }) => void) => {
    webviewMock.dragHandlers.add(cb);
    const unlisten = () => {
      webviewMock.dragHandlers.delete(cb);
    };
    if (webviewMock.deferNext) {
      return new Promise<() => void>((resolve) => {
        webviewMock.deferredResolvers.push(() => resolve(unlisten));
      });
    }
    return Promise.resolve(unlisten);
  }),
}));

vi.mock('@tauri-apps/api/webview', () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: webviewMock.onDragDropEvent,
  }),
}));

const mocks = vi.hoisted(() => ({
  getSettings: vi.fn(),
  updateSettings: vi.fn(),
  windowMinimize: vi.fn(),
  windowToggleMaximize: vi.fn(),
  windowClose: vi.fn(),
  scanDevices: vi.fn(),
  listTrustedDevices: vi.fn(),
  removeTrustedDevice: vi.fn(),
  getConnectionState: vi.fn(),
  connectWifi: vi.fn(),
  connectUsb: vi.fn(),
  disconnectSession: vi.fn(),
  getDeviceInfo: vi.fn(),
  listDir: vi.fn(),
  createFolder: vi.fn(),
  renameFile: vi.fn(),
  deleteFiles: vi.fn(),
  listTrashItems: vi.fn(),
  restoreTrashItems: vi.fn(),
  deleteTrashItems: vi.fn(),
  emptyTrash: vi.fn(),
  listTransfers: vi.fn(),
  enqueueDownload: vi.fn(),
  enqueueUpload: vi.fn(),
  cancelTransfer: vi.fn(),
  clearFinishedTransfers: vi.fn(),
  getPhotoLibrary: vi.fn(),
  getVideoLibrary: vi.fn(),
  getAudioLibrary: vi.fn(),
  getMediaPlaybackSupport: vi.fn(),
  openMediaExternally: vi.fn(),
  getClipboardHistory: vi.fn(),
  postClipboardText: vi.fn(),
  clearClipboardHistory: vi.fn(),
  deleteClipboardItem: vi.fn(),
  copyToLinuxClipboard: vi.fn(),
  pickLocalPaths: vi.fn(),
  listenConnection: vi.fn(async () => () => {}),
  listenTransfers: vi.fn(async () => () => {}),
  listenClipboard: vi.fn(async () => () => {}),
  listenLibraryChange: vi.fn(async () => () => {}),
  listenFolderChange: vi.fn(async () => () => {}),
}));

vi.mock('../lib/ipc', () => ({ ipc: mocks }));

import App from '../App';
import { ClipboardView } from '../components/ClipboardView';
import { FilesView } from '../components/FilesView';
import { MusicView } from '../components/MusicView';
import { PhotosView } from '../components/PhotosView';
import { VideosView } from '../components/VideosView';

describe('format & sort utilities', () => {
  it('formats sizes, durations, filename extensions, and remote parent paths', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(1536)).toBe('1.50 KB');
    expect(formatDuration(125)).toBe('2:05');
    expect(formatDuration(3661)).toBe('1:01:01');
    expect(splitFilenameExt('quarterly-report.pdf', false)).toEqual(['quarterly-report', '.pdf']);
    expect(splitFilenameExt('.bashrc', false)).toEqual(['.bashrc', '']);
    expect(splitFilenameExt('DCIM.dir', true)).toEqual(['DCIM.dir', '']);
    expect(parentRemotePath('/storage/emulated/0/DCIM/Camera', '/storage/emulated/0')).toBe(
      '/storage/emulated/0/DCIM',
    );
    expect(parentRemotePath('/storage/emulated/0', '/storage/emulated/0')).toBe('/storage/emulated/0');
  });

  it('keeps directories before files and uses natural numeric ordering', () => {
    const entries = [
      {
        name: 'img_10.jpg',
        path: '/img_10.jpg',
        isDirectory: false,
        fileSize: 20,
        createdTimestamp: 1,
        modifiedTimestamp: 2,
        isTrash: false,
      },
      {
        name: 'img_2.jpg',
        path: '/img_2.jpg',
        isDirectory: false,
        fileSize: 10,
        createdTimestamp: 1,
        modifiedTimestamp: 1,
        isTrash: false,
      },
      {
        name: 'DCIM',
        path: '/DCIM',
        isDirectory: true,
        fileSize: 0,
        createdTimestamp: 1,
        modifiedTimestamp: 1,
        isTrash: false,
      },
    ];
    const sorted = sortFileEntries(entries, 'name', 'asc', '');
    expect(sorted.map((e) => e.name)).toEqual(['DCIM', 'img_2.jpg', 'img_10.jpg']);
  });
});

describe('App workflows', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    webviewMock.dragHandlers.clear();
    webviewMock.deferredResolvers = [];
    webviewMock.deferNext = false;
    useAppStore.setState({
      initialized: false,
      tab: 'files',
      settings: {
        useSystemDecorations: false,
        theme: 'system',
        language: 'en',
        downloadDir: '',
        trashDir: '~/.HandShakerTrash',
        clipboardAutoSync: true,
        isTilingWm: false,
      },
      connection: { status: 'disconnected', handshake: null, error: null },
      discovery: { wifi: [], usb: [] },
      trustedDevices: [],
      deviceInfo: null,
      files: [],
      selectedPaths: [],
      transfers: [],
      transferDrawerOpen: false,
      photoLibrary: null,
      quickLookIndex: null,
      videoLibrary: null,
      activeVideo: null,
      audioLibrary: null,
      activeAudio: null,
      mediaSupport: null,
      clipboard: [],
      toastError: null,
      statusNotice: null,
    });

    mocks.getMediaPlaybackSupport.mockResolvedValue({
      embeddedAudio: true,
      embeddedVideo: true,
      missingElements: [],
      reason: null,
    });
    mocks.getSettings.mockResolvedValue({
      useSystemDecorations: false,
      theme: 'system',
      language: 'en',
      downloadDir: '/home/howard/Downloads',
      trashDir: '/home/howard/.HandShakerTrash',
      clipboardAutoSync: true,
      isTilingWm: true,
    });
    mocks.scanDevices.mockResolvedValue({
      wifi: [
        {
          serviceName: 'handshaker_ssp_ (3)._handshaker_ssp._tcp.local.',
          displayName: 'vivo X200 Pro mini',
          host: '192.168.0.103',
          port: 46647,
          addresses: ['192.168.0.103'],
          hostname: null,
          deviceUuid: 'bfd208a26332f56e',
          deviceName: 'vivo X200 Pro mini',
          phoneModel: 'V2419A',
          productBrand: 'vivo',
          productManufacturer: 'vivo',
          apkVersionName: '1.3.1.28',
          isTrusted: true,
          isLocked: false,
        },
        {
          serviceName: 'handshaker_ssp_._handshaker_ssp._tcp.local.',
          displayName: 'OnePlus 7Pro',
          host: '192.168.0.101',
          port: 37831,
          addresses: ['192.168.0.101'],
          hostname: 'oneplus-7pro',
          deviceUuid: 'op-uuid-1',
          deviceName: 'OnePlus 7Pro',
          phoneModel: 'GM1917',
          productBrand: 'OnePlus',
          productManufacturer: 'OnePlus',
          apkVersionName: '2.5.0',
          isTrusted: false,
          isLocked: false,
        },
      ],
      usb: [
        {
          bus: 1,
          address: 6,
          vendorId: 0x2d95,
          productId: 0x6002,
          manufacturer: 'vivo',
          product: 'vivo X200 Pro mini',
          serial: 'ABC',
          alreadyAoa: false,
          interfaceCount: 2,
        },
      ],
    });
    mocks.listTrustedDevices.mockResolvedValue([
      {
        deviceUuid: 'bfd208a26332f56e',
        deviceName: 'vivo X200 Pro mini',
        pairedAtUnix: 1700000000,
        phoneModel: 'V2419A',
        productBrand: 'vivo',
        productManufacturer: 'vivo',
      },
    ]);
    mocks.listTransfers.mockResolvedValue([]);
    mocks.listTrashItems.mockResolvedValue([]);
    mocks.restoreTrashItems.mockResolvedValue([]);
    mocks.deleteTrashItems.mockResolvedValue(0);
    mocks.emptyTrash.mockResolvedValue(0);
    mocks.getConnectionState.mockResolvedValue({
      status: 'disconnected',
      handshake: null,
      error: null,
    });
  });

  it('renders discovery, enforces USB switch consent, and toggles settings decorations', async () => {
    mocks.updateSettings.mockResolvedValue({
      useSystemDecorations: true,
      theme: 'dark',
      language: 'en',
      downloadDir: '/home/howard/Downloads',
      trashDir: '/home/howard/.HandShakerTrash',
      clipboardAutoSync: true,
      isTilingWm: true,
    });

    useAppStore.setState({ initialized: false, tab: 'files' });
    render(<App />);
    await waitFor(() =>
      expect(screen.getByText('vivo · V2419A · 192.168.0.103:46647 · HandShaker v1.3.1.28')).toBeInTheDocument(),
    );
    expect(screen.getByText('OnePlus 7Pro')).toBeInTheDocument();
    expect(screen.getByText('OnePlus · GM1917 · 192.168.0.101:37831 · HandShaker v2.5.0')).toBeInTheDocument();
    expect(screen.getByText('Paired')).toBeInTheDocument();

    const usbBtn = screen.getByRole('button', { name: 'Connect USB' });
    expect(usbBtn).toBeDisabled();

    const consent = screen.getByLabelText(/Allow sending AOA mode-switch/i);
    fireEvent.click(consent);
    expect(usbBtn).toBeEnabled();

    const directBtn = screen.getByRole('button', { name: 'Direct' });
    expect(directBtn).toBeDisabled();
    fireEvent.change(screen.getByLabelText('Phone IP or hostname'), {
      target: { value: '192.168.1.42' },
    });
    expect(directBtn).toBeEnabled();

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    await waitFor(() =>
      expect(screen.getByText(/Tiling compositor detected/i)).toBeInTheDocument(),
    );

    const decCheckbox = screen.getByLabelText(/Use system window titlebar/i);
    fireEvent.click(decCheckbox);
    await waitFor(() =>
      expect(mocks.updateSettings).toHaveBeenCalledWith({ useSystemDecorations: true }),
    );
  });

  it('loads connected device files, Photos QuickLook on Spacebar, Clipboard notes, and Transfer drawer', async () => {
    mocks.getConnectionState.mockResolvedValue({
      status: 'connected',
      handshake: {
        deviceUuid: 'bfd208a26332f56e',
        deviceName: 'vivo X200 Pro mini',
        apkVersion: '235',
        apkVersionName: '2.5.0',
        trustType: 5,
        transport: 'wifi',
        transportLabel: '192.168.0.103:44289',
        isSmartisanDevice: false,
      },
      error: null,
    });
    mocks.getDeviceInfo.mockResolvedValue({
      phoneName: 'vivo X200 Pro mini',
      phoneModel: 'V2419A',
      productBrand: 'vivo',
      productManufacturer: 'vivo',
      apkVersionName: '2.5.0',
      rootPath: '/storage/emulated/0',
      externalStoragePath: null,
      batteryPercentage: 88,
      diskSize: 256_000_000_000,
      usedDiskSize: 64_000_000_000,
      freeDiskSize: 192_000_000_000,
      picVideoSize: 16_000_000_000,
      audioSize: 2_000_000_000,
      downloadSize: 4_000_000_000,
      appSize: 22_000_000_000,
      cacheSize: 1_000_000_000,
      otherSize: 19_000_000_000,
    });
    mocks.listDir.mockResolvedValue([
      {
        name: 'DCIM',
        path: '/storage/emulated/0/DCIM',
        isDirectory: true,
        fileSize: 0,
        createdTimestamp: 1700000000,
        modifiedTimestamp: 1700000000,
        isTrash: false,
      },
      {
        name: 'report-final.pdf',
        path: '/storage/emulated/0/report-final.pdf',
        isDirectory: false,
        fileSize: 2048,
        createdTimestamp: 1700000000,
        modifiedTimestamp: 1700000000,
        isTrash: false,
      },
    ]);
    mocks.getPhotoLibrary.mockResolvedValue({
      cameraAlbumId: 10,
      albums: [
        {
          albumId: 10,
          albumName: 'Camera',
          albumPath: '/storage/emulated/0/DCIM/Camera',
          coverThumbUrl: null,
        },
      ],
      images: [
        {
          mediaId: 1001,
          albumId: 10,
          albumName: 'Camera',
          path: '/storage/emulated/0/DCIM/Camera/IMG_01.jpg',
          title: 'IMG_01.jpg',
          mimeType: 'image/jpeg',
          fileSize: 4096,
          width: 4032,
          height: 3024,
          orientation: 0,
          dateTaken: 1700000000,
          modifiedTimestamp: 1700000000,
          starred: false,
          thumbUrl: 'http://127.0.0.1:9999/thumb?id=1001',
          streamUrl: 'http://127.0.0.1:9999/stream?id=1001',
        },
      ],
    });
    mocks.getClipboardHistory.mockResolvedValue([
      { text: 'Phone clipboard note', timestampMs: 1700000000000, rawGzipB64: 'AAA=' },
    ]);
    mocks.listTransfers.mockResolvedValue([
      {
        id: 'tx-1',
        direction: 'download',
        fileName: 'IMG_01.jpg',
        localPath: '/home/howard/Downloads/IMG_01.jpg',
        remotePath: '/storage/emulated/0/DCIM/Camera/IMG_01.jpg',
        totalBytes: 1000,
        transferredBytes: 500,
        status: 'active',
        error: null,
        createdAtMs: 1700000000000,
      },
    ]);

    useAppStore.setState({ initialized: false, tab: 'files' });
    render(<App />);
    await waitFor(() => expect(screen.getByText('DCIM')).toBeInTheDocument());
    expect(screen.getByText('report-final')).toBeInTheDocument();
    expect(screen.getByText('.pdf')).toBeInTheDocument();

    // Selecting DCIM then filtering to report-final must NOT mark header Select All as checked
    fireEvent.click(screen.getByLabelText('Select DCIM'));
    const filterInput = screen.getByLabelText('Filter files in folder');
    fireEvent.change(filterInput, { target: { value: 'report-final' } });
    expect(screen.getByLabelText('Select all entries')).not.toBeChecked();
    fireEvent.change(filterInput, { target: { value: '' } });
    fireEvent.click(screen.getByLabelText('Select DCIM'));

    // Open Photos and trigger Spacebar QuickLook
    fireEvent.click(screen.getByRole('button', { name: 'Photos' }));
    await waitFor(() => expect(screen.getByLabelText('Preview IMG_01.jpg')).toBeInTheDocument());
    fireEvent.keyDown(window, { code: 'Space' });
    await waitFor(() => expect(screen.getByText('4032×3024 • 4.00 KB •', { exact: false })).toBeInTheDocument());
    fireEvent.keyDown(window, { code: 'Space' });

    // Open Clipboard and submit a note
    fireEvent.click(screen.getByRole('button', { name: 'Clipboard' }));
    await waitFor(() => expect(screen.getByText('Phone clipboard note')).toBeInTheDocument());
    fireEvent.change(screen.getByLabelText('Send clipboard note to phone'), {
      target: { value: 'Hello from Linux' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Push to Phone' }));
    await waitFor(() => expect(mocks.postClipboardText).toHaveBeenCalledWith('Hello from Linux'));

    fireEvent.click(screen.getByRole('button', { name: 'Copy' }));
    await waitFor(() =>
      expect(screen.getByRole('status')).toHaveTextContent('Copied snippet to Linux clipboard'),
    );

    // Open Transfers drawer & cancel active transfer
    fireEvent.click(screen.getByRole('button', { name: /Transfers/i }));
    await waitFor(() => expect(screen.getByText('500 B / 1000 B (50%)')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: 'Cancel IMG_01.jpg' }));
    expect(mocks.cancelTransfer).toHaveBeenCalledWith('tx-1');
  });

  it('does not duplicate drag-and-drop uploads under StrictMode async unlisten race', async () => {
    webviewMock.deferNext = true;
    mocks.enqueueUpload.mockResolvedValue(1);
    useAppStore.setState({
      currentPath: '/storage/emulated/0/Documents',
      connection: {
        status: 'connected',
        handshake: null,
        error: null,
      },
    });

    const { unmount } = render(
      <StrictMode>
        <FilesView />
      </StrictMode>,
    );

    expect(webviewMock.deferredResolvers).toHaveLength(2);
    for (const resolve of webviewMock.deferredResolvers) {
      resolve();
    }

    await waitFor(() => expect(webviewMock.dragHandlers.size).toBe(1));

    for (const handler of Array.from(webviewMock.dragHandlers)) {
      handler({ payload: { type: 'drop', paths: ['/tmp/demo.txt', '/tmp/demo.txt'] } });
    }

    await waitFor(() => expect(mocks.enqueueUpload).toHaveBeenCalledTimes(1));
    expect(mocks.enqueueUpload).toHaveBeenCalledWith(
      ['/tmp/demo.txt'],
      '/storage/emulated/0/Documents',
    );

    unmount();
    await waitFor(() => expect(webviewMock.dragHandlers.size).toBe(0));
  });

  it('avoids mounting audio/video elements and offers external player when GStreamer elements are missing', async () => {
    mocks.getMediaPlaybackSupport.mockResolvedValue({
      embeddedAudio: false,
      embeddedVideo: false,
      missingElements: ['autoaudiosink', 'souphttpsrc', 'autovideosink'],
      reason:
        'In-app playback is disabled to prevent WebKitGTK crash because GStreamer element(s) are missing: autoaudiosink, souphttpsrc, autovideosink.',
    });
    mocks.openMediaExternally.mockResolvedValue('mpv');
    mocks.getConnectionState.mockResolvedValue({
      status: 'connected',
      handshake: {
        deviceUuid: 'bfd208a26332f56e',
        deviceName: 'vivo X200 Pro mini',
        apkVersion: '235',
        apkVersionName: '2.5.0',
        trustType: 5,
        transport: 'wifi',
        transportLabel: '192.168.0.103:44289',
        isSmartisanDevice: false,
      },
      error: null,
    });
    mocks.getDeviceInfo.mockResolvedValue({
      phoneName: 'vivo X200 Pro mini',
      phoneModel: 'V2419A',
      productBrand: 'vivo',
      productManufacturer: 'vivo',
      apkVersionName: '2.5.0',
      rootPath: '/storage/emulated/0',
      externalStoragePath: null,
      batteryPercentage: 88,
      diskSize: 100,
      usedDiskSize: 20,
      freeDiskSize: 80,
      picVideoSize: 10,
      audioSize: 5,
      downloadSize: 2,
      appSize: 2,
      cacheSize: 1,
      otherSize: 0,
    });
    mocks.listDir.mockResolvedValue([]);
    mocks.getVideoLibrary.mockResolvedValue({
      albums: [],
      videos: [
        {
          mediaId: 7,
          albumId: 1,
          albumName: 'Camera',
          path: '/storage/emulated/0/Movies/clip.mp4',
          name: 'clip.mp4',
          mimeType: 'video/mp4',
          fileSize: 2048,
          durationSecs: 12,
          width: 1920,
          height: 1080,
          modifiedTimestamp: 1700000000,
          thumbUrl: 'http://127.0.0.1:9999/thumb?id=7',
          streamUrl: 'http://127.0.0.1:9999/stream?path=%2Fclip.mp4&token=t',
        },
      ],
    });
    mocks.getAudioLibrary.mockResolvedValue({
      albums: [],
      tracks: [
        {
          mediaId: 9,
          albumId: 2,
          path: '/storage/emulated/0/Music/song.flac',
          title: 'Morning Brew',
          artist: 'Tester',
          mimeType: 'audio/flac',
          fileSize: 4096,
          durationSecs: 185,
          bitrate: 900,
          sampleRate: 44100,
          year: 2026,
          track: 1,
          streamUrl: 'http://127.0.0.1:9999/stream?path=%2Fsong.flac&token=t',
        },
      ],
    });

    useAppStore.setState({ initialized: false, tab: 'files' });
    render(<App />);
    await waitFor(() => expect(screen.getByText('vivo X200 Pro mini')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Videos' }));
    await waitFor(() => expect(screen.getByText('clip.mp4')).toBeInTheDocument());
    fireEvent.click(screen.getAllByRole('button', { name: 'Play clip.mp4' })[0]);

    await waitFor(() =>
      expect(screen.getByText('In-app video playback unavailable')).toBeInTheDocument(),
    );
    expect(document.querySelector('video')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Open in System Player' }));
    await waitFor(() =>
      expect(mocks.openMediaExternally).toHaveBeenCalledWith(
        'http://127.0.0.1:9999/stream?path=%2Fclip.mp4&token=t',
      ),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Close dialog' }));

    fireEvent.click(screen.getByRole('button', { name: 'Music' }));
    await waitFor(() => expect(screen.getByText('Morning Brew')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: 'Play Morning Brew' }));

    await waitFor(() =>
      expect(screen.getByRole('button', { name: 'Open in System Player' })).toBeInTheDocument(),
    );
    expect(document.querySelector('audio')).toBeNull();
  });

  it('uploads dropped files from Photos, Videos, and Music views to DCIM, Movies, and Music', async () => {
    mocks.enqueueUpload.mockResolvedValue(1);
    mocks.listTransfers.mockResolvedValue([]);

    const cases = [
      {
        label: 'Photos default root',
        View: PhotosView,
        rootPath: null as string | null,
        expectedDir: '/storage/emulated/0/DCIM',
        paths: ['/home/user/pic.jpg'],
      },
      {
        label: 'Photos custom root',
        View: PhotosView,
        rootPath: '/storage/primary',
        expectedDir: '/storage/primary/DCIM',
        paths: ['/home/user/a.jpg', '/home/user/Album'],
      },
      {
        label: 'Videos',
        View: VideosView,
        rootPath: '/storage/emulated/0/',
        expectedDir: '/storage/emulated/0/Movies',
        paths: ['/home/user/clip.mp4'],
      },
      {
        label: 'Music',
        View: MusicView,
        rootPath: '/storage/emulated/0',
        expectedDir: '/storage/emulated/0/Music',
        paths: ['/home/user/song.flac'],
      },
    ];

    for (const c of cases) {
      mocks.enqueueUpload.mockClear();
      useAppStore.setState({
        deviceInfo: c.rootPath
          ? {
              phoneName: 'Test Phone',
              phoneModel: 'ModelX',
              productBrand: 'Brand',
              productManufacturer: 'Maker',
              apkVersionName: '2.5.0',
              rootPath: c.rootPath,
              externalStoragePath: null,
              batteryPercentage: 80,
              diskSize: 100,
              usedDiskSize: 20,
              freeDiskSize: 80,
              picVideoSize: 10,
              audioSize: 5,
              downloadSize: 2,
              appSize: 2,
              cacheSize: 1,
              otherSize: 0,
            }
          : null,
      });

      const { unmount } = render(<c.View />);
      await waitFor(() => expect(webviewMock.dragHandlers.size).toBe(1));

      await act(async () => {
        for (const handler of webviewMock.dragHandlers) {
          handler({ payload: { type: 'enter', paths: [], position: { x: 20, y: 20 } } });
        }
      });
      expect(
        screen.getByText(`Drop Linux files or folders here to upload to ${c.expectedDir}`),
      ).toBeInTheDocument();

      await act(async () => {
        for (const handler of webviewMock.dragHandlers) {
          handler({ payload: { type: 'drop', paths: c.paths, position: { x: 20, y: 20 } } });
        }
      });

      await waitFor(() =>
        expect(mocks.enqueueUpload).toHaveBeenCalledWith(c.paths, c.expectedDir),
      );
      expect(
        screen.queryByText(`Drop Linux files or folders here to upload to ${c.expectedDir}`),
      ).not.toBeInTheDocument();

      unmount();
    }
  });

  it('resolves system locale and switches UI cleanly to Simplified Chinese across views', async () => {
    expect(resolveLanguage('zh-CN', ['en-US'])).toBe('zh-CN');
    expect(resolveLanguage('en', ['zh-CN'])).toBe('en');
    expect(resolveLanguage('system', ['zh-CN', 'en-US'])).toBe('zh-CN');
    expect(resolveLanguage('system', ['zh-TW'])).toBe('zh-CN');
    expect(resolveLanguage('system', ['en-US'])).toBe('en');

    mocks.updateSettings.mockResolvedValue({
      useSystemDecorations: false,
      theme: 'system',
      language: 'zh-CN',
      downloadDir: '/home/howard/Downloads',
      trashDir: '/home/howard/.HandShakerTrash',
      clipboardAutoSync: true,
      isTilingWm: true,
    });

    const { unmount } = render(<App />);
    await waitFor(() => expect(screen.getByText('Connect Your Android Phone')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    await waitFor(() => expect(screen.getByText('Window & Appearance')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: '简体中文' }));
    await waitFor(() => expect(mocks.updateSettings).toHaveBeenCalledWith({ language: 'zh-CN' }));
    await waitFor(() => expect(document.documentElement.lang).toBe('zh-CN'));

    expect(screen.getByText('窗口与外观')).toBeInTheDocument();
    expect(screen.getByText('界面语言')).toBeInTheDocument();
    expect(screen.getByText('使用系统窗口标题栏（原生窗口边框）')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: '浅色' })).toBeInTheDocument();

    // Navigate back to Welcome (Connect Device) from Settings and check Chinese Welcome labels
    fireEvent.click(screen.getByRole('button', { name: '连接设备' }));
    await waitFor(() => expect(screen.getByText('连接你的 Android 手机')).toBeInTheDocument());
    expect(screen.getByText('Wi-Fi 局域网设备 (2)')).toBeInTheDocument();
    expect(screen.getByText('USB AOA 设备 (1)')).toBeInTheDocument();
    expect(screen.getByText('已信任的配对手机 (1)')).toBeInTheDocument();
    expect(screen.getByText('已配对')).toBeInTheDocument();
    unmount();

    // Verify connected tabs in Simplified Chinese
    mocks.getSettings.mockResolvedValue({
      useSystemDecorations: false,
      theme: 'system',
      language: 'zh-CN',
      downloadDir: '/home/howard/Downloads',
      trashDir: '/home/howard/.HandShakerTrash',
      clipboardAutoSync: true,
      isTilingWm: true,
    });
    mocks.getConnectionState.mockResolvedValue({
      status: 'connected',
      handshake: {
        deviceUuid: 'bfd208a26332f56e',
        deviceName: 'vivo X200 Pro mini',
        apkVersion: '235',
        apkVersionName: '2.5.0',
        trustType: 5,
        transport: 'wifi',
        transportLabel: '192.168.0.103:44289',
        isSmartisanDevice: false,
      },
      error: null,
    });
    mocks.getDeviceInfo.mockResolvedValue({
      phoneName: 'vivo X200 Pro mini',
      phoneModel: 'V2419A',
      productBrand: 'vivo',
      productManufacturer: 'vivo',
      apkVersionName: '2.5.0',
      rootPath: '/storage/emulated/0',
      externalStoragePath: null,
      batteryPercentage: 88,
      diskSize: 256_000_000_000,
      usedDiskSize: 64_000_000_000,
      freeDiskSize: 192_000_000_000,
      picVideoSize: 16_000_000_000,
      audioSize: 2_000_000_000,
      downloadSize: 4_000_000_000,
      appSize: 22_000_000_000,
      cacheSize: 1_000_000_000,
      otherSize: 19_000_000_000,
    });
    mocks.listDir.mockResolvedValue([]);
    mocks.getPhotoLibrary.mockResolvedValue({ cameraAlbumId: null, albums: [], images: [] });
    mocks.getVideoLibrary.mockResolvedValue({ albums: [], videos: [] });
    mocks.getAudioLibrary.mockResolvedValue({ albums: [], tracks: [] });
    mocks.getClipboardHistory.mockResolvedValue([]);
    mocks.listTransfers.mockResolvedValue([
      {
        id: 'tx-zh',
        direction: 'upload',
        fileName: 'photo.jpg',
        localPath: '/tmp/photo.jpg',
        remotePath: '/storage/emulated/0/DCIM/photo.jpg',
        totalBytes: 1000,
        transferredBytes: 250,
        status: 'active',
        error: null,
        createdAtMs: 1700000000000,
      },
    ]);

    useAppStore.setState({ initialized: false, tab: 'files' });
    render(<App />);
    await waitFor(() => expect(screen.getByText('手机存储')).toBeInTheDocument());
    expect(screen.getByText('当前文件夹为空。')).toBeInTheDocument();
    expect(screen.getByText('存储空间')).toBeInTheDocument();
    expect(screen.getByText('电量 88%')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '照片' }));
    await waitFor(() => expect(screen.getByText('全部照片 (0)')).toBeInTheDocument());
    expect(screen.getByText('当前相册中暂无照片。')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: '视频' }));
    await waitFor(() => expect(screen.getByText('视频 (0)')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: '音乐' }));
    await waitFor(() => expect(screen.getByText('音乐（0 首歌曲 · 0 张专辑）')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: '剪贴板' }));
    await waitFor(() => expect(screen.getByText('剪贴板同步 (0)')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /传输队列/ }));
    await waitFor(() => expect(screen.getByText('传输队列 (1)')).toBeInTheDocument());
    expect(screen.getByText('传输中')).toBeInTheDocument();
  });

  it('shows file path under QuickLook title, puts Delete left of Download, and requires confirmation', async () => {
    mocks.deleteFiles.mockResolvedValue(undefined);
    useAppStore.setState({
      tab: 'photos',
      connection: { status: 'connected', handshake: null, error: null },
      photoLibrary: {
        cameraAlbumId: 10,
        albums: [
          {
            albumId: 10,
            albumName: 'Camera',
            albumPath: '/storage/emulated/0/DCIM/Camera',
            coverThumbUrl: null,
          },
        ],
        images: [
          {
            mediaId: 101,
            albumId: 10,
            albumName: 'Camera',
            path: '/storage/emulated/0/DCIM/Camera/IMG_01.jpg',
            title: 'IMG_01.jpg',
            mimeType: 'image/jpeg',
            fileSize: 2048,
            width: 1920,
            height: 1080,
            orientation: 0,
            dateTaken: 1700000000,
            modifiedTimestamp: 1700000000,
            starred: false,
            thumbUrl: 'http://127.0.0.1:9999/thumb?id=101',
            streamUrl: 'http://127.0.0.1:9999/stream?id=101',
          },
          {
            mediaId: 102,
            albumId: 10,
            albumName: 'Camera',
            path: '/storage/emulated/0/DCIM/Camera/IMG_02.jpg',
            title: 'IMG_02.jpg',
            mimeType: 'image/jpeg',
            fileSize: 4096,
            width: 1920,
            height: 1080,
            orientation: 0,
            dateTaken: 1700000010,
            modifiedTimestamp: 1700000010,
            starred: false,
            thumbUrl: 'http://127.0.0.1:9999/thumb?id=102',
            streamUrl: 'http://127.0.0.1:9999/stream?id=102',
          },
        ],
      },
    });

    render(<PhotosView />);
    fireEvent.click(screen.getByRole('button', { name: 'Preview IMG_01.jpg' }));

    const quickLookDialog = document.querySelector('#photo-quicklook-title')?.closest('dialog');
    expect(quickLookDialog).toBeTruthy();
    expect(screen.getByText('IMG_01.jpg')).toBeInTheDocument();
    expect(screen.getByText('/storage/emulated/0/DCIM/Camera/IMG_01.jpg')).toBeInTheDocument();

    const quickLookScope = within(quickLookDialog as HTMLElement);
    const deleteBtn = quickLookScope.getByRole('button', { name: 'Delete' });
    const downloadBtn = quickLookScope.getByRole('button', { name: 'Download' });
    expect(
      Boolean(deleteBtn.compareDocumentPosition(downloadBtn) & Node.DOCUMENT_POSITION_FOLLOWING),
    ).toBe(true);

    // Clicking Delete opens confirmation modal without deleting yet
    fireEvent.click(deleteBtn);
    expect(mocks.deleteFiles).not.toHaveBeenCalled();

    const confirmDialog = document.querySelector('#photos-delete-dialog-title')?.closest('dialog');
    expect(confirmDialog).toBeTruthy();
    const confirmScope = within(confirmDialog as HTMLElement);
    expect(confirmScope.getByText('/storage/emulated/0/DCIM/Camera/IMG_01.jpg')).toBeInTheDocument();

    // Cancel keeps IMG_01.jpg
    fireEvent.click(confirmScope.getByRole('button', { name: 'Cancel' }));
    expect(mocks.deleteFiles).not.toHaveBeenCalled();

    // Re-open confirmation and confirm permanent delete
    fireEvent.click(deleteBtn);
    const confirmScope2 = within(confirmDialog as HTMLElement);
    fireEvent.click(
      confirmScope2.getByLabelText('Delete permanently instead of moving to trash'),
    );
    fireEvent.click(confirmScope2.getByRole('button', { name: 'Delete Permanently' }));

    await waitFor(() =>
      expect(mocks.deleteFiles).toHaveBeenCalledWith(
        ['/storage/emulated/0/DCIM/Camera/IMG_01.jpg'],
        false,
      ),
    );
    await waitFor(() =>
      expect(screen.getByText('/storage/emulated/0/DCIM/Camera/IMG_02.jpg')).toBeInTheDocument(),
    );
  });

  it('supports multi-select copy and confirmed delete across Photos, Videos, Music, and Clipboard', async () => {
    mocks.deleteFiles.mockResolvedValue(undefined);
    mocks.deleteClipboardItem.mockResolvedValue(undefined);
    mocks.enqueueDownload.mockResolvedValue(2);
    mocks.pickLocalPaths.mockResolvedValue(['/home/howard/Pictures/Export']);

    useAppStore.setState({
      connection: { status: 'connected', handshake: null, error: null },
      photoLibrary: {
        cameraAlbumId: 10,
        albums: [],
        images: [
          {
            mediaId: 1,
            albumId: 10,
            albumName: 'Camera',
            path: '/storage/emulated/0/DCIM/Camera/P1.jpg',
            title: 'P1.jpg',
            mimeType: 'image/jpeg',
            fileSize: 100,
            width: 100,
            height: 100,
            orientation: 0,
            dateTaken: 1700000000,
            modifiedTimestamp: 1700000000,
            starred: false,
            thumbUrl: 'http://127.0.0.1:9999/t1',
            streamUrl: 'http://127.0.0.1:9999/s1',
          },
          {
            mediaId: 2,
            albumId: 10,
            albumName: 'Camera',
            path: '/storage/emulated/0/DCIM/Camera/P2.jpg',
            title: 'P2.jpg',
            mimeType: 'image/jpeg',
            fileSize: 200,
            width: 100,
            height: 100,
            orientation: 0,
            dateTaken: 1700000001,
            modifiedTimestamp: 1700000001,
            starred: false,
            thumbUrl: 'http://127.0.0.1:9999/t2',
            streamUrl: 'http://127.0.0.1:9999/s2',
          },
        ],
      },
      videoLibrary: {
        albums: [],
        videos: [
          {
            mediaId: 11,
            albumId: 1,
            path: '/storage/emulated/0/Movies/V1.mp4',
            name: 'V1.mp4',
            mimeType: 'video/mp4',
            fileSize: 500,
            width: 1920,
            height: 1080,
            durationSecs: 10,
            modifiedTimestamp: 1700000000,
            thumbUrl: 'http://127.0.0.1:9999/vt1',
            streamUrl: 'http://127.0.0.1:9999/vs1',
          },
          {
            mediaId: 12,
            albumId: 1,
            path: '/storage/emulated/0/Movies/V2.mp4',
            name: 'V2.mp4',
            mimeType: 'video/mp4',
            fileSize: 600,
            width: 1920,
            height: 1080,
            durationSecs: 20,
            modifiedTimestamp: 1700000001,
            thumbUrl: 'http://127.0.0.1:9999/vt2',
            streamUrl: 'http://127.0.0.1:9999/vs2',
          },
        ],
      },
      audioLibrary: {
        albums: [],
        tracks: [
          {
            mediaId: 21,
            albumId: 2,
            path: '/storage/emulated/0/Music/A1.flac',
            title: 'A1',
            artist: 'Artist 1',
            mimeType: 'audio/flac',
            fileSize: 300,
            durationSecs: 120,
            bitrate: 900,
            sampleRate: 44100,
            year: 2026,
            track: 1,
            streamUrl: 'http://127.0.0.1:9999/as1',
          },
          {
            mediaId: 22,
            albumId: 2,
            path: '/storage/emulated/0/Music/A2.flac',
            title: 'A2',
            artist: 'Artist 2',
            mimeType: 'audio/flac',
            fileSize: 400,
            durationSecs: 140,
            bitrate: 900,
            sampleRate: 44100,
            year: 2026,
            track: 2,
            streamUrl: 'http://127.0.0.1:9999/as2',
          },
        ],
      },
      clipboard: [
        { text: 'Secret snippet', timestampMs: 1700000000000, rawGzipB64: 'AAA=' },
      ],
    });

    // Photos multi-select: select 2 photos, Copy to folder, then Delete with confirmation
    useAppStore.setState({ tab: 'photos' });
    const photosRender = render(<PhotosView />);
    fireEvent.click(screen.getByRole('button', { name: 'Multi-select' }));
    fireEvent.click(screen.getByRole('button', { name: 'Select P1.jpg' }));
    fireEvent.click(screen.getByRole('button', { name: 'Select P2.jpg' }));
    expect(screen.getByText('2 of 2 selected')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Copy to…' }));
    await waitFor(() =>
      expect(mocks.enqueueDownload).toHaveBeenCalledWith(
        [
          { remotePath: '/storage/emulated/0/DCIM/Camera/P1.jpg', isDirectory: false, fileSize: 100 },
          { remotePath: '/storage/emulated/0/DCIM/Camera/P2.jpg', isDirectory: false, fileSize: 200 },
        ],
        '/home/howard/Pictures/Export',
      ),
    );

    fireEvent.click(screen.getByRole('button', { name: 'Delete (2)' }));
    expect(mocks.deleteFiles).not.toHaveBeenCalled();
    const photosDelDlg = document.querySelector('#photos-delete-dialog-title')?.closest('dialog');
    fireEvent.click(within(photosDelDlg as HTMLElement).getByRole('button', { name: 'Move to Trash' }));
    await waitFor(() =>
      expect(mocks.deleteFiles).toHaveBeenCalledWith(
        ['/storage/emulated/0/DCIM/Camera/P1.jpg', '/storage/emulated/0/DCIM/Camera/P2.jpg'],
        true,
      ),
    );
    await waitFor(() =>
      expect(screen.getByText('No photos found in this album.')).toBeInTheDocument(),
    );
    photosRender.unmount();

    // Videos multi-select: select V1 & V2, download + confirmed delete
    mocks.enqueueDownload.mockClear();
    mocks.deleteFiles.mockClear();
    useAppStore.setState({ tab: 'videos' });
    const videosRender = render(<VideosView />);
    fireEvent.click(screen.getByRole('button', { name: 'Multi-select' }));
    fireEvent.click(screen.getByRole('button', { name: 'Select V1.mp4' }));
    fireEvent.click(screen.getByRole('button', { name: 'Select V2.mp4' }));
    fireEvent.click(screen.getByRole('button', { name: 'Download (2)' }));
    await waitFor(() =>
      expect(mocks.enqueueDownload).toHaveBeenCalledWith(
        [
          { remotePath: '/storage/emulated/0/Movies/V1.mp4', isDirectory: false, fileSize: 500 },
          { remotePath: '/storage/emulated/0/Movies/V2.mp4', isDirectory: false, fileSize: 600 },
        ],
        undefined,
      ),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Delete (2)' }));
    expect(mocks.deleteFiles).not.toHaveBeenCalled();
    const videosDelDlg = document.querySelector('#videos-delete-dialog-title')?.closest('dialog');
    fireEvent.click(within(videosDelDlg as HTMLElement).getByRole('button', { name: 'Move to Trash' }));
    await waitFor(() =>
      expect(mocks.deleteFiles).toHaveBeenCalledWith(
        ['/storage/emulated/0/Movies/V1.mp4', '/storage/emulated/0/Movies/V2.mp4'],
        true,
      ),
    );
    videosRender.unmount();

    // Music multi-select: select A1 & A2 via Select All, copy to folder + confirmed delete
    mocks.enqueueDownload.mockClear();
    mocks.deleteFiles.mockClear();
    useAppStore.setState({ tab: 'music' });
    const musicRender = render(<MusicView />);
    fireEvent.click(screen.getByRole('button', { name: 'Multi-select' }));
    fireEvent.click(screen.getByRole('button', { name: 'Select All' }));
    expect(screen.getByText('2 of 2 selected')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Copy to…' }));
    await waitFor(() =>
      expect(mocks.enqueueDownload).toHaveBeenCalledWith(
        [
          { remotePath: '/storage/emulated/0/Music/A1.flac', isDirectory: false, fileSize: 300 },
          { remotePath: '/storage/emulated/0/Music/A2.flac', isDirectory: false, fileSize: 400 },
        ],
        '/home/howard/Pictures/Export',
      ),
    );
    fireEvent.click(screen.getByRole('button', { name: 'Delete (2)' }));
    expect(mocks.deleteFiles).not.toHaveBeenCalled();
    const musicDelDlg = document.querySelector('#music-delete-dialog-title')?.closest('dialog');
    fireEvent.click(within(musicDelDlg as HTMLElement).getByRole('button', { name: 'Move to Trash' }));
    await waitFor(() =>
      expect(mocks.deleteFiles).toHaveBeenCalledWith(
        ['/storage/emulated/0/Music/A1.flac', '/storage/emulated/0/Music/A2.flac'],
        true,
      ),
    );
    musicRender.unmount();

    // Clipboard single-item delete also requires confirmation
    useAppStore.setState({ tab: 'clipboard' });
    const clipRender = render(<ClipboardView />);
    fireEvent.click(screen.getByRole('button', { name: 'Delete clipboard note' }));
    expect(mocks.deleteClipboardItem).not.toHaveBeenCalled();
    const clipDelDlg = document.querySelector('#delete-clip-title')?.closest('dialog');
    expect(clipDelDlg).toBeTruthy();
    fireEvent.click(within(clipDelDlg as HTMLElement).getByRole('button', { name: 'Delete' }));
    await waitFor(() =>
      expect(mocks.deleteClipboardItem).toHaveBeenCalledWith(1700000000000, 'AAA='),
    );
    clipRender.unmount();
  });

  it('picks Linux trashDir via Choose folder button in Settings like downloadDir', async () => {
    mocks.pickLocalPaths.mockResolvedValueOnce(['/home/howard/Backups/PhoneTrash']);
    mocks.updateSettings.mockImplementation(async (patch: Record<string, unknown>) => ({
      useSystemDecorations: false,
      theme: 'system',
      language: 'en',
      downloadDir: '/home/howard/Downloads',
      trashDir: (patch.trashDir as string) ?? '/home/howard/.HandShakerTrash',
      clipboardAutoSync: true,
      isTilingWm: true,
    }));

    useAppStore.setState({ initialized: false, tab: 'files' });
    render(<App />);

    await waitFor(() => expect(screen.getByText('Connect Your Android Phone')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const trashHeading = await waitFor(() => screen.getByRole('heading', { name: 'Trash' }));
    const trashCard = trashHeading.closest('section') as HTMLElement;

    expect(within(trashCard).getByText('Linux trash directory')).toBeInTheDocument();
    expect(within(trashCard).getByText('/home/howard/.HandShakerTrash')).toBeInTheDocument();
    expect(within(trashCard).queryByRole('textbox')).not.toBeInTheDocument();
    expect(within(trashCard).queryByRole('button', { name: 'Save' })).not.toBeInTheDocument();
    expect(within(trashCard).queryByRole('button', { name: 'Reset' })).not.toBeInTheDocument();

    // Pick valid local Linux directory via native picker
    fireEvent.click(within(trashCard).getByRole('button', { name: 'Choose…' }));
    await waitFor(() => expect(mocks.pickLocalPaths).toHaveBeenCalledWith(true, false));
    await waitFor(() =>
      expect(mocks.updateSettings).toHaveBeenCalledWith({
        trashDir: '/home/howard/Backups/PhoneTrash',
      }),
    );
    expect(await waitFor(() => within(trashCard).getByText('/home/howard/Backups/PhoneTrash'))).toBeInTheDocument();

    // Picking system/home root shows inline validation error without calling backend
    mocks.pickLocalPaths.mockResolvedValueOnce(['/home']);
    fireEvent.click(within(trashCard).getByRole('button', { name: 'Choose…' }));
    expect(
      await waitFor(() => within(trashCard).getByText(/Invalid Linux trash directory/i)),
    ).toBeInTheDocument();
    expect(mocks.updateSettings).toHaveBeenCalledTimes(1);
  });

  it('opens Trash page from Settings card and supports viewing, restoring, deleting, and emptying', async () => {
    const sampleItems = [
      {
        entryName: 'report.pdf',
        isDirectory: false,
        sizeBytes: 2048,
        originalPath: '/storage/emulated/0/Documents/report.pdf',
        deletionDate: '2026-11-15T12:00:00Z',
        deviceUuid: 'u1',
        deviceName: 'vivo X200 Pro mini',
        deviceModel: 'vivo V2419A',
        modifiedTimestampMs: 1794744000000,
      },
      {
        entryName: 'Vacation',
        isDirectory: true,
        sizeBytes: 4096,
        originalPath: '/storage/emulated/0/Pictures/Vacation',
        deletionDate: '2026-11-15T12:05:00Z',
        deviceUuid: 'u1',
        deviceName: 'vivo X200 Pro mini',
        deviceModel: 'vivo V2419A',
        modifiedTimestampMs: 1794744300000,
      },
    ];
    let currentItems = [...sampleItems];
    mocks.listTrashItems.mockImplementation(async () => [...currentItems]);
    mocks.restoreTrashItems.mockImplementation(async (entryNames: string[]) => {
      const restored = currentItems
        .filter((it) => entryNames.includes(it.entryName))
        .map((it) => it.originalPath as string);
      currentItems = currentItems.filter((it) => !entryNames.includes(it.entryName));
      return restored;
    });
    mocks.deleteTrashItems.mockImplementation(async (entryNames: string[]) => {
      const before = currentItems.length;
      currentItems = currentItems.filter((it) => !entryNames.includes(it.entryName));
      return before - currentItems.length;
    });
    mocks.emptyTrash.mockImplementation(async () => {
      const count = currentItems.length;
      currentItems = [];
      return count;
    });

    useAppStore.setState({ initialized: false, tab: 'files' });
    render(<App />);

    await waitFor(() => expect(screen.getByText('Connect Your Android Phone')).toBeInTheDocument());
    fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
    fireEvent.click(await waitFor(() => screen.getByRole('button', { name: 'Trash' })));

    await waitFor(() => expect(mocks.listTrashItems).toHaveBeenCalledTimes(1));
    expect(await waitFor(() => screen.getByText('report.pdf'))).toBeInTheDocument();
    expect(screen.getByText('/storage/emulated/0/Documents/report.pdf')).toBeInTheDocument();
    expect(screen.getByText('Vacation')).toBeInTheDocument();

    // Disconnected phone disables restore actions cleanly
    expect(screen.getByText(/Connect phone to restore files/i)).toBeInTheDocument();
    const rowRestoreBtns = screen.getAllByRole('button', { name: 'Restore' });
    expect(rowRestoreBtns[0]).toBeDisabled();

    // Connect phone -> Restore becomes enabled and restores selected/row item
    act(() => {
      useAppStore.setState({
        connection: {
          status: 'connected',
          handshake: {
            deviceUuid: 'u1',
            deviceName: 'vivo X200 Pro mini',
            apkVersion: '1',
            apkVersionName: '1.3.1.28',
            trustType: 1,
            transport: 'wifi',
            transportLabel: 'Wi-Fi',
            isSmartisanDevice: false,
          },
          error: null,
        },
      });
    });

    await waitFor(() => expect(screen.getAllByRole('button', { name: 'Restore' })[0]).toBeEnabled());
    fireEvent.click(screen.getAllByRole('button', { name: 'Restore' })[0]);

    await waitFor(() => expect(mocks.restoreTrashItems).toHaveBeenCalledWith(['report.pdf']));
    await waitFor(() => expect(screen.queryByText('report.pdf')).not.toBeInTheDocument());
    expect(screen.getByText('Restored 1 item(s) to phone')).toBeInTheDocument();

    // Re-populate with 2 items to test Delete Permanently & Empty Trash confirmation dialogs
    currentItems = [...sampleItems];
    fireEvent.click(screen.getByRole('button', { name: 'Refresh' }));
    expect(await waitFor(() => screen.getByText('report.pdf'))).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('report.pdf'));
    fireEvent.click(screen.getByRole('button', { name: 'Delete Permanently' }));
    const deleteDialog = await waitFor(() => screen.getByRole('dialog'));
    fireEvent.click(within(deleteDialog).getByRole('button', { name: 'Delete Permanently' }));

    await waitFor(() => expect(mocks.deleteTrashItems).toHaveBeenCalledWith(['report.pdf']));
    await waitFor(() => expect(screen.queryByText('report.pdf')).not.toBeInTheDocument());
    expect(screen.getByText('Vacation')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Empty Trash' }));
    const emptyDialog = await waitFor(() => screen.getByRole('dialog'));
    fireEvent.click(within(emptyDialog).getByRole('button', { name: 'Empty Trash' }));

    await waitFor(() => expect(mocks.emptyTrash).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.getByText('Trash is empty')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: 'Back to Settings' }));
    expect(await waitFor(() => screen.getByRole('heading', { name: 'Trash' }))).toBeInTheDocument();
  });

  it('blocks restoring Trash items from another phone or missing origin metadata and shows localized error', async () => {
    mocks.listTrashItems.mockResolvedValue([
      {
        entryName: 'vivo_note.txt',
        isDirectory: false,
        sizeBytes: 128,
        originalPath: '/storage/emulated/0/Download/vivo_note.txt',
        deletionDate: '2026-11-15T12:00:00Z',
        deviceUuid: 'u1',
        deviceName: 'vivo X200 Pro mini',
        deviceModel: 'vivo V2419A',
        modifiedTimestampMs: 1794744000000,
      },
      {
        entryName: 'oneplus_photo.jpg',
        isDirectory: false,
        sizeBytes: 4096,
        originalPath: '/storage/emulated/0/DCIM/oneplus_photo.jpg',
        deletionDate: '2026-11-15T12:05:00Z',
        deviceUuid: 'u2',
        deviceName: 'OnePlus 7Pro',
        deviceModel: 'OnePlus GM1917',
        modifiedTimestampMs: 1794744300000,
      },
      {
        entryName: 'legacy_file.bin',
        isDirectory: false,
        sizeBytes: 64,
        originalPath: '/storage/emulated/0/Download/legacy_file.bin',
        deletionDate: '2026-11-15T11:00:00Z',
        deviceUuid: null,
        deviceName: null,
        deviceModel: null,
        modifiedTimestampMs: 1794740400000,
      },
    ]);

    useAppStore.setState({ initialized: false, tab: 'files' });
    render(<App />);

    await waitFor(() => expect(screen.getByText('Connect Your Android Phone')).toBeInTheDocument());
    act(() => {
      useAppStore.setState({
        tab: 'trash',
        connection: {
          status: 'connected',
          handshake: {
            deviceUuid: 'u1',
            deviceName: 'vivo X200 Pro mini',
            apkVersion: '1',
            apkVersionName: '1.3.1.28',
            trustType: 1,
            transport: 'wifi',
            transportLabel: 'Wi-Fi',
            isSmartisanDevice: false,
          },
          error: null,
        },
      });
    });

    expect(await waitFor(() => screen.getByText('OnePlus 7Pro (OnePlus GM1917)'))).toBeInTheDocument();
    expect(screen.getByText('Other phone')).toBeInTheDocument();
    expect(screen.getByText('Unknown phone')).toBeInTheDocument();

    // Row 2 belongs to OnePlus 7Pro (u2), not connected vivo (u1)
    const restoreBtns = screen.getAllByRole('button', { name: 'Restore' });
    fireEvent.click(restoreBtns[1]);

    expect(
      await waitFor(() =>
        screen.getByText(
          'Cannot restore "oneplus_photo.jpg": this file belongs to "OnePlus 7Pro (OnePlus GM1917)", not the currently connected phone.',
        ),
      ),
    ).toBeInTheDocument();
    expect(mocks.restoreTrashItems).not.toHaveBeenCalled();

    // Row 3 has missing origin metadata
    fireEvent.click(restoreBtns[2]);
    expect(
      await waitFor(() =>
        screen.getByText(
          'Cannot restore "legacy_file.bin": source phone metadata is missing, so its origin cannot be verified.',
        ),
      ),
    ).toBeInTheDocument();
    expect(mocks.restoreTrashItems).not.toHaveBeenCalled();

    // Mixed batch selection including u1 + u2 also aborts cleanly in Chinese locale
    act(() => {
      useAppStore.setState((s) => ({
        settings: { ...s.settings, language: 'zh-CN' },
        toastError: null,
      }));
    });
    fireEvent.click(screen.getByLabelText('vivo_note.txt'));
    fireEvent.click(screen.getByLabelText('oneplus_photo.jpg'));
    fireEvent.click(screen.getByRole('button', { name: /恢复选中/ }));

    expect(
      await waitFor(() =>
        screen.getByText(
          '文件「oneplus_photo.jpg」来自手机「OnePlus 7Pro (OnePlus GM1917)」，不是从当前连接的手机删除的，无法恢复。',
        ),
      ),
    ).toBeInTheDocument();
    expect(mocks.restoreTrashItems).not.toHaveBeenCalled();
  });

  it('switches Photos view to Albums mode, lists all folders with paths and counts, and drills into a folder', async () => {
    useAppStore.setState({
      tab: 'photos',
      selectedAlbumId: null,
      quickLookIndex: null,
      photoLibrary: {
        cameraAlbumId: 10,
        albums: [
          {
            albumId: 10,
            albumName: 'Camera',
            albumPath: '/storage/emulated/0/DCIM/Camera',
            coverThumbUrl: 'http://127.0.0.1:4444/thumb?id=1',
          },
          {
            albumId: 20,
            albumName: 'Screenshots',
            albumPath: '/storage/emulated/0/Pictures/Screenshots',
            coverThumbUrl: null,
          },
        ],
        images: [
          {
            mediaId: 1,
            albumId: 10,
            albumName: 'Camera',
            path: '/storage/emulated/0/DCIM/Camera/IMG_01.jpg',
            title: 'IMG_01.jpg',
            mimeType: 'image/jpeg',
            fileSize: 1024,
            width: 1200,
            height: 800,
            orientation: 0,
            dateTaken: 1700000000,
            modifiedTimestamp: 1700000000,
            starred: false,
            thumbUrl: 'http://127.0.0.1:4444/thumb?id=1',
            streamUrl: 'http://127.0.0.1:4444/stream?id=1',
          },
          {
            mediaId: 2,
            albumId: 10,
            albumName: 'Camera',
            path: '/storage/emulated/0/DCIM/Camera/IMG_02.jpg',
            title: 'IMG_02.jpg',
            mimeType: 'image/jpeg',
            fileSize: 2048,
            width: 1200,
            height: 800,
            orientation: 0,
            dateTaken: 1700000100,
            modifiedTimestamp: 1700000100,
            starred: false,
            thumbUrl: 'http://127.0.0.1:4444/thumb?id=2',
            streamUrl: 'http://127.0.0.1:4444/stream?id=2',
          },
          {
            mediaId: 3,
            albumId: 20,
            albumName: 'Screenshots',
            path: '/storage/emulated/0/Pictures/Screenshots/screen_01.png',
            title: 'screen_01.png',
            mimeType: 'image/png',
            fileSize: 512,
            width: 1080,
            height: 2400,
            orientation: 0,
            dateTaken: 1700000200,
            modifiedTimestamp: 1700000200,
            starred: false,
            thumbUrl: 'http://127.0.0.1:4444/thumb?id=3',
            streamUrl: 'http://127.0.0.1:4444/stream?id=3',
          },
          {
            mediaId: 4,
            albumId: 0,
            albumName: '',
            path: '/storage/emulated/0/Pictures/WeChat/wc_01.jpg',
            title: 'wc_01.jpg',
            mimeType: 'image/jpeg',
            fileSize: 256,
            width: 800,
            height: 600,
            orientation: 0,
            dateTaken: 1700000300,
            modifiedTimestamp: 1700000300,
            starred: false,
            thumbUrl: 'http://127.0.0.1:4444/thumb?id=4',
            streamUrl: 'http://127.0.0.1:4444/stream?id=4',
          },
          {
            mediaId: 5,
            albumId: 0,
            albumName: '',
            path: '/storage/emulated/0/Download/dl_01.jpg',
            title: 'dl_01.jpg',
            mimeType: 'image/jpeg',
            fileSize: 256,
            width: 800,
            height: 600,
            orientation: 0,
            dateTaken: 1700000400,
            modifiedTimestamp: 1700000400,
            starred: false,
            thumbUrl: 'http://127.0.0.1:4444/thumb?id=5',
            streamUrl: 'http://127.0.0.1:4444/stream?id=5',
          },
        ],
      },
      settings: {
        useSystemDecorations: false,
        theme: 'system',
        language: 'en',
        downloadDir: '/home/howard/Downloads',
        trashDir: '/home/howard/.HandShakerTrash',
        clipboardAutoSync: true,
        isTilingWm: true,
      },
    });

    const rendered = render(<PhotosView />);
    expect(screen.getByRole('button', { name: 'All Photos (5)' })).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Albums (4)' }));

    expect(
      await waitFor(() => screen.getByRole('button', { name: 'Open album Camera (2 photos)' })),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Open album Screenshots (1 photos)' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Open album WeChat (1 photos)' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: 'Open album Download (1 photos)' }),
    ).toBeInTheDocument();
    expect(screen.getByText('/storage/emulated/0/DCIM/Camera')).toBeInTheDocument();
    expect(screen.getByText('/storage/emulated/0/Pictures/Screenshots')).toBeInTheDocument();
    expect(screen.getByText('/storage/emulated/0/Pictures/WeChat')).toBeInTheDocument();
    expect(screen.getByText('/storage/emulated/0/Download')).toBeInTheDocument();
    expect(screen.getByText('2 photo(s)')).toBeInTheDocument();
    expect(screen.getAllByText('1 photo(s)')).toHaveLength(3);

    fireEvent.click(screen.getByRole('button', { name: 'Open album WeChat (1 photos)' }));
    expect(await waitFor(() => screen.getByLabelText('Preview wc_01.jpg'))).toBeInTheDocument();
    expect(screen.queryByLabelText('Preview dl_01.jpg')).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Back to Albums' }));
    fireEvent.click(
      await waitFor(() => screen.getByRole('button', { name: 'Open album Screenshots (1 photos)' })),
    );

    expect(await waitFor(() => screen.getByLabelText('Preview screen_01.png'))).toBeInTheDocument();
    expect(screen.queryByLabelText('Preview IMG_01.jpg')).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Back to Albums' })).toBeInTheDocument();
    expect(screen.getByText('/storage/emulated/0/Pictures/Screenshots')).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: 'Back to Albums' }));
    expect(
      await waitFor(() => screen.getByRole('button', { name: 'Open album Camera (2 photos)' })),
    ).toBeInTheDocument();

    rendered.unmount();
  });
});
