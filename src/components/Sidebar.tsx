import {
  ArrowLeftRight,
  Clipboard,
  Film,
  Folder,
  Image as ImageIcon,
  LogOut,
  Music,
  Settings,
  Smartphone,
} from 'lucide-react';
import { useI18n } from '../lib/useI18n';
import { useAppStore, type NavTab } from '../store/useAppStore';
import { StorageBar } from './StorageBar';

const NAV_ITEMS: { id: Exclude<NavTab, 'trash'>; requiresConn: boolean; icon: typeof Folder }[] = [
  { id: 'files', requiresConn: true, icon: Folder },
  { id: 'photos', requiresConn: true, icon: ImageIcon },
  { id: 'videos', requiresConn: true, icon: Film },
  { id: 'music', requiresConn: true, icon: Music },
  { id: 'clipboard', requiresConn: true, icon: Clipboard },
  { id: 'settings', requiresConn: false, icon: Settings },
];

export function Sidebar() {
  const tab = useAppStore((s) => s.tab);
  const setTab = useAppStore((s) => s.setTab);
  const connection = useAppStore((s) => s.connection);
  const deviceInfo = useAppStore((s) => s.deviceInfo);
  const transfers = useAppStore((s) => s.transfers);
  const transferDrawerOpen = useAppStore((s) => s.transferDrawerOpen);
  const setTransferDrawerOpen = useAppStore((s) => s.setTransferDrawerOpen);
  const disconnect = useAppStore((s) => s.disconnect);
  const { t } = useI18n();

  const isConnected = connection.status === 'connected';
  const activeCount = transfers.filter((tx) => tx.status === 'active' || tx.status === 'queued').length;

  return (
    <aside className="sidebar" aria-label={t.sidebar.primaryNavigation}>
      <nav className="nav-list">
        {!isConnected && (
          <button
            type="button"
            className={`nav-item ${tab !== 'settings' && tab !== 'trash' ? 'active' : ''}`}
            aria-current={tab !== 'settings' && tab !== 'trash' ? 'page' : undefined}
            onClick={() => setTab('files')}
          >
            <Smartphone size={16} aria-hidden="true" />
            <span>{t.sidebar.connectDevice}</span>
          </button>
        )}
        {NAV_ITEMS.map((item) => {
          const Icon = item.icon;
          const disabled = item.requiresConn && !isConnected;
          const active = !disabled && (tab === item.id || (item.id === 'settings' && tab === 'trash'));
          return (
            <button
              key={item.id}
              type="button"
              className={`nav-item ${active ? 'active' : ''}`}
              disabled={disabled}
              aria-current={active ? 'page' : undefined}
              onClick={() => setTab(item.id)}
            >
              <Icon size={16} aria-hidden="true" />
              <span>{t.sidebar.tabs[item.id]}</span>
            </button>
          );
        })}
      </nav>

      <button
        type="button"
        className="btn"
        style={{ justifyContent: 'space-between' }}
        onClick={() => setTransferDrawerOpen(!transferDrawerOpen)}
      >
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 7 }}>
          <ArrowLeftRight size={15} aria-hidden="true" />
          <span>{t.sidebar.transfers}</span>
        </span>
        {activeCount > 0 && <span className="pill ok">{activeCount}</span>}
      </button>

      {isConnected && deviceInfo && <StorageBar info={deviceInfo} />}

      {isConnected && (
        <button type="button" className="btn danger" onClick={() => void disconnect()}>
          <LogOut size={15} aria-hidden="true" />
          <span>{t.sidebar.disconnect}</span>
        </button>
      )}
    </aside>
  );
}
