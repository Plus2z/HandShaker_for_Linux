import { useEffect, useState } from 'react';
import { useI18n } from '../lib/useI18n';
import { useAppStore } from '../store/useAppStore';
import { ModalDialog } from './ModalDialog';

export interface DeleteTargetItem {
  path: string;
  name?: string;
}

interface DeleteFilesDialogProps {
  open: boolean;
  items: DeleteTargetItem[];
  titleId: string;
  onClose: () => void;
  onConfirm: (paths: string[], isTrash: boolean) => void | Promise<unknown>;
}

/**
 * Shared secondary-confirmation dialog for deleting files or media items on the phone.
 * Resets permanent-delete opt-in on open and keeps Cancel focused by default.
 */
export function DeleteFilesDialog({
  open,
  items,
  titleId,
  onClose,
  onConfirm,
}: DeleteFilesDialogProps) {
  const { t } = useI18n();
  const trashDir = useAppStore((s) => s.settings.trashDir || '~/.HandShakerTrash');
  const [permanentDelete, setPermanentDelete] = useState(false);

  useEffect(() => {
    if (open) {
      setPermanentDelete(false);
    }
  }, [open]);

  const previewItems = items.slice(0, 3);
  const remaining = items.length - previewItems.length;

  return (
    <ModalDialog
      open={open}
      onClose={onClose}
      title={t.files.deleteConfirmTitle(items.length)}
      titleId={titleId}
    >
      <p style={{ margin: 0, color: 'var(--text-secondary)' }}>{t.files.deleteConfirmDesc}</p>
      {items.length > 0 && (
        <div
          style={{
            padding: '8px 10px',
            borderRadius: 7,
            background: 'var(--bg-elevated)',
            border: '1px solid var(--border)',
            fontSize: 12,
            color: 'var(--text-secondary)',
            display: 'flex',
            flexDirection: 'column',
            gap: 2,
          }}
        >
          {items.length === 1 ? (
            <>
              {items[0].name && (
                <div style={{ fontWeight: 600, color: 'var(--text-primary)', wordBreak: 'break-all' }}>
                  {items[0].name}
                </div>
              )}
              <div style={{ color: 'var(--text-muted)', wordBreak: 'break-all' }}>{items[0].path}</div>
            </>
          ) : (
            <>
              {previewItems.map((it) => (
                <div key={it.path} style={{ wordBreak: 'break-all' }}>
                  {it.name || it.path}
                </div>
              ))}
              {remaining > 0 && (
                <div style={{ color: 'var(--text-muted)' }}>{t.files.deleteMoreItems(remaining)}</div>
              )}
            </>
          )}
        </div>
      )}
      <label style={{ display: 'flex', alignItems: 'center', gap: 8, fontSize: 12.5 }}>
        <input
          type="checkbox"
          checked={permanentDelete}
          onChange={(e) => setPermanentDelete(e.target.checked)}
        />
        <span>{t.files.deletePermCheckbox}</span>
      </label>
      {!permanentDelete && (
        <div style={{ fontSize: 12, color: 'var(--text-muted)', wordBreak: 'break-all' }}>
          {t.files.deleteTrashTargetHint(trashDir)}
        </div>
      )}
      <div className="dialog-actions">
        <button type="button" className="btn" autoFocus onClick={onClose}>
          {t.common.cancel}
        </button>
        <button
          type="button"
          className="btn danger"
          onClick={() => {
            const paths = items.map((it) => it.path);
            onClose();
            void onConfirm(paths, !permanentDelete);
          }}
        >
          {permanentDelete ? t.files.deletePermAction : t.files.moveToTrash}
        </button>
      </div>
    </ModalDialog>
  );
}
