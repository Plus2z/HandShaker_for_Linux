import { useState, type FormEvent } from 'react';
import { Copy, RefreshCw, Send, Trash2 } from 'lucide-react';
import { formatTimestamp } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import { useAppStore } from '../store/useAppStore';
import type { ClipboardItem } from '../types/api';
import { ModalDialog } from './ModalDialog';

export function ClipboardView() {
  const clipboard = useAppStore((s) => s.clipboard);
  const clipboardLoading = useAppStore((s) => s.clipboardLoading);
  const settings = useAppStore((s) => s.settings);
  const loadClipboard = useAppStore((s) => s.loadClipboard);
  const pushClipboard = useAppStore((s) => s.pushClipboard);
  const clearClipboard = useAppStore((s) => s.clearClipboard);
  const deleteClipboard = useAppStore((s) => s.deleteClipboard);
  const copyClipboardToLinux = useAppStore((s) => s.copyClipboardToLinux);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const { lang, t } = useI18n();

  const [draft, setDraft] = useState('');
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ClipboardItem | null>(null);

  const handleSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!draft.trim()) return;
    void pushClipboard(draft);
    setDraft('');
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', height: '100%', minHeight: 0 }}>
      <div className="toolbar" style={{ justifyContent: 'space-between' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <strong>{t.clipboard.heading(clipboard.length)}</strong>
          <label style={{ display: 'inline-flex', alignItems: 'center', gap: 6, fontSize: 12.5, cursor: 'pointer' }}>
            <input
              type="checkbox"
              checked={settings.clipboardAutoSync}
              onChange={(e) => void updateSettings({ clipboardAutoSync: e.target.checked })}
            />
            <span>{t.clipboard.autoSync}</span>
          </label>
        </div>

        <div style={{ display: 'flex', gap: 8 }}>
          <button type="button" className="btn" onClick={() => void loadClipboard()}>
            <RefreshCw size={14} aria-hidden="true" />
            <span>{clipboardLoading ? t.clipboard.refreshing : t.common.refresh}</span>
          </button>
          <button
            type="button"
            className="btn danger"
            disabled={clipboard.length === 0}
            onClick={() => setClearConfirmOpen(true)}
          >
            <Trash2 size={14} aria-hidden="true" />
            <span>{t.clipboard.clearAll}</span>
          </button>
        </div>
      </div>

      <div className="view-scroll" style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
        <form onSubmit={handleSubmit} className="card" style={{ display: 'flex', gap: 10, alignItems: 'flex-end' }}>
          <textarea
            aria-label={t.clipboard.sendAria}
            className="textarea-input"
            rows={2}
            style={{ flex: 1, resize: 'vertical' }}
            placeholder={t.clipboard.sendPlaceholder}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
          />
          <button type="submit" className="btn primary" disabled={!draft.trim()}>
            <Send size={14} aria-hidden="true" />
            <span>{t.clipboard.pushToPhone}</span>
          </button>
        </form>

        {clipboard.length === 0 ? (
          <p style={{ color: 'var(--text-muted)', margin: 0 }}>
            {t.clipboard.empty}
          </p>
        ) : (
          <div className="clip-grid">
            {clipboard.map((item) => (
              <article key={`${item.timestampMs}-${item.rawGzipB64.slice(0, 12)}`} className="clip-note">
                <pre className="clip-text">{item.text}</pre>
                <div style={{ marginTop: 'auto', display: 'flex', alignItems: 'center', justifyContent: 'space-between', paddingTop: 6, borderTop: '1px solid var(--border)' }}>
                  <span style={{ fontSize: 11.5, color: 'var(--text-muted)' }}>
                    {formatTimestamp(item.timestampMs, lang)}
                  </span>
                  <div style={{ display: 'flex', gap: 4 }}>
                    <button
                      type="button"
                      className="btn"
                      onClick={() => void copyClipboardToLinux(item.text)}
                    >
                      <Copy size={12} aria-hidden="true" />
                      <span>{t.clipboard.copy}</span>
                    </button>
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={t.clipboard.deleteAria}
                      onClick={() => setDeleteTarget(item)}
                    >
                      <Trash2 size={14} aria-hidden="true" />
                    </button>
                  </div>
                </div>
              </article>
            ))}
          </div>
        )}
      </div>

      <ModalDialog
        open={clearConfirmOpen}
        onClose={() => setClearConfirmOpen(false)}
        title={t.clipboard.clearConfirmTitle}
        titleId="clear-clip-title"
      >
        <p style={{ margin: 0, color: 'var(--text-secondary)' }}>
          {t.clipboard.clearConfirmDesc}
        </p>
        <div className="dialog-actions">
          <button type="button" className="btn" autoFocus onClick={() => setClearConfirmOpen(false)}>
            {t.common.cancel}
          </button>
          <button
            type="button"
            className="btn danger"
            onClick={() => {
              setClearConfirmOpen(false);
              void clearClipboard();
            }}
          >
            {t.clipboard.clearHistory}
          </button>
        </div>
      </ModalDialog>

      <ModalDialog
        open={deleteTarget !== null}
        onClose={() => setDeleteTarget(null)}
        title={t.clipboard.deleteConfirmTitle}
        titleId="delete-clip-title"
      >
        {deleteTarget && (
          <pre
            style={{
              margin: 0,
              padding: '8px 10px',
              borderRadius: 7,
              background: 'var(--bg-elevated)',
              border: '1px solid var(--border)',
              fontSize: 12,
              maxHeight: 120,
              overflowY: 'auto',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
              color: 'var(--text-secondary)',
            }}
          >
            {deleteTarget.text}
          </pre>
        )}
        <div className="dialog-actions">
          <button type="button" className="btn" autoFocus onClick={() => setDeleteTarget(null)}>
            {t.common.cancel}
          </button>
          <button
            type="button"
            className="btn danger"
            onClick={() => {
              const item = deleteTarget;
              setDeleteTarget(null);
              if (item) {
                void deleteClipboard(item);
              }
            }}
          >
            {t.clipboard.deleteConfirmAction}
          </button>
        </div>
      </ModalDialog>
    </div>
  );
}
