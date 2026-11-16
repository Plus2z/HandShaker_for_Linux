import { useEffect, useState } from 'react';
import { FolderOpen, Trash2 } from 'lucide-react';
import { ipc } from '../lib/ipc';
import { useI18n } from '../lib/useI18n';
import { useAppStore } from '../store/useAppStore';
import type { LanguagePreference, ThemePreference } from '../types/api';

const LANGUAGE_OPTIONS: LanguagePreference[] = ['system', 'zh-CN', 'en'];
const THEME_OPTIONS: ThemePreference[] = ['system', 'light', 'dark'];
const DEFAULT_TRASH_DIR = '~/.HandShakerTrash';

function normalizeTrashPath(raw: string): string {
  return raw.trim().replace(/\/+$/, '');
}

function isValidLinuxTrashDir(raw: string): boolean {
  const trimmed = normalizeTrashPath(raw);
  if (!trimmed || trimmed.includes('\\') || trimmed.includes('\0')) return false;
  if (!(trimmed.startsWith('/') || trimmed.startsWith('~/'))) return false;
  if (
    trimmed === '~' ||
    trimmed === '/' ||
    trimmed === '/home' ||
    trimmed === '/root' ||
    trimmed === '/mnt' ||
    trimmed === '/media' ||
    trimmed === '/storage' ||
    trimmed === '/storage/emulated' ||
    trimmed === '/storage/emulated/0' ||
    trimmed === '/sdcard'
  ) {
    return false;
  }
  const rest = trimmed.startsWith('~/') ? trimmed.slice(2) : trimmed.slice(1);
  const segs = rest.split('/').filter(Boolean);
  if (segs.length === 0) return false;
  if (segs.some((seg) => seg === '.' || seg === '..')) return false;
  return true;
}

interface FolderPickerRowProps {
  label: string;
  path: string;
  chooseLabel: string;
  onChoose: () => void;
}

function FolderPickerRow({ label, path, chooseLabel, onChoose }: FolderPickerRowProps) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
      <div style={{ minWidth: 0, flex: 1 }}>
        <div style={{ fontWeight: 500 }}>{label}</div>
        <div style={{ fontSize: 12, color: 'var(--text-muted)', wordBreak: 'break-all' }}>
          {path}
        </div>
      </div>
      <button type="button" className="btn" onClick={onChoose}>
        <FolderOpen size={14} aria-hidden="true" />
        <span>{chooseLabel}</span>
      </button>
    </div>
  );
}

export function SettingsView() {
  const settings = useAppStore((s) => s.settings);
  const updateSettings = useAppStore((s) => s.updateSettings);
  const setTab = useAppStore((s) => s.setTab);
  const { pref, t } = useI18n();

  const [trashDirError, setTrashDirError] = useState<string | null>(null);

  useEffect(() => {
    setTrashDirError(null);
  }, [settings.trashDir]);

  const pickDownloadFolder = async () => {
    const dirs = await ipc.pickLocalPaths(true, false);
    if (dirs[0]) {
      await updateSettings({ downloadDir: dirs[0] });
    }
  };

  const pickTrashFolder = async () => {
    const dirs = await ipc.pickLocalPaths(true, false);
    if (!dirs[0]) return;
    const cleaned = normalizeTrashPath(dirs[0]);
    if (!isValidLinuxTrashDir(cleaned)) {
      setTrashDirError(t.settings.trashDirInvalid);
      return;
    }
    setTrashDirError(null);
    await updateSettings({ trashDir: cleaned });
  };

  return (
    <div className="view-scroll">
      <div style={{ maxWidth: 720, margin: '0 auto', display: 'flex', flexDirection: 'column', gap: 16 }}>
        <h1 style={{ margin: 0, fontSize: 18 }}>{t.settings.title}</h1>

        <section className="card" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <h2 style={{ margin: 0, fontSize: 14.5 }}>{t.settings.windowAppearance}</h2>

          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
            <div>
              <div style={{ fontWeight: 500 }}>{t.settings.languageLabel}</div>
              <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                {t.settings.languageDesc}
              </div>
            </div>
            <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
              {LANGUAGE_OPTIONS.map((opt) => (
                <button
                  key={opt}
                  type="button"
                  className={`chip-btn ${pref === opt ? 'active' : ''}`}
                  aria-pressed={pref === opt}
                  onClick={() => void updateSettings({ language: opt })}
                >
                  {t.settings.languageOptions[opt]}
                </button>
              ))}
            </div>
          </div>

          <label style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
            <div>
              <div style={{ fontWeight: 500 }}>{t.settings.useSystemDecorations}</div>
              <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                {settings.isTilingWm ? t.settings.tilingWmDesc : t.settings.floatingWmDesc}
              </div>
            </div>
            <input
              type="checkbox"
              checked={settings.useSystemDecorations}
              onChange={(e) => void updateSettings({ useSystemDecorations: e.target.checked })}
            />
          </label>

          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
            <div>
              <div style={{ fontWeight: 500 }}>{t.settings.colorTheme}</div>
              <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                {t.settings.colorThemeDesc}
              </div>
            </div>
            <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
              {THEME_OPTIONS.map((themeOpt) => (
                <button
                  key={themeOpt}
                  type="button"
                  className={`chip-btn ${settings.theme === themeOpt ? 'active' : ''}`}
                  onClick={() => void updateSettings({ theme: themeOpt })}
                >
                  {t.settings.themes[themeOpt]}
                </button>
              ))}
            </div>
          </div>
        </section>

        <section className="card" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <h2 style={{ margin: 0, fontSize: 14.5 }}>{t.settings.transfersClipboard}</h2>

          <FolderPickerRow
            label={t.settings.downloadDir}
            path={settings.downloadDir || '~/Downloads'}
            chooseLabel={t.settings.chooseFolder}
            onChoose={() => void pickDownloadFolder()}
          />

          <label style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
            <div>
              <div style={{ fontWeight: 500 }}>{t.settings.clipboardAutoSync}</div>
              <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>
                {t.settings.clipboardAutoSyncDesc}
              </div>
            </div>
            <input
              type="checkbox"
              checked={settings.clipboardAutoSync}
              onChange={(e) => void updateSettings({ clipboardAutoSync: e.target.checked })}
            />
          </label>
        </section>

        <section className="card" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 12 }}>
            <h2 style={{ margin: 0, fontSize: 14.5 }}>{t.settings.trashSection}</h2>
            <button type="button" className="btn" onClick={() => setTab('trash')}>
              <Trash2 size={14} aria-hidden="true" />
              <span>{t.settings.openTrash}</span>
            </button>
          </div>

          <FolderPickerRow
            label={t.settings.trashDir}
            path={settings.trashDir || DEFAULT_TRASH_DIR}
            chooseLabel={t.settings.chooseFolder}
            onChoose={() => void pickTrashFolder()}
          />

          {trashDirError && (
            <div style={{ fontSize: 12, color: 'var(--danger)' }}>{trashDirError}</div>
          )}
        </section>

        <section className="card" style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
          <h2 style={{ margin: 0, fontSize: 14.5 }}>{t.settings.usbPermissions}</h2>
          <p style={{ margin: 0, fontSize: 12.5, color: 'var(--text-secondary)' }}>
            {t.settings.usbPermissionsDesc}
          </p>
          <pre
            style={{
              margin: 0,
              padding: 10,
              borderRadius: 8,
              background: 'var(--bg-elevated)',
              fontSize: 12,
              overflowX: 'auto',
            }}
          >
            sudo install -m 0644 packaging/udev/70-handshaker.rules /etc/udev/rules.d/70-handshaker.rules{'\n'}
            sudo udevadm control --reload-rules &amp;&amp; sudo udevadm trigger
          </pre>
        </section>
      </div>
    </div>
  );
}
