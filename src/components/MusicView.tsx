import { CheckSquare, Download, ExternalLink, Play, RefreshCw, X } from 'lucide-react';
import { formatBytes, formatDuration } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import { useMediaSelection } from '../lib/useMediaSelection';
import { useAppStore } from '../store/useAppStore';
import { DeleteFilesDialog } from './DeleteFilesDialog';
import { MediaSelectionBar } from './MediaSelectionBar';
import { FileDropZone, useMediaUploadDir } from './FileDropZone';

export function MusicView() {
  const audioLibrary = useAppStore((s) => s.audioLibrary);
  const audioLoading = useAppStore((s) => s.audioLoading);
  const activeAudio = useAppStore((s) => s.activeAudio);
  const mediaSupport = useAppStore((s) => s.mediaSupport);
  const setActiveAudio = useAppStore((s) => s.setActiveAudio);
  const loadAudio = useAppStore((s) => s.loadAudio);
  const enqueueDownloads = useAppStore((s) => s.enqueueDownloads);
  const openMediaExternally = useAppStore((s) => s.openMediaExternally);
  const { t } = useI18n();

  const tracks = audioLibrary?.tracks ?? [];
  const albumById = new Map((audioLibrary?.albums ?? []).map((a) => [a.albumId, a]));
  const canEmbedAudio = mediaSupport?.embeddedAudio === true;
  const selection = useMediaSelection(tracks);

  const uploadDir = useMediaUploadDir('Music');

  return (
    <FileDropZone targetDir={uploadDir}>
      <div className="toolbar" style={{ justifyContent: 'space-between' }}>
        <strong>
          {t.music.heading(tracks.length, audioLibrary?.albums.length ?? 0)}
        </strong>
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
          <button type="button" className="btn" onClick={() => void loadAudio()}>
            <RefreshCw size={14} aria-hidden="true" />
            <span>{audioLoading ? t.common.syncing : t.common.refresh}</span>
          </button>
        </div>
      </div>

      {selection.multiSelect && (
        <MediaSelectionBar
          selectedCount={selection.selectedItems.length}
          totalCount={tracks.length}
          allSelected={selection.allSelected}
          onSelectAll={selection.selectAll}
          onDownload={() => void selection.downloadSelected()}
          onCopyTo={() => void selection.copySelectedToFolder()}
          onDelete={selection.requestDeleteSelected}
          onDone={selection.exitMultiSelect}
        />
      )}

      <div className="table-scroller">
        <table className="hs-table">
          <caption className="sr-only">{t.music.tableCaption}</caption>
          <colgroup>
            <col style={{ width: '44px' }} />
            <col style={{ width: '38%' }} />
            <col style={{ width: '24%' }} />
            <col style={{ width: '16%' }} />
            <col style={{ width: '10%' }} />
            <col style={{ width: '12%' }} />
          </colgroup>
          <thead>
            <tr>
              <th scope="col" style={{ paddingLeft: 12 }}>
                {selection.multiSelect ? (
                  <input
                    type="checkbox"
                    aria-label={t.mediaSelect.selectAllAria}
                    checked={selection.allSelected}
                    onChange={(e) => selection.selectAll(e.target.checked)}
                  />
                ) : null}
              </th>
              <th scope="col" style={{ padding: '8px 12px' }}>{t.music.colTitle}</th>
              <th scope="col" style={{ padding: '8px 12px' }}>{t.music.colArtist}</th>
              <th scope="col" style={{ padding: '8px 12px' }}>{t.music.colDuration}</th>
              <th scope="col" style={{ padding: '8px 12px' }}>{t.music.colSize}</th>
              <th scope="col" style={{ padding: '8px 12px' }}>{t.music.colActions}</th>
            </tr>
          </thead>
          <tbody>
            {audioLoading && tracks.length === 0 ? (
              <tr>
                <td colSpan={6} style={{ padding: 24, textAlign: 'center', color: 'var(--text-muted)' }}>
                  {t.music.loading}
                </td>
              </tr>
            ) : tracks.length === 0 ? (
              <tr>
                <td colSpan={6} style={{ padding: 24, textAlign: 'center', color: 'var(--text-muted)' }}>
                  {t.music.empty}
                </td>
              </tr>
            ) : (
              tracks.map((track) => {
                const isPlaying = activeAudio?.path === track.path;
                const isSelected = selection.multiSelect && selection.selectedSet.has(track.path);
                const rowHighlighted = selection.multiSelect ? isSelected : isPlaying;
                const album = albumById.get(track.albumId);
                return (
                  <tr
                    key={`${track.mediaId}-${track.path}`}
                    className={`hs-row ${rowHighlighted ? 'selected' : ''}`}
                    onClick={
                      selection.multiSelect
                        ? () => selection.toggleSelectPath(track.path)
                        : undefined
                    }
                  >
                    <td
                      style={{ paddingLeft: 12 }}
                      onClick={selection.multiSelect ? (e) => e.stopPropagation() : undefined}
                    >
                      {selection.multiSelect ? (
                        <input
                          type="checkbox"
                          aria-label={t.mediaSelect.selectItemAria(track.title)}
                          checked={isSelected}
                          onChange={() => selection.toggleSelectPath(track.path)}
                        />
                      ) : album ? (
                        <img
                          src={album.thumbUrl}
                          alt=""
                          width={24}
                          height={24}
                          style={{ borderRadius: 4, objectFit: 'cover', display: 'block' }}
                        />
                      ) : null}
                    </td>
                    <td title={track.title} style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontWeight: 500 }}>
                      {track.title}
                    </td>
                    <td style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: 'var(--text-secondary)' }}>
                      {track.artist}
                    </td>
                    <td style={{ color: 'var(--text-muted)', fontSize: 12.5 }}>
                      {formatDuration(track.durationSecs)}
                    </td>
                    <td style={{ color: 'var(--text-muted)', fontSize: 12.5 }}>
                      {formatBytes(track.fileSize)}
                    </td>
                    <td>
                      <div style={{ display: 'flex', gap: 4 }}>
                        <button
                          type="button"
                          className="icon-btn"
                          aria-label={t.music.playAria(track.title)}
                          onClick={(e) => {
                            e.stopPropagation();
                            setActiveAudio(track);
                          }}
                        >
                          <Play size={14} aria-hidden="true" />
                        </button>
                        <button
                          type="button"
                          className="icon-btn"
                          aria-label={t.music.downloadAria(track.title)}
                          onClick={(e) => {
                            e.stopPropagation();
                            void enqueueDownloads([
                              { remotePath: track.path, isDirectory: false, fileSize: track.fileSize },
                            ]);
                          }}
                        >
                          <Download size={14} aria-hidden="true" />
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>

      {activeAudio && (
        <div className="audio-player-bar">
          <div style={{ minWidth: 160, maxWidth: 260 }}>
            <div style={{ fontWeight: 600, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {activeAudio.title}
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>{activeAudio.artist}</div>
          </div>
          {canEmbedAudio ? (
            <audio controls autoPlay src={activeAudio.streamUrl} style={{ flex: 1, height: 34 }} />
          ) : (
            <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 10, flexWrap: 'wrap' }}>
              <span style={{ fontSize: 12.5, color: 'var(--text-secondary)' }}>
                {mediaSupport?.missingElements?.length
                  ? t.music.missingGstreamerAudio(mediaSupport.missingElements)
                  : (mediaSupport?.reason ?? t.music.audioFallback)}
              </span>
              <div style={{ display: 'flex', gap: 8 }}>
                <button
                  type="button"
                  className="btn"
                  onClick={() => void openMediaExternally(activeAudio.streamUrl)}
                >
                  <ExternalLink size={14} aria-hidden="true" />
                  <span>{t.music.openInSystemPlayer}</span>
                </button>
                <button
                  type="button"
                  className="btn primary"
                  onClick={() =>
                    void enqueueDownloads([
                      {
                        remotePath: activeAudio.path,
                        isDirectory: false,
                        fileSize: activeAudio.fileSize,
                      },
                    ])
                  }
                >
                  <Download size={14} aria-hidden="true" />
                  <span>{t.common.download}</span>
                </button>
              </div>
            </div>
          )}
          <button
            type="button"
            className="icon-btn"
            aria-label={t.music.closePlayer}
            onClick={() => setActiveAudio(null)}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      )}

      <DeleteFilesDialog
        open={selection.deleteTargets.length > 0}
        items={selection.deleteTargets}
        titleId="music-delete-dialog-title"
        onClose={selection.closeDeleteDialog}
        onConfirm={selection.confirmDelete}
      />
    </FileDropZone>
  );
}
