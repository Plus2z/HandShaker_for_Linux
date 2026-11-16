import { useEffect } from 'react';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { AlertCircle, CheckCircle2, X } from 'lucide-react';
import { ClipboardView } from './components/ClipboardView';
import { FilesView } from './components/FilesView';
import { MusicView } from './components/MusicView';
import { PhotosView } from './components/PhotosView';
import { SettingsView } from './components/SettingsView';
import { Sidebar } from './components/Sidebar';
import { TitleBar } from './components/TitleBar';
import { TransferDrawer } from './components/TransferDrawer';
import { TrashView } from './components/TrashView';
import { VideosView } from './components/VideosView';
import { WelcomeConnectView } from './components/WelcomeConnectView';
import { useI18n } from './lib/useI18n';
import { useAppStore } from './store/useAppStore';

export function App() {
  const { lang, t } = useI18n();
  const initialized = useAppStore((s) => s.initialized);
  const tab = useAppStore((s) => s.tab);
  const settings = useAppStore((s) => s.settings);
  const connection = useAppStore((s) => s.connection);
  const toastError = useAppStore((s) => s.toastError);
  const setToastError = useAppStore((s) => s.setToastError);
  const statusNotice = useAppStore((s) => s.statusNotice);
  const setStatusNotice = useAppStore((s) => s.setStatusNotice);
  const bootstrap = useAppStore((s) => s.bootstrap);

  useEffect(() => {
    let disposed = false;
    let stop: (() => void) | undefined;
    void bootstrap().then((fn) => {
      if (disposed) {
        fn();
      } else {
        stop = fn;
      }
    });
    return () => {
      disposed = true;
      if (stop) stop();
    };
  }, [bootstrap]);

  useEffect(() => {
    if (!statusNotice) return;
    const timer = window.setTimeout(() => setStatusNotice(null), 2600);
    return () => window.clearTimeout(timer);
  }, [statusNotice, setStatusNotice]);

  // Refresh phone clipboard on window return (helpful on Wayland compositors without wlr-data-control)
  useEffect(() => {
    const syncOnFocus = () => {
      const state = useAppStore.getState();
      if (state.connection.status === 'connected' && state.tab === 'clipboard') {
        void state.loadClipboard();
      }
    };
    window.addEventListener('focus', syncOnFocus);
    let disposed = false;
    let unlistenFocus: (() => void) | undefined;
    try {
      void getCurrentWindow()
        .onFocusChanged(({ payload: focused }) => {
          if (focused && !disposed) syncOnFocus();
        })
        .then((u) => {
          if (disposed) {
            void Promise.resolve(u()).catch(() => {});
          } else {
            unlistenFocus = u;
          }
        })
        .catch(() => {});
    } catch {
      // Outside Tauri runtime (Vitest)
    }
    return () => {
      disposed = true;
      window.removeEventListener('focus', syncOnFocus);
      if (unlistenFocus) {
        void Promise.resolve(unlistenFocus()).catch(() => {});
      }
    };
  }, []);

  useEffect(() => {
    document.documentElement.lang = lang;
  }, [lang]);

  useEffect(() => {
    const root = document.documentElement;
    const apply = () => {
      if (settings.theme === 'light' || settings.theme === 'dark') {
        root.setAttribute('data-theme', settings.theme);
      } else {
        root.removeAttribute('data-theme');
      }
    };
    apply();

    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const onChange = () => {
      if (settings.theme === 'system') apply();
    };
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  }, [settings.theme]);

  const isConnected = connection.status === 'connected';

  const renderMain = () => {
    if (!initialized) {
      return (
        <div className="view-scroll" style={{ color: 'var(--text-muted)' }}>
          {t.app.initializing}
        </div>
      );
    }
    if (tab === 'settings') return <SettingsView />;
    if (tab === 'trash') return <TrashView />;
    if (!isConnected) return <WelcomeConnectView />;
    switch (tab) {
      case 'files':
        return <FilesView />;
      case 'photos':
        return <PhotosView />;
      case 'videos':
        return <VideosView />;
      case 'music':
        return <MusicView />;
      case 'clipboard':
        return <ClipboardView />;
    }
  };

  return (
    <div className="app-shell">
      <TitleBar />
      <div className="workspace">
        <Sidebar />
        <main className="main-pane">{renderMain()}</main>
        <TransferDrawer />
      </div>

      {statusNotice && (
        <div
          className="status-banner"
          role="status"
          aria-live="polite"
          style={toastError ? { bottom: 72 } : undefined}
        >
          <CheckCircle2 size={16} color="var(--success)" style={{ flexShrink: 0, marginTop: 2 }} aria-hidden="true" />
          <div style={{ flex: 1, fontSize: 12.5 }}>{statusNotice}</div>
          <button
            type="button"
            className="icon-btn"
            aria-label={t.app.dismissStatusNotice}
            onClick={() => setStatusNotice(null)}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      )}

      {toastError && (
        <div className="toast-banner" role="alert">
          <AlertCircle size={16} color="var(--danger)" style={{ flexShrink: 0, marginTop: 2 }} aria-hidden="true" />
          <div style={{ flex: 1, fontSize: 12.5 }}>{toastError}</div>
          <button
            type="button"
            className="icon-btn"
            aria-label={t.app.dismissError}
            onClick={() => setToastError(null)}
          >
            <X size={14} aria-hidden="true" />
          </button>
        </div>
      )}
    </div>
  );
}

export default App;
