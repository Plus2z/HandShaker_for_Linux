import { ArrowDownToLine, ArrowUpFromLine, Trash2, X } from 'lucide-react';
import { formatBytes } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import { useAppStore } from '../store/useAppStore';

export function TransferDrawer() {
  const open = useAppStore((s) => s.transferDrawerOpen);
  const setOpen = useAppStore((s) => s.setTransferDrawerOpen);
  const transfers = useAppStore((s) => s.transfers);
  const cancelTransfer = useAppStore((s) => s.cancelTransfer);
  const clearFinishedTransfers = useAppStore((s) => s.clearFinishedTransfers);
  const { t } = useI18n();

  if (!open) return null;

  return (
    <aside className="transfer-drawer" aria-label={t.transfers.queueAria}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          padding: '12px 14px',
          borderBottom: '1px solid var(--border)',
        }}
      >
        <strong>{t.transfers.heading(transfers.length)}</strong>
        <div style={{ display: 'flex', gap: 6 }}>
          <button
            type="button"
            className="btn"
            onClick={() => void clearFinishedTransfers()}
            title={t.transfers.clearTitle}
          >
            <Trash2 size={13} aria-hidden="true" />
            <span>{t.transfers.clear}</span>
          </button>
          <button
            type="button"
            className="icon-btn"
            aria-label={t.transfers.closeAria}
            onClick={() => setOpen(false)}
          >
            <X size={15} aria-hidden="true" />
          </button>
        </div>
      </div>

      <div style={{ flex: 1, overflowY: 'auto', padding: 12, display: 'flex', flexDirection: 'column', gap: 10 }}>
        {transfers.length === 0 ? (
          <p style={{ color: 'var(--text-muted)', margin: 0, fontSize: 12.5 }}>
            {t.transfers.empty}
          </p>
        ) : (
          [...transfers].reverse().map((tx) => {
            const pct =
              tx.totalBytes > 0
                ? Math.min(100, Math.round((tx.transferredBytes / tx.totalBytes) * 100))
                : tx.status === 'completed'
                  ? 100
                  : 0;
            const canCancel = tx.status === 'queued' || tx.status === 'active';
            return (
              <div key={tx.id} className="card" style={{ padding: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, minWidth: 0 }}>
                    {tx.direction === 'download' ? (
                      <ArrowDownToLine size={14} aria-hidden="true" />
                    ) : (
                      <ArrowUpFromLine size={14} aria-hidden="true" />
                    )}
                    <span
                      style={{
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                        fontWeight: 500,
                      }}
                      title={tx.fileName}
                    >
                      {tx.fileName}
                    </span>
                  </div>
                  {canCancel && (
                    <button
                      type="button"
                      className="icon-btn"
                      aria-label={t.transfers.cancelAria(tx.fileName)}
                      onClick={() => void cancelTransfer(tx.id)}
                    >
                      <X size={14} aria-hidden="true" />
                    </button>
                  )}
                </div>
                <div className="progress-track">
                  <div className="progress-fill" style={{ width: `${pct}%` }} />
                </div>
                <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 11.5, color: 'var(--text-muted)' }}>
                  <span>
                    {formatBytes(tx.transferredBytes)} / {formatBytes(tx.totalBytes)} ({pct}%)
                  </span>
                  <span>{t.transfers.status[tx.status] ?? tx.status}</span>
                </div>
                {tx.error && (
                  <div style={{ color: 'var(--danger)', fontSize: 11.5 }}>{tx.error}</div>
                )}
              </div>
            );
          })
        )}
      </div>
    </aside>
  );
}
