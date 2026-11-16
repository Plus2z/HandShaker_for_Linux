import { Minus, Smartphone, Square, Wifi, Usb, X } from 'lucide-react';
import { ipc } from '../lib/ipc';
import { useI18n } from '../lib/useI18n';
import { useAppStore } from '../store/useAppStore';

export function TitleBar() {
  const settings = useAppStore((s) => s.settings);
  const connection = useAppStore((s) => s.connection);
  const { t } = useI18n();

  if (settings.useSystemDecorations) {
    return null;
  }

  const hs = connection.handshake;

  return (
    <header className="titlebar" data-tauri-drag-region>
      <div className="titlebar-left" data-tauri-drag-region>
        <Smartphone size={15} aria-hidden="true" />
        <span>{t.titleBar.appTitle}</span>
        {connection.status === 'connected' && hs ? (
          <span className="pill ok">
            {hs.transport === 'usb_aoa' ? (
              <Usb size={12} aria-hidden="true" />
            ) : (
              <Wifi size={12} aria-hidden="true" />
            )}
            <span>{hs.deviceName}</span>
          </span>
        ) : connection.status === 'trust_waiting' ? (
          <span className="pill warn">{t.titleBar.waitingTrust}</span>
        ) : (
          <span className="pill">{t.titleBar.notConnected}</span>
        )}
      </div>

      <div className="titlebar-controls">
        <button
          type="button"
          className="icon-btn"
          aria-label={t.titleBar.minimizeWindow}
          onClick={() => void ipc.windowMinimize()}
        >
          <Minus size={14} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={t.titleBar.toggleMaximizeWindow}
          onClick={() => void ipc.windowToggleMaximize()}
        >
          <Square size={12} aria-hidden="true" />
        </button>
        <button
          type="button"
          className="icon-btn"
          aria-label={t.titleBar.closeWindow}
          onClick={() => void ipc.windowClose()}
        >
          <X size={14} aria-hidden="true" />
        </button>
      </div>
    </header>
  );
}
