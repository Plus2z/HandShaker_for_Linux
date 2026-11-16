import { CheckSquare, Copy, Download, Trash2, X } from 'lucide-react';
import { useI18n } from '../lib/useI18n';

interface MediaSelectionBarProps {
  selectedCount: number;
  totalCount: number;
  allSelected: boolean;
  onSelectAll: (checked: boolean) => void;
  onDownload: () => void;
  onCopyTo: () => void;
  onDelete: () => void;
  onDone: () => void;
}

/**
 * Shared batch-action bar displayed in Photos, Videos, and Music views during multi-select mode.
 */
export function MediaSelectionBar({
  selectedCount,
  totalCount,
  allSelected,
  onSelectAll,
  onDownload,
  onCopyTo,
  onDelete,
  onDone,
}: MediaSelectionBarProps) {
  const { t } = useI18n();

  return (
    <div className="media-selection-bar" role="region" aria-label={t.mediaSelect.selectionBarAria}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span style={{ fontSize: 12.5, fontWeight: 600 }}>
          {t.mediaSelect.selectedCount(selectedCount, totalCount)}
        </span>
        <button
          type="button"
          className="btn"
          disabled={totalCount === 0}
          onClick={() => onSelectAll(!allSelected)}
        >
          <CheckSquare size={14} aria-hidden="true" />
          <span>{allSelected ? t.mediaSelect.deselectAll : t.mediaSelect.selectAll}</span>
        </button>
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
        <button
          type="button"
          className="btn"
          disabled={selectedCount === 0}
          onClick={onDownload}
        >
          <Download size={14} aria-hidden="true" />
          <span>{t.mediaSelect.downloadCount(selectedCount)}</span>
        </button>
        <button
          type="button"
          className="btn primary"
          disabled={selectedCount === 0}
          onClick={onCopyTo}
        >
          <Copy size={14} aria-hidden="true" />
          <span>{t.mediaSelect.copyToFolder}</span>
        </button>
        <button
          type="button"
          className="btn danger"
          disabled={selectedCount === 0}
          onClick={onDelete}
        >
          <Trash2 size={14} aria-hidden="true" />
          <span>{t.mediaSelect.deleteCount(selectedCount)}</span>
        </button>
        <button type="button" className="btn" onClick={onDone}>
          <X size={14} aria-hidden="true" />
          <span>{t.mediaSelect.exitMultiSelect}</span>
        </button>
      </div>
    </div>
  );
}
