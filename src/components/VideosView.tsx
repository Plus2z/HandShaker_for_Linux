import { Check, CheckSquare, Download, ExternalLink, Play, RefreshCw } from 'lucide-react';
import { formatBytes, formatDuration } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import { useMediaSelection } from '../lib/useMediaSelection';
import { useAppStore } from '../store/useAppStore';
import { DeleteFilesDialog } from './DeleteFilesDialog';
import { MediaSelectionBar } from './MediaSelectionBar';
import { ModalDialog } from './ModalDialog';
import { FileDropZone, useMediaUploadDir } from './FileDropZone';

export function VideosView() {
  const videoLibrary = useAppStore((s) => s.videoLibrary);
  const videosLoading = useAppStore((s) => s.videosLoading);
  const activeVideo = useAppStore((s) => s.activeVideo);
  const mediaSupport = useAppStore((s) => s.mediaSupport);
  const setActiveVideo = useAppStore((s) => s.setActiveVideo);
  const loadVideos = useAppStore((s) => s.loadVideos);
  const enqueueDownloads = useAppStore((s) => s.enqueueDownloads);
  const openMediaExternally = useAppStore((s) => s.openMediaExternally);
  const { t } = useI18n();

  const videos = videoLibrary?.videos ?? [];
  const canEmbedVideo = mediaSupport?.embeddedVideo === true;
  const selection = useMediaSelection(videos);

  const uploadDir = useMediaUploadDir('Movies');

  return (
    <FileDropZone targetDir={uploadDir}>
      <div className="toolbar" style={{ justifyContent: 'space-between' }}>
        <strong>{t.videos.heading(videos.length)}</strong>
        <div style={{ display: 'flex', gap: 8 }}>
          <button
            type="button"
            className={`btn ${selection.multiSelect ? 'primary' : ''}`}
            aria-pressed={selection.multiSelect}
            onClick={selection.toggleMultiSelect}
          >
            <CheckSquare size={14} aria-hidden="true" />
            <span>
              {selection.multiSelect ? t.mediaSelect.exitMultiSelect : t.mediaSelect.multiSelect}
            </span>
          </button>
          <button type="button" className="btn" onClick={() => void loadVideos()}>
            <RefreshCw size={14} aria-hidden="true" />
            <span>{videosLoading ? t.common.syncing : t.common.refresh}</span>
          </button>
        </div>
      </div>

      {selection.multiSelect && (
        <MediaSelectionBar
          selectedCount={selection.selectedItems.length}
          totalCount={videos.length}
          allSelected={selection.allSelected}
          onSelectAll={selection.selectAll}
          onDownload={() => void selection.downloadSelected()}
          onCopyTo={() => void selection.copySelectedToFolder()}
          onDelete={selection.requestDeleteSelected}
          onDone={selection.exitMultiSelect}
        />
      )}

      <div className="view-scroll">
        {videosLoading && videos.length === 0 ? (
          <p style={{ color: 'var(--text-muted)' }}>{t.videos.loading}</p>
        ) : videos.length === 0 ? (
          <p style={{ color: 'var(--text-muted)' }}>{t.videos.empty}</p>
        ) : (
          <div className="video-grid">
            {videos.map((v) => {
              const isSelected = selection.multiSelect && selection.selectedSet.has(v.path);
              return (
                <article
                  key={`${v.mediaId}-${v.path}`}
                  className={`video-card ${isSelected ? 'selected' : ''}`}
                >
                  <button
                    type="button"
                    className="video-thumb-wrap"
                    aria-label={
                      selection.multiSelect
                        ? t.mediaSelect.selectItemAria(v.name)
                        : t.videos.playAria(v.name)
                    }
                    aria-pressed={selection.multiSelect ? isSelected : undefined}
                    onClick={() =>
                      selection.multiSelect ? selection.toggleSelectPath(v.path) : setActiveVideo(v)
                    }
                  >
                    <img src={v.thumbUrl} alt={v.name} loading="lazy" />
                    {selection.multiSelect && (
                      <span
                        className={`media-check-badge ${isSelected ? 'checked' : ''}`}
                        aria-hidden="true"
                      >
                        {isSelected ? <Check size={13} strokeWidth={2.75} /> : null}
                      </span>
                    )}
                    <span className="duration-badge">{formatDuration(v.durationSecs)}</span>
                  </button>
                  <div
                    style={{
                      padding: 10,
                      display: 'flex',
                      flexDirection: 'column',
                      gap: 6,
                      cursor: selection.multiSelect ? 'pointer' : undefined,
                    }}
                    onClick={
                      selection.multiSelect ? () => selection.toggleSelectPath(v.path) : undefined
                    }
                  >
                    <div
                      title={v.name}
                      style={{
                        fontWeight: 500,
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                    >
                      {v.name}
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', fontSize: 12, color: 'var(--text-muted)', minHeight: 28 }}>
                      <span>{formatBytes(v.fileSize)}</span>
                      {!selection.multiSelect && (
                        <div style={{ display: 'flex', gap: 4 }}>
                          <button
                            type="button"
                            className="icon-btn"
                            aria-label={t.videos.playAria(v.name)}
                            onClick={() => setActiveVideo(v)}
                          >
                            <Play size={14} aria-hidden="true" />
                          </button>
                          <button
                            type="button"
                            className="icon-btn"
                            aria-label={t.videos.downloadAria(v.name)}
                            onClick={() =>
                              void enqueueDownloads([
                                { remotePath: v.path, isDirectory: false, fileSize: v.fileSize },
                              ])
                            }
                          >
                            <Download size={14} aria-hidden="true" />
                          </button>
                        </div>
                      )}
                    </div>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </div>

      <ModalDialog
        open={activeVideo !== null}
        onClose={() => setActiveVideo(null)}
        title={activeVideo?.name ?? t.videos.playerTitleFallback}
        titleId="video-player-title"
        maxWidth="900px"
      >
        {activeVideo && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            {canEmbedVideo ? (
              <video
                controls
                autoPlay
                preload="metadata"
                src={activeVideo.streamUrl}
                style={{ width: '100%', maxHeight: '62vh', borderRadius: 8, background: '#000' }}
              />
            ) : (
              <div
                className="panel"
                style={{
                  padding: '20px 18px',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 10,
                  background: 'var(--bg-elevated)',
                }}
              >
                <div style={{ fontWeight: 600 }}>{t.videos.inAppUnavailable}</div>
                <p style={{ margin: 0, fontSize: 13, color: 'var(--text-secondary)', lineHeight: 1.5 }}>
                  {mediaSupport?.missingElements?.length
                    ? t.videos.missingGstreamer(mediaSupport.missingElements)
                    : (mediaSupport?.reason ?? t.videos.checkingGstreamer)}
                </p>
              </div>
            )}
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
              <span style={{ fontSize: 12.5, color: 'var(--text-secondary)' }}>
                {activeVideo.width}×{activeVideo.height} • {formatBytes(activeVideo.fileSize)}
              </span>
              <div style={{ display: 'flex', gap: 8 }}>
                <button
                  type="button"
                  className="btn"
                  onClick={() => void openMediaExternally(activeVideo.streamUrl)}
                >
                  <ExternalLink size={14} aria-hidden="true" />
                  <span>{t.videos.openInSystemPlayer}</span>
                </button>
                <button
                  type="button"
                  className="btn primary"
                  onClick={() =>
                    void enqueueDownloads([
                      {
                        remotePath: activeVideo.path,
                        isDirectory: false,
                        fileSize: activeVideo.fileSize,
                      },
                    ])
                  }
                >
                  <Download size={14} aria-hidden="true" />
                  <span>{t.videos.downloadVideo}</span>
                </button>
              </div>
            </div>
          </div>
        )}
      </ModalDialog>

      <DeleteFilesDialog
        open={selection.deleteTargets.length > 0}
        items={selection.deleteTargets}
        titleId="videos-delete-dialog-title"
        onClose={selection.closeDeleteDialog}
        onConfirm={selection.confirmDelete}
      />
    </FileDropZone>
  );
}
