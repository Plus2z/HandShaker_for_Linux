import { useMemo, useRef, useState } from 'react';
import { FileDropZone } from './FileDropZone';
import { useVirtualizer } from '@tanstack/react-virtual';
import {
  ArrowDown,
  ArrowUp,
  ChevronRight,
  Download,
  File as FileIcon,
  Folder,
  FolderPlus,
  Pencil,
  RefreshCw,
  Trash2,
  Upload,
} from 'lucide-react';
import {
  formatBytes,
  formatTimestamp,
  joinRemotePath,
  parentRemotePath,
  splitFilenameExt,
} from '../lib/format';
import { ipc } from '../lib/ipc';
import { useI18n } from '../lib/useI18n';
import { sortFileEntries, useAppStore, type SortKey } from '../store/useAppStore';
import { DeleteFilesDialog } from './DeleteFilesDialog';
import { ModalDialog } from './ModalDialog';

export function FilesView() {
  const deviceInfo = useAppStore((s) => s.deviceInfo);
  const currentPath = useAppStore((s) => s.currentPath);
  const files = useAppStore((s) => s.files);
  const filesLoading = useAppStore((s) => s.filesLoading);
  const selectedPaths = useAppStore((s) => s.selectedPaths);
  const sortKey = useAppStore((s) => s.sortKey);
  const sortDir = useAppStore((s) => s.sortDir);
  const fileSearch = useAppStore((s) => s.fileSearch);
  const setFileSearch = useAppStore((s) => s.setFileSearch);
  const setSort = useAppStore((s) => s.setSort);
  const toggleSelectPath = useAppStore((s) => s.toggleSelectPath);
  const selectAllFiles = useAppStore((s) => s.selectAllFiles);
  const openDir = useAppStore((s) => s.openDir);
  const createFolder = useAppStore((s) => s.createFolder);
  const renameFile = useAppStore((s) => s.renameFile);
  const deleteFiles = useAppStore((s) => s.deleteFiles);
  const enqueueDownloads = useAppStore((s) => s.enqueueDownloads);
  const { lang, t } = useI18n();

  const rootPath = deviceInfo?.rootPath || '/storage/emulated/0';
  const sorted = useMemo(
    () => sortFileEntries(files, sortKey, sortDir, fileSearch),
    [files, sortKey, sortDir, fileSearch],
  );

  const [newFolderOpen, setNewFolderOpen] = useState(false);
  const [newFolderName, setNewFolderName] = useState('');
  const [renameTarget, setRenameTarget] = useState<{ path: string; name: string } | null>(null);
  const [renameValue, setRenameValue] = useState('');
  const [deleteOpen, setDeleteOpen] = useState(false);

  const scrollRef = useRef<HTMLDivElement>(null);
  const rowVirtualizer = useVirtualizer({
    count: sorted.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => 40,
    overscan: 10,
    initialRect: { width: 900, height: 600 },
  });


  const crumbs = useMemo(() => {
    const cleanRoot = rootPath.replace(/\/+$/, '');
    const rel = currentPath.startsWith(cleanRoot)
      ? currentPath.slice(cleanRoot.length).replace(/^\/+/, '')
      : '';
    const parts = rel ? rel.split('/').filter(Boolean) : [];
    const list = [{ label: t.files.phoneStorage, path: cleanRoot }];
    let acc = cleanRoot;
    for (const part of parts) {
      acc = `${acc}/${part}`;
      list.push({ label: part, path: acc });
    }
    return list;
  }, [currentPath, rootPath, t.files.phoneStorage]);

  const selectedItems = useMemo(
    () => sorted.filter((f) => selectedPaths.includes(f.path)),
    [sorted, selectedPaths],
  );

  const handleUploadPick = async (directory: boolean) => {
    const picks = await ipc.pickLocalPaths(directory, true);
    if (picks.length > 0) {
      await useAppStore.getState().enqueueUploads(picks);
    }
  };

  const handleDownloadSelected = async () => {
    if (selectedItems.length === 0) return;
    await enqueueDownloads(
      selectedItems.map((it) => ({
        remotePath: it.path,
        isDirectory: it.isDirectory,
        fileSize: it.fileSize,
      })),
    );
  };

  const renderSortIcon = (col: SortKey) => {
    if (sortKey !== col) return null;
    return sortDir === 'asc' ? (
      <ArrowUp size={13} aria-hidden="true" />
    ) : (
      <ArrowDown size={13} aria-hidden="true" />
    );
  };

  const virtualRows = rowVirtualizer.getVirtualItems();
  const paddingTop = virtualRows.length > 0 ? virtualRows[0].start : 0;
  const paddingBottom =
    virtualRows.length > 0
      ? Math.max(0, rowVirtualizer.getTotalSize() - virtualRows[virtualRows.length - 1].end)
      : 0;

  return (
    <FileDropZone targetDir={currentPath}>
      <div className="toolbar">
        <button
          type="button"
          className="btn"
          disabled={currentPath === rootPath}
          onClick={() => void openDir(parentRemotePath(currentPath, rootPath))}
          title={t.files.upTitle}
        >
          {t.files.up}
        </button>

        <nav className="breadcrumbs" aria-label={t.files.breadcrumbsAria}>
          {crumbs.map((c, idx) => (
            <span key={c.path} style={{ display: 'inline-flex', alignItems: 'center', gap: 2 }}>
              {idx > 0 && <ChevronRight size={13} aria-hidden="true" color="var(--text-muted)" />}
              <button
                type="button"
                className="crumb-btn"
                onClick={() => void openDir(c.path)}
              >
                {c.label}
              </button>
            </span>
          ))}
        </nav>

        <input
          type="search"
          aria-label={t.files.filterAria}
          className="search-input"
          placeholder={t.files.filterPlaceholder}
          value={fileSearch}
          onChange={(e) => setFileSearch(e.target.value)}
        />

        <button type="button" className="btn" onClick={() => void openDir(currentPath)} title={t.files.refreshFolderTitle}>
          <RefreshCw size={14} aria-hidden="true" />
        </button>

        <button type="button" className="btn" onClick={() => { setNewFolderName(''); setNewFolderOpen(true); }}>
          <FolderPlus size={14} aria-hidden="true" />
          <span>{t.files.newFolder}</span>
        </button>

        <button type="button" className="btn" onClick={() => void handleUploadPick(false)}>
          <Upload size={14} aria-hidden="true" />
          <span>{t.files.uploadFiles}</span>
        </button>

        <button type="button" className="btn" onClick={() => void handleUploadPick(true)}>
          <Folder size={14} aria-hidden="true" />
          <span>{t.files.uploadFolder}</span>
        </button>

        <button
          type="button"
          className="btn primary"
          disabled={selectedItems.length === 0}
          onClick={() => void handleDownloadSelected()}
        >
          <Download size={14} aria-hidden="true" />
          <span>{t.files.downloadCount(selectedItems.length)}</span>
        </button>

        <button
          type="button"
          className="btn"
          disabled={selectedItems.length !== 1}
          onClick={() => {
            const target = selectedItems[0];
            if (!target) return;
            setRenameTarget({ path: target.path, name: target.name });
            setRenameValue(target.name);
          }}
        >
          <Pencil size={14} aria-hidden="true" />
          <span>{t.files.rename}</span>
        </button>

        <button
          type="button"
          className="btn danger"
          disabled={selectedItems.length === 0}
          onClick={() => setDeleteOpen(true)}
        >
          <Trash2 size={14} aria-hidden="true" />
          <span>{t.files.delete}</span>
        </button>
      </div>


      <div ref={scrollRef} className="table-scroller">
        <table className="hs-table">
          <caption className="sr-only">{t.files.tableCaption(currentPath)}</caption>
          <colgroup>
            <col style={{ width: '40px' }} />
            <col style={{ width: '56%' }} />
            <col style={{ width: '16%' }} />
            <col style={{ width: '28%' }} />
          </colgroup>
          <thead>
            <tr>
              <th scope="col" style={{ paddingLeft: 12 }}>
                <input
                  type="checkbox"
                  aria-label={t.files.selectAllAria}
                  checked={sorted.length > 0 && selectedItems.length === sorted.length}
                  onChange={(e) => selectAllFiles(e.target.checked)}
                />
              </th>
              <th
                scope="col"
                aria-sort={sortKey === 'name' ? (sortDir === 'asc' ? 'ascending' : 'descending') : undefined}
              >
                <button type="button" onClick={() => setSort('name')}>
                  <span>{t.files.colName}</span>
                  {renderSortIcon('name')}
                </button>
              </th>
              <th
                scope="col"
                aria-sort={sortKey === 'size' ? (sortDir === 'asc' ? 'ascending' : 'descending') : undefined}
              >
                <button type="button" onClick={() => setSort('size')}>
                  <span>{t.files.colSize}</span>
                  {renderSortIcon('size')}
                </button>
              </th>
              <th
                scope="col"
                aria-sort={sortKey === 'modified' ? (sortDir === 'asc' ? 'ascending' : 'descending') : undefined}
              >
                <button type="button" onClick={() => setSort('modified')}>
                  <span>{t.files.colModified}</span>
                  {renderSortIcon('modified')}
                </button>
              </th>
            </tr>
          </thead>
          <tbody>
            {filesLoading && sorted.length === 0 ? (
              <tr>
                <td colSpan={4} style={{ padding: 24, textAlign: 'center', color: 'var(--text-muted)' }}>
                  {t.files.loadingDir}
                </td>
              </tr>
            ) : sorted.length === 0 ? (
              <tr>
                <td colSpan={4} style={{ padding: 24, textAlign: 'center', color: 'var(--text-muted)' }}>
                  {fileSearch ? t.files.noMatchingFiles : t.files.emptyFolder}
                </td>
              </tr>
            ) : (
              <>
                {paddingTop > 0 && (
                  <tr aria-hidden="true">
                    <td colSpan={4} style={{ height: paddingTop, padding: 0, border: 0 }} />
                  </tr>
                )}
                {virtualRows.map((vRow) => {
                  const item = sorted[vRow.index];
                  const isSel = selectedPaths.includes(item.path);
                  const [stem, ext] = splitFilenameExt(item.name, item.isDirectory);
                  return (
                    <tr
                      key={item.path}
                      className={`hs-row ${isSel ? 'selected' : ''}`}
                      onClick={(e) => toggleSelectPath(item.path, e.ctrlKey || e.metaKey)}
                      onDoubleClick={() => {
                        if (item.isDirectory) {
                          void openDir(item.path);
                        } else {
                          void enqueueDownloads([
                            {
                              remotePath: item.path,
                              isDirectory: false,
                              fileSize: item.fileSize,
                            },
                          ]);
                        }
                      }}
                    >
                      <td onClick={(e) => e.stopPropagation()}>
                        <input
                          type="checkbox"
                          aria-label={t.files.selectEntryAria(item.name)}
                          checked={isSel}
                          onChange={() => toggleSelectPath(item.path, true)}
                        />
                      </td>
                      <td>
                        <div className="filename-cell" title={item.name}>
                          {item.isDirectory ? (
                            <Folder size={16} color="var(--accent)" aria-hidden="true" />
                          ) : (
                            <FileIcon size={15} color="var(--text-muted)" aria-hidden="true" />
                          )}
                          <span className="filename-text">
                            <span className="filename-stem">{stem}</span>
                            {ext && <span className="filename-ext">{ext}</span>}
                          </span>
                        </div>
                      </td>
                      <td style={{ color: 'var(--text-secondary)', fontSize: 12.5 }}>
                        {item.isDirectory ? t.files.folderType : formatBytes(item.fileSize)}
                      </td>
                      <td style={{ color: 'var(--text-muted)', fontSize: 12.5 }}>
                        {formatTimestamp(item.modifiedTimestamp, lang)}
                      </td>
                    </tr>
                  );
                })}
                {paddingBottom > 0 && (
                  <tr aria-hidden="true">
                    <td colSpan={4} style={{ height: paddingBottom, padding: 0, border: 0 }} />
                  </tr>
                )}
              </>
            )}
          </tbody>
        </table>
      </div>

      <ModalDialog
        open={newFolderOpen}
        onClose={() => setNewFolderOpen(false)}
        title={t.files.newFolderTitle}
        titleId="new-folder-title"
      >
        <form
          onSubmit={(e) => {
            e.preventDefault();
            const clean = newFolderName.trim();
            if (!clean) return;
            setNewFolderOpen(false);
            void createFolder(joinRemotePath(currentPath, clean));
          }}
        >
          <input
            autoFocus
            className="text-input"
            style={{ width: '100%' }}
            placeholder={t.files.folderNamePlaceholder}
            value={newFolderName}
            onChange={(e) => setNewFolderName(e.target.value)}
          />
          <div className="dialog-actions">
            <button type="button" className="btn" onClick={() => setNewFolderOpen(false)}>
              {t.common.cancel}
            </button>
            <button type="submit" className="btn primary">
              {t.files.create}
            </button>
          </div>
        </form>
      </ModalDialog>

      <ModalDialog
        open={renameTarget !== null}
        onClose={() => setRenameTarget(null)}
        title={t.files.renameTitle}
        titleId="rename-dialog-title"
      >
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (!renameTarget) return;
            const clean = renameValue.trim();
            if (!clean || clean === renameTarget.name) {
              setRenameTarget(null);
              return;
            }
            const dst = joinRemotePath(parentRemotePath(renameTarget.path, '/'), clean);
            setRenameTarget(null);
            void renameFile(renameTarget.path, dst);
          }}
        >
          <input
            autoFocus
            className="text-input"
            style={{ width: '100%' }}
            value={renameValue}
            onChange={(e) => setRenameValue(e.target.value)}
          />
          <div className="dialog-actions">
            <button type="button" className="btn" onClick={() => setRenameTarget(null)}>
              {t.common.cancel}
            </button>
            <button type="submit" className="btn primary">
              {t.files.save}
            </button>
          </div>
        </form>
      </ModalDialog>

      <DeleteFilesDialog
        open={deleteOpen}
        items={selectedItems.map((it) => ({ path: it.path, name: it.name }))}
        titleId="delete-dialog-title"
        onClose={() => setDeleteOpen(false)}
        onConfirm={deleteFiles}
      />
    </FileDropZone>
  );
}
