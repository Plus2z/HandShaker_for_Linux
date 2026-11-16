import { formatBytes } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import type { DeviceInfo } from '../types/api';

interface StorageBarProps {
  info: DeviceInfo;
}

export function StorageBar({ info }: StorageBarProps) {
  const { t } = useI18n();
  const total = Math.max(info.diskSize, 1);
  const segments = [
    { key: 'media', label: t.storage.segments.media, value: info.picVideoSize, color: '#ec4899' },
    { key: 'audio', label: t.storage.segments.audio, value: info.audioSize, color: '#8b5cf6' },
    { key: 'downloads', label: t.storage.segments.downloads, value: info.downloadSize, color: '#3b82f6' },
    { key: 'apps', label: t.storage.segments.apps, value: info.appSize, color: '#f59e0b' },
    { key: 'cache', label: t.storage.segments.cache, value: info.cacheSize, color: '#10b981' },
    { key: 'other', label: t.storage.segments.other, value: info.otherSize, color: '#64748b' },
  ].filter((s) => s.value > 0);

  return (
    <div className="storage-card" aria-label={t.storage.ariaLabel}>
      <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 12, fontWeight: 600 }}>
        <span>{t.storage.heading}</span>
        <span>{t.storage.battery(info.batteryPercentage)}</span>
      </div>
      <div
        className="storage-bar-track"
        role="img"
        aria-label={t.storage.usedOf(formatBytes(info.usedDiskSize), formatBytes(info.diskSize))}
      >
        {segments.map((seg) => (
          <div
            key={seg.key}
            className="storage-seg"
            title={`${seg.label}: ${formatBytes(seg.value)}`}
            style={{
              width: `${Math.max((seg.value / total) * 100, 1)}%`,
              backgroundColor: seg.color,
            }}
          />
        ))}
      </div>
      <div style={{ fontSize: 11.5, color: 'var(--text-secondary)' }}>
        {t.storage.freeOf(formatBytes(info.freeDiskSize), formatBytes(info.diskSize))}
      </div>
      {segments.length > 0 && (
        <div className="storage-legend">
          {segments.map((seg) => (
            <div key={seg.key} className="storage-legend-item" title={`${seg.label}: ${formatBytes(seg.value)}`}>
              <span className="dot" style={{ backgroundColor: seg.color }} />
              <span>
                {seg.label} {formatBytes(seg.value)}
              </span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
