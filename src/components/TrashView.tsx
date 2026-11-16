import { useEffect, useMemo, useState } from 'react';
import {
  ArrowLeft,
  File as FileIcon,
  Folder,
  RefreshCw,
  RotateCcw,
  Trash2,
} from 'lucide-react';
import { formatBytes, formatTimestamp, formatTrashSourcePhone } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import { useAppStore } from '../store/useAppStore';
import type { TrashItem } from '../types/api';
import { ModalDialog } from './ModalDialog';

function formatTrashTimestamp(item: TrashItem, lang: 'en' | 'zh-CN'): string {
  if (item.deletionDate) {
    const parsed = Date.parse(item.deletionDate);
    if (!Number.isNaN(parsed)) {
      return formatTimestamp(parsed, lang);
    }
    return item.deletionDate;
  }
  return formatTimestamp(item.modifiedTimestampMs, lang);
}

export function TrashView() {
  const settings = useAppStore((s) => s.settings);
  const connection = useAppStore((s) => s.connection);
  const trashItems = useAppStore((s) => s.trashItems);
  const trashLoading = useAppStore((s) => s.trashLoading);
  const trashBusy = useAppStore((s) => s.trashBusy);
  const setTab = useAppStore((s) => s.setTab);
  const loadTrash = useAppStore((s) => s.loadTrash);
  const restoreTrash = useAppStore((s) => s.restoreTrash);
  const deleteTrash = useAppStore((s) => s.deleteTrash);
  const emptyTrash = useAppStore((s) => s.emptyTrash);
  const { lang, t } = useI18n();

  const [filterText, setFilterText] = useState('');
  const [selectedNames, setSelectedNames] = useState<string[]>([]);
  const [confirmDeleteTargets, setConfirmDeleteTargets] = useState<string[] | null>(null);
  const [confirmEmptyOpen, setConfirmEmptyOpen] = useState(false);

  const isConnected = connection.status === 'connected';
  const currentDeviceUuid = connection.handshake?.deviceUuid?.trim() || '';

  useEffect(() => {
    void loadTrash();
  }, [loadTrash, settings.trashDir]);

  useEffect(() => {
    const valid = new Set(trashItems.map((it) => it.entryName));
    setSelectedNames((prev) => prev.filter((name) => valid.has(name)));
  }, [trashItems]);

  const visibleItems = useMemo(() => {
    const query = filterText.trim().toLowerCase();
    if (!query) return trashItems;
    return trashItems.filter(
      (item) =>
        item.entryName.toLowerCase().includes(query) ||
        (item.originalPath?.toLowerCase().includes(query) ?? false) ||
        (item.deviceName?.toLowerCase().includes(query) ?? false) ||
        (item.deviceModel?.toLowerCase().includes(query) ?? false) ||
        (item.deviceUuid?.toLowerCase().includes(query) ?? false),
    );
  }, [trashItems, filterText]);

  const allVisibleSelected =
    visibleItems.length > 0 && visibleItems.every((it) => selectedNames.includes(it.entryName));

  const toggleSelectAll = (checked: boolean) => {
    if (!checked) {
      setSelectedNames([]);
      return;
    }
    setSelectedNames(visibleItems.map((it) => it.entryName));
  };

  const toggleSelectItem = (entryName: string) => {
    setSelectedNames((prev) =>
      prev.includes(entryName) ? prev.filter((n) => n !== entryName) : [...prev, entryName],
    );
  };

  const handleConfirmDelete = async () => {
    if (!confirmDeleteTargets || confirmDeleteTargets.length === 0) return;
    const ok = await deleteTrash(confirmDeleteTargets);
    if (ok) {
      setConfirmDeleteTargets(null);
    }
  };

  const handleConfirmEmpty = async () => {
    const ok = await emptyTrash();
    if (ok) {
      setConfirmEmptyOpen(false);
    }
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <div className="toolbar" style={{ flexWrap: 'wrap', gap: 8 }}>
        <button type="button" className="btn" onClick={() => setTab('settings')}>
          <ArrowLeft size={14} aria-hidden="true" />
          <span>{t.trash.backToSettings}</span>
        </button>

        <div style={{ display: 'flex', flexDirection: 'column', minWidth: 0, marginRight: 8 }}>
          <h1 style={{ margin: 0, fontSize: 15, fontWeight: 600 }}>{t.trash.title}</h1>
          <span
            style={{
              fontSize: 11.5,
              color: 'var(--text-muted)',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
              whiteSpace: 'nowrap',
              maxWidth: 320,
            }}
            title={settings.trashDir || '~/.HandShakerTrash'}
          >
            {t.trash.dirLabel(settings.trashDir || '~/.HandShakerTrash')}
          </span>
        </div>

        <input
          type="search"
          aria-label={t.trash.filterPlaceholder}
          className="search-input"
          placeholder={t.trash.filterPlaceholder}
          value={filterText}
          onChange={(e) => setFilterText(e.target.value)}
        />

        <button
          type="button"
          className="btn"
          aria-label={t.common.refresh}
          title={t.common.refresh}
          disabled={trashLoading || trashBusy}
          onClick={() => void loadTrash()}
        >
          <RefreshCw size={14} aria-hidden="true" />
        </button>

        <button
          type="button"
          className="btn primary"
          disabled={!isConnected || selectedNames.length === 0 || trashBusy}
          title={!isConnected ? t.trash.connectToRestore : undefined}
          onClick={() => void restoreTrash(selectedNames)}
        >
          <RotateCcw size={14} aria-hidden="true" />
          <span>
            {selectedNames.length > 0
              ? `${t.trash.restoreSelected} (${selectedNames.length})`
              : t.trash.restoreSelected}
          </span>
        </button>

        <button
          type="button"
          className="btn danger"
          disabled={selectedNames.length === 0 || trashBusy}
          onClick={() => setConfirmDeleteTargets(selectedNames)}
        >
          <Trash2 size={14} aria-hidden="true" />
          <span>{t.trash.deletePermanently}</span>
        </button>

        <button
          type="button"
          className="btn danger"
          disabled={trashItems.length === 0 || trashBusy}
          onClick={() => setConfirmEmptyOpen(true)}
        >
          <span>{t.trash.emptyTrash}</span>
        </button>
      </div>

      {!isConnected && (
        <div
          style={{
            padding: '8px 14px',
            fontSize: 12.5,
            color: 'var(--text-secondary)',
            borderBottom: '1px solid var(--border-subtle)',
            background: 'var(--bg-subtle, transparent)',
          }}
        >
          {t.trash.connectToRestore}
        </div>
      )}

      <div className="table-scroller">
        <table className="hs-table">
          <caption className="sr-only">{t.trash.title}</caption>
          <colgroup>
            <col style={{ width: '40px' }} />
            <col style={{ width: '19%' }} />
            <col style={{ width: '27%' }} />
            <col style={{ width: '19%' }} />
            <col style={{ width: '14%' }} />
            <col style={{ width: '9%' }} />
            <col style={{ width: '110px' }} />
          </colgroup>
          <thead>
            <tr>
              <th scope="col" style={{ paddingLeft: 12 }}>
                <input
                  type="checkbox"
                  aria-label={t.trash.columns.selectAll}
                  checked={allVisibleSelected}
                  onChange={(e) => toggleSelectAll(e.target.checked)}
                />
              </th>
              <th scope="col">{t.trash.columns.name}</th>
              <th scope="col">{t.trash.columns.originalPath}</th>
              <th scope="col">{t.trash.columns.sourceDevice}</th>
              <th scope="col">{t.trash.columns.deletedAt}</th>
              <th scope="col">{t.trash.columns.size}</th>
              <th scope="col" />
            </tr>
          </thead>
          <tbody>
            {trashLoading && trashItems.length === 0 ? (
              <tr>
                <td colSpan={7} style={{ padding: 24, textAlign: 'center', color: 'var(--text-muted)' }}>
                  {t.common.syncing}
                </td>
              </tr>
            ) : visibleItems.length === 0 ? (
              <tr>
                <td colSpan={7} style={{ padding: 32, textAlign: 'center', color: 'var(--text-muted)' }}>
                  <div style={{ fontWeight: 500, marginBottom: 4 }}>
                    {filterText ? t.trash.noMatches : t.trash.emptyTitle}
                  </div>
                  {!filterText && <div style={{ fontSize: 12 }}>{t.trash.emptyHint}</div>}
                </td>
              </tr>
            ) : (
              visibleItems.map((item) => {
                const selected = selectedNames.includes(item.entryName);
                const sourcePhone = formatTrashSourcePhone(item);
                const itemUuid = item.deviceUuid?.trim() || '';
                const isSameDevice = Boolean(isConnected && currentDeviceUuid && itemUuid === currentDeviceUuid);
                const isOtherDevice = Boolean(isConnected && currentDeviceUuid && itemUuid && itemUuid !== currentDeviceUuid);
                return (
                  <tr key={item.entryName} className={selected ? 'selected' : undefined}>
                    <td style={{ paddingLeft: 12 }}>
                      <input
                        type="checkbox"
                        aria-label={item.entryName}
                        checked={selected}
                        onChange={() => toggleSelectItem(item.entryName)}
                      />
                    </td>
                    <td>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8, minWidth: 0 }}>
                        {item.isDirectory ? (
                          <Folder size={16} aria-hidden="true" style={{ flexShrink: 0 }} />
                        ) : (
                          <FileIcon size={16} aria-hidden="true" style={{ flexShrink: 0 }} />
                        )}
                        <span
                          style={{
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            whiteSpace: 'nowrap',
                          }}
                          title={item.entryName}
                        >
                          {item.entryName}
                        </span>
                      </div>
                    </td>
                    <td
                      style={{
                        color: item.originalPath ? 'var(--text-secondary)' : 'var(--text-muted)',
                        fontSize: 12.5,
                        wordBreak: 'break-all',
                      }}
                    >
                      {item.originalPath || t.trash.unknownPath}
                    </td>
                    <td style={{ fontSize: 12.5 }} title={itemUuid || undefined}>
                      {sourcePhone ? (
                        <div style={{ display: 'flex', flexDirection: 'column', gap: 1 }}>
                          <span style={{ color: 'var(--text-secondary)', wordBreak: 'break-word' }}>
                            {sourcePhone}
                          </span>
                          {isOtherDevice && (
                            <span style={{ fontSize: 11, color: 'var(--danger)' }}>
                              {t.trash.otherDeviceBadge}
                            </span>
                          )}
                          {isSameDevice && (
                            <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>
                              {t.trash.currentDeviceBadge}
                            </span>
                          )}
                        </div>
                      ) : (
                        <span style={{ color: 'var(--text-muted)' }}>{t.trash.unknownDevice}</span>
                      )}
                    </td>
                    <td style={{ fontSize: 12.5, color: 'var(--text-secondary)' }}>
                      {formatTrashTimestamp(item, lang)}
                    </td>
                    <td style={{ fontSize: 12.5, color: 'var(--text-secondary)' }}>
                      {formatBytes(item.sizeBytes)}
                    </td>
                    <td>
                      <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 6 }}>
                        <button
                          type="button"
                          className="btn"
                          disabled={!isConnected || !item.originalPath || trashBusy}
                          title={!isConnected ? t.trash.connectToRestore : t.trash.restore}
                          onClick={() => void restoreTrash([item.entryName])}
                        >
                          <RotateCcw size={13} aria-hidden="true" />
                          <span>{t.trash.restore}</span>
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

      <ModalDialog
        open={confirmDeleteTargets !== null}
        onClose={() => {
          if (!trashBusy) setConfirmDeleteTargets(null);
        }}
        title={t.trash.confirmDeleteTitle}
        subtitle={
          confirmDeleteTargets ? t.trash.confirmDeleteBody(confirmDeleteTargets.length) : undefined
        }
        titleId="trash-delete-modal-title"
      >
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 12 }}>
          <button
            type="button"
            className="btn"
            disabled={trashBusy}
            onClick={() => setConfirmDeleteTargets(null)}
          >
            {t.common.cancel}
          </button>
          <button
            type="button"
            className="btn danger"
            disabled={trashBusy}
            onClick={() => void handleConfirmDelete()}
          >
            {t.trash.deletePermanently}
          </button>
        </div>
      </ModalDialog>

      <ModalDialog
        open={confirmEmptyOpen}
        onClose={() => {
          if (!trashBusy) setConfirmEmptyOpen(false);
        }}
        title={t.trash.confirmEmptyTitle}
        subtitle={t.trash.confirmEmptyBody(trashItems.length)}
        titleId="trash-empty-modal-title"
      >
        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, marginTop: 12 }}>
          <button
            type="button"
            className="btn"
            disabled={trashBusy}
            onClick={() => setConfirmEmptyOpen(false)}
          >
            {t.common.cancel}
          </button>
          <button
            type="button"
            className="btn danger"
            disabled={trashBusy}
            onClick={() => void handleConfirmEmpty()}
          >
            {t.trash.emptyTrash}
          </button>
        </div>
      </ModalDialog>
    </div>
  );
}
