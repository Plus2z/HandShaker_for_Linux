import { useEffect, useMemo, useRef, useState } from 'react';
import { useVirtualizer } from '@tanstack/react-virtual';
import {
  ArrowLeft,
  Check,
  CheckSquare,
  ChevronLeft,
  ChevronRight,
  Download,
  FolderOpen,
  RefreshCw,
  Trash2,
} from 'lucide-react';
import { formatBytes, formatTimestamp } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import { useMediaSelection } from '../lib/useMediaSelection';
import { useAppStore } from '../store/useAppStore';
import type { ImageItem, PhotoLibrary } from '../types/api';
import { DeleteFilesDialog } from './DeleteFilesDialog';
import { MediaSelectionBar } from './MediaSelectionBar';
import { ModalDialog } from './ModalDialog';
import { FileDropZone, useMediaUploadDir } from './FileDropZone';

interface PhotoFolderEntry {
  albumId: number;
  albumName: string;
  albumPath: string;
  coverThumbUrl: string | null;
  count: number;
  images: ImageItem[];
}

function parentDirOf(path: string): string {
  const clean = path.replace(/\/+$/, '');
  const idx = clean.lastIndexOf('/');
  return idx > 0 ? clean.slice(0, idx) : '';
}

function addToBucket<K, V>(map: Map<K, V[]>, key: K, item: V): void {
  const bucket = map.get(key);
  if (bucket) {
    bucket.push(item);
  } else {
    map.set(key, [item]);
  }
}

function folderTitleFallback(path: string, albumName: string): string {
  return albumName || path.split('/').filter(Boolean).pop() || 'Album';
}

function buildPhotoFolders(library: PhotoLibrary | null): PhotoFolderEntry[] {
  if (!library) return [];

  const imagesByAlbum = new Map<number, ImageItem[]>();
  const zeroAlbumByPath = new Map<string, ImageItem[]>();
  for (const img of library.images) {
    if (img.albumId > 0) {
      addToBucket(imagesByAlbum, img.albumId, img);
    } else {
      addToBucket(zeroAlbumByPath, parentDirOf(img.path), img);
    }
  }

  const folders: PhotoFolderEntry[] = [];
  const seenIds = new Set<number>();
  const seenZeroPaths = new Set<string>();
  let nextSyntheticId = -1;

  for (const album of library.albums) {
    if (album.albumId > 0) {
      if (seenIds.has(album.albumId)) continue;
      seenIds.add(album.albumId);
      const matching = imagesByAlbum.get(album.albumId) ?? [];
      const inferredPath = matching[0] ? parentDirOf(matching[0].path) : '';
      folders.push({
        albumId: album.albumId,
        albumName: album.albumName,
        albumPath: album.albumPath.replace(/\/+$/, '') || inferredPath,
        coverThumbUrl: matching[0]?.thumbUrl ?? album.coverThumbUrl ?? null,
        count: matching.length,
        images: matching,
      });
    } else {
      const cleanPath = album.albumPath.replace(/\/+$/, '');
      const matching = cleanPath ? (zeroAlbumByPath.get(cleanPath) ?? []) : [];
      if (cleanPath) seenZeroPaths.add(cleanPath);
      folders.push({
        albumId: nextSyntheticId--,
        albumName: album.albumName || folderTitleFallback(cleanPath, ''),
        albumPath: cleanPath,
        coverThumbUrl: matching[0]?.thumbUrl ?? album.coverThumbUrl ?? null,
        count: matching.length,
        images: matching,
      });
    }
  }

  for (const [albumId, images] of imagesByAlbum.entries()) {
    if (seenIds.has(albumId) || images.length === 0) continue;
    const first = images[0];
    const fallbackPath = parentDirOf(first.path);
    folders.push({
      albumId,
      albumName: folderTitleFallback(fallbackPath, first.albumName),
      albumPath: fallbackPath,
      coverThumbUrl: first.thumbUrl ?? null,
      count: images.length,
      images,
    });
  }

  for (const [folderPath, images] of zeroAlbumByPath.entries()) {
    if (seenZeroPaths.has(folderPath) || images.length === 0) continue;
    const first = images[0];
    folders.push({
      albumId: nextSyntheticId--,
      albumName: folderTitleFallback(folderPath, first.albumName),
      albumPath: folderPath,
      coverThumbUrl: first.thumbUrl ?? null,
      count: images.length,
      images,
    });
  }

  return folders;
}

export function PhotosView() {
  const photoLibrary = useAppStore((s) => s.photoLibrary);
  const photosLoading = useAppStore((s) => s.photosLoading);
  const selectedAlbumId = useAppStore((s) => s.selectedAlbumId);
  const quickLookIndex = useAppStore((s) => s.quickLookIndex);
  const setSelectedAlbumId = useAppStore((s) => s.setSelectedAlbumId);
  const setQuickLookIndex = useAppStore((s) => s.setQuickLookIndex);
  const loadPhotos = useAppStore((s) => s.loadPhotos);
  const enqueueDownloads = useAppStore((s) => s.enqueueDownloads);
  const { lang, t } = useI18n();

  const [viewMode, setViewMode] = useState<'photos' | 'albums'>('photos');
  const [focusedIdx, setFocusedIdx] = useState(0);

  const folderAlbums = useMemo(() => buildPhotoFolders(photoLibrary), [photoLibrary]);

  const activeFolder = useMemo(
    () =>
      selectedAlbumId === null
        ? null
        : folderAlbums.find((folder) => folder.albumId === selectedAlbumId) ?? null,
    [folderAlbums, selectedAlbumId],
  );

  const visibleImages = useMemo(() => {
    const imgs = photoLibrary?.images ?? [];
    if (selectedAlbumId === null) return imgs;
    return activeFolder ? activeFolder.images : imgs.filter((i) => i.albumId === selectedAlbumId);
  }, [activeFolder, photoLibrary, selectedAlbumId]);

  const selection = useMediaSelection(visibleImages);
  const hasDeleteDialogOpen = selection.deleteTargets.length > 0;

  const COLS = 5;
  const rowsCount = Math.ceil(visibleImages.length / COLS);
  const gridScrollRef = useRef<HTMLDivElement>(null);

  const rowVirtualizer = useVirtualizer({
    count: rowsCount,
    getScrollElement: () => gridScrollRef.current,
    estimateSize: () => 176,
    overscan: 4,
    initialRect: { width: 900, height: 600 },
  });

  // Spacebar toggles QuickLook preview outside multi-select; ArrowLeft/ArrowRight step photos while open
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (hasDeleteDialogOpen) return;
      if (e.code === 'Space' && viewMode === 'photos' && !selection.multiSelect && visibleImages.length > 0) {
        e.preventDefault();
        if (quickLookIndex !== null) {
          setQuickLookIndex(null);
        } else {
          setQuickLookIndex(Math.min(focusedIdx, visibleImages.length - 1));
        }
      } else if (e.key === 'Escape' && selection.multiSelect && quickLookIndex === null) {
        e.preventDefault();
        selection.exitMultiSelect();
      } else if (quickLookIndex !== null) {
        if (e.key === 'ArrowRight') {
          e.preventDefault();
          setQuickLookIndex(Math.min(visibleImages.length - 1, quickLookIndex + 1));
        } else if (e.key === 'ArrowLeft') {
          e.preventDefault();
          setQuickLookIndex(Math.max(0, quickLookIndex - 1));
        }
      }
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [
    focusedIdx,
    hasDeleteDialogOpen,
    quickLookIndex,
    selection,
    setQuickLookIndex,
    viewMode,
    visibleImages.length,
  ]);

  const activePhoto =
    quickLookIndex !== null && quickLookIndex < visibleImages.length
      ? visibleImages[quickLookIndex]
      : null;

  const handleSelectAlbum = (albumId: number | null) => {
    selection.clearSelection();
    setQuickLookIndex(null);
    setSelectedAlbumId(albumId);
    setViewMode('photos');
  };

  const handleOpenAlbumsMode = () => {
    selection.exitMultiSelect();
    setQuickLookIndex(null);
    setSelectedAlbumId(null);
    setViewMode('albums');
  };

  const defaultUploadDir = useMediaUploadDir('DCIM');
  const uploadDir = activeFolder?.albumPath || defaultUploadDir;

  return (
    <FileDropZone targetDir={uploadDir}>
      <div className="album-chips">
        <button
          type="button"
          className={`chip-btn ${viewMode === 'photos' && selectedAlbumId === null ? 'active' : ''}`}
          onClick={() => handleSelectAlbum(null)}
        >
          {t.photos.allPhotos(photoLibrary?.images.length ?? 0)}
        </button>
        <button
          type="button"
          className={`chip-btn ${viewMode === 'albums' ? 'active' : ''}`}
          aria-pressed={viewMode === 'albums'}
          style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}
          onClick={handleOpenAlbumsMode}
        >
          <FolderOpen size={12} aria-hidden="true" />
          <span>{t.photos.albumsMode(folderAlbums.length)}</span>
        </button>
        {folderAlbums.map((a) => (
          <button
            key={`${a.albumId}-${a.albumPath}`}
            type="button"
            className={`chip-btn ${viewMode === 'photos' && selectedAlbumId === a.albumId ? 'active' : ''}`}
            onClick={() => handleSelectAlbum(a.albumId)}
          >
            {a.albumName}
          </button>
        ))}
        <div
          style={{
            marginLeft: 'auto',
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            flexShrink: 0,
          }}
        >
          {viewMode === 'photos' && (
            <button
              type="button"
              className={`chip-btn ${selection.multiSelect ? 'active' : ''}`}
              aria-pressed={selection.multiSelect}
              style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}
              onClick={selection.toggleMultiSelect}
            >
              <CheckSquare size={12} aria-hidden="true" />
              <span>
                {selection.multiSelect ? t.mediaSelect.exitMultiSelect : t.mediaSelect.multiSelect}
              </span>
            </button>
          )}
          <button
            type="button"
            className="chip-btn"
            style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}
            onClick={() => void loadPhotos()}
          >
            <RefreshCw size={12} aria-hidden="true" />
            <span>{photosLoading ? t.common.syncing : t.common.refresh}</span>
          </button>
        </div>
      </div>

      {viewMode === 'photos' && activeFolder && (
        <div className="album-active-bar">
          <button type="button" className="btn" onClick={handleOpenAlbumsMode}>
            <ArrowLeft size={14} aria-hidden="true" />
            <span>{t.photos.backToAlbums}</span>
          </button>
          <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0, flex: 1 }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
              <span style={{ fontWeight: 600, fontSize: 13.5 }}>{activeFolder.albumName}</span>
              <span style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                {t.photos.folderPhotoCount(visibleImages.length)}
              </span>
            </div>
            {activeFolder.albumPath && (
              <span style={{ fontSize: 11.5, color: 'var(--text-muted)', wordBreak: 'break-all' }}>
                {activeFolder.albumPath}
              </span>
            )}
          </div>
        </div>
      )}

      {viewMode === 'photos' && selection.multiSelect && (
        <MediaSelectionBar
          selectedCount={selection.selectedItems.length}
          totalCount={visibleImages.length}
          allSelected={selection.allSelected}
          onSelectAll={selection.selectAll}
          onDownload={() => void selection.downloadSelected()}
          onCopyTo={() => void selection.copySelectedToFolder()}
          onDelete={selection.requestDeleteSelected}
          onDone={selection.exitMultiSelect}
        />
      )}

      <div ref={gridScrollRef} className="view-scroll" style={{ padding: 16 }}>
        {viewMode === 'albums' ? (
          photosLoading && folderAlbums.length === 0 ? (
            <p style={{ color: 'var(--text-muted)' }}>{t.photos.loading}</p>
          ) : folderAlbums.length === 0 ? (
            <p style={{ color: 'var(--text-muted)' }}>{t.photos.emptyAlbums}</p>
          ) : (
            <div className="album-folder-grid">
              {folderAlbums.map((folder) => (
                <button
                  key={`${folder.albumId}-${folder.albumPath}`}
                  type="button"
                  className="album-folder-card"
                  aria-label={t.photos.openAlbumAria(folder.albumName, folder.count)}
                  onClick={() => handleSelectAlbum(folder.albumId)}
                >
                  <div className="album-folder-cover">
                    {folder.coverThumbUrl ? (
                      <img src={folder.coverThumbUrl} alt={folder.albumName} loading="lazy" />
                    ) : (
                      <FolderOpen size={30} color="var(--text-muted)" aria-hidden="true" />
                    )}
                    <span className="duration-badge">{t.photos.folderPhotoCount(folder.count)}</span>
                  </div>
                  <div className="album-folder-body">
                    <div className="album-folder-title">
                      <FolderOpen size={14} aria-hidden="true" style={{ flexShrink: 0 }} />
                      <span title={folder.albumName}>{folder.albumName}</span>
                    </div>
                    {folder.albumPath && (
                      <div className="album-folder-path" title={folder.albumPath}>
                        {folder.albumPath}
                      </div>
                    )}
                  </div>
                </button>
              ))}
            </div>
          )
        ) : photosLoading && visibleImages.length === 0 ? (
          <p style={{ color: 'var(--text-muted)' }}>{t.photos.loading}</p>
        ) : visibleImages.length === 0 ? (
          <p style={{ color: 'var(--text-muted)' }}>{t.photos.empty}</p>
        ) : (
          <div style={{ height: rowVirtualizer.getTotalSize(), position: 'relative' }}>
            {rowVirtualizer.getVirtualItems().map((vRow) => {
              const start = vRow.index * COLS;
              const slice = visibleImages.slice(start, start + COLS);
              return (
                <div
                  key={vRow.key}
                  style={{
                    position: 'absolute',
                    top: 0,
                    left: 0,
                    width: '100%',
                    height: 166,
                    transform: `translateY(${vRow.start}px)`,
                    display: 'grid',
                    gridTemplateColumns: `repeat(${COLS}, minmax(0, 1fr))`,
                    gap: 10,
                  }}
                >
                  {slice.map((img, offset) => {
                    const globalIdx = start + offset;
                    const isSelected = selection.multiSelect && selection.selectedSet.has(img.path);
                    return (
                      <button
                        key={`${img.mediaId}-${img.path}`}
                        type="button"
                        className={`photo-tile ${isSelected ? 'selected' : ''}`}
                        aria-label={
                          selection.multiSelect
                            ? t.mediaSelect.selectItemAria(img.title)
                            : t.photos.previewAria(img.title)
                        }
                        aria-pressed={selection.multiSelect ? isSelected : undefined}
                        onFocus={() => setFocusedIdx(globalIdx)}
                        onClick={() => {
                          setFocusedIdx(globalIdx);
                          if (selection.multiSelect) {
                            selection.toggleSelectPath(img.path);
                          } else {
                            setQuickLookIndex(globalIdx);
                          }
                        }}
                      >
                        <img src={img.thumbUrl} alt={img.title} loading="lazy" />
                        {selection.multiSelect && (
                          <span
                            className={`media-check-badge ${isSelected ? 'checked' : ''}`}
                            aria-hidden="true"
                          >
                            {isSelected ? <Check size={13} strokeWidth={2.75} /> : null}
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>
              );
            })}
          </div>
        )}
      </div>

      <ModalDialog
        open={activePhoto !== null}
        onClose={() => setQuickLookIndex(null)}
        title={activePhoto?.title ?? t.photos.previewTitleFallback}
        subtitle={activePhoto?.path}
        titleId="photo-quicklook-title"
        maxWidth="920px"
      >
        {activePhoto && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                background: '#090b0f',
                borderRadius: 8,
                maxHeight: '62vh',
                overflow: 'hidden',
              }}
            >
              <img
                src={activePhoto.streamUrl}
                alt={activePhoto.title}
                style={{ maxWidth: '100%', maxHeight: '62vh', objectFit: 'contain' }}
              />
            </div>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', flexWrap: 'wrap', gap: 10 }}>
              <div style={{ fontSize: 12.5, color: 'var(--text-secondary)' }}>
                {activePhoto.width}×{activePhoto.height} • {formatBytes(activePhoto.fileSize)} •{' '}
                {formatTimestamp(activePhoto.dateTaken || activePhoto.modifiedTimestamp, lang)}
              </div>
              <div style={{ display: 'flex', gap: 8 }}>
                <button
                  type="button"
                  className="btn"
                  disabled={quickLookIndex === null || quickLookIndex <= 0}
                  onClick={() => setQuickLookIndex(Math.max(0, (quickLookIndex ?? 0) - 1))}
                >
                  <ChevronLeft size={14} aria-hidden="true" />
                  <span>{t.photos.prev}</span>
                </button>
                <button
                  type="button"
                  className="btn"
                  disabled={quickLookIndex === null || quickLookIndex >= visibleImages.length - 1}
                  onClick={() =>
                    setQuickLookIndex(Math.min(visibleImages.length - 1, (quickLookIndex ?? 0) + 1))
                  }
                >
                  <span>{t.photos.next}</span>
                  <ChevronRight size={14} aria-hidden="true" />
                </button>
                <button
                  type="button"
                  className="btn danger"
                  onClick={() =>
                    selection.requestDeleteSingle({
                      path: activePhoto.path,
                      name: activePhoto.title,
                    })
                  }
                >
                  <Trash2 size={14} aria-hidden="true" />
                  <span>{t.common.delete}</span>
                </button>
                <button
                  type="button"
                  className="btn primary"
                  onClick={() =>
                    void enqueueDownloads([
                      {
                        remotePath: activePhoto.path,
                        isDirectory: false,
                        fileSize: activePhoto.fileSize,
                      },
                    ])
                  }
                >
                  <Download size={14} aria-hidden="true" />
                  <span>{t.common.download}</span>
                </button>
              </div>
            </div>
          </div>
        )}
      </ModalDialog>

      <DeleteFilesDialog
        open={hasDeleteDialogOpen}
        items={selection.deleteTargets}
        titleId="photos-delete-dialog-title"
        onClose={selection.closeDeleteDialog}
        onConfirm={selection.confirmDelete}
      />
    </FileDropZone>
  );
}
