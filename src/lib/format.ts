export const nameCollator = new Intl.Collator(undefined, {
  numeric: true,
  sensitivity: 'base',
});

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let idx = 0;
  let val = bytes;
  while (val >= 1024 && idx < units.length - 1) {
    val /= 1024;
    idx += 1;
  }
  const fixed = val >= 100 || idx === 0 ? 0 : val >= 10 ? 1 : 2;
  return `${val.toFixed(fixed)} ${units[idx]}`;
}

export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds <= 0) return '0:00';
  const total = Math.floor(seconds);
  const hrs = Math.floor(total / 3600);
  const mins = Math.floor((total % 3600) / 60);
  const secs = total % 60;
  if (hrs > 0) {
    return `${hrs}:${String(mins).padStart(2, '0')}:${String(secs).padStart(2, '0')}`;
  }
  return `${mins}:${String(secs).padStart(2, '0')}`;
}

export function formatTimestamp(tsSecondsOrMs: number, locale?: string): string {
  if (!tsSecondsOrMs || tsSecondsOrMs <= 0) return '—';
  const ms = tsSecondsOrMs < 1e12 ? tsSecondsOrMs * 1000 : tsSecondsOrMs;
  const d = new Date(ms);
  return d.toLocaleString(locale, {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  });
}

/** Splits filename into [stem, extension] so `.filename` keeps extension visible when truncated. */
export function splitFilenameExt(name: string, isDirectory: boolean): [string, string] {
  if (isDirectory) return [name, ''];
  const dotIdx = name.lastIndexOf('.');
  if (dotIdx <= 0 || dotIdx >= name.length - 1) {
    return [name, ''];
  }
  const ext = name.slice(dotIdx);
  if (ext.length > 10) return [name, ''];
  return [name.slice(0, dotIdx), ext];
}

export function parentRemotePath(path: string, rootPath: string): string {
  const clean = path.replace(/\/+$/, '');
  const cleanRoot = rootPath.replace(/\/+$/, '');
  if (!clean || clean === cleanRoot) return cleanRoot || '/';
  const idx = clean.lastIndexOf('/');
  if (idx <= 0) return cleanRoot || '/';
  const next = clean.slice(0, idx);
  return next.length < cleanRoot.length ? cleanRoot : next;
}

export function joinRemotePath(dir: string, name: string): string {
  const cleanDir = dir.replace(/\/+$/, '');
  const cleanName = name.replace(/^\/+/, '');
  return `${cleanDir}/${cleanName}`;
}

export function formatTrashSourcePhone(item: {
  deviceName?: string | null;
  deviceModel?: string | null;
  deviceUuid?: string | null;
}): string {
  const name = item.deviceName?.trim() || '';
  const model = item.deviceModel?.trim() || '';
  if (name && model && name.toLowerCase() !== model.toLowerCase()) {
    return `${name} (${model})`;
  }
  return name || model || item.deviceUuid?.trim() || '';
}
