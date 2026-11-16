import { type CSSProperties, type ReactNode, useEffect, useRef, useState } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { joinRemotePath } from '../lib/format';
import { useI18n } from '../lib/useI18n';
import { useAppStore } from '../store/useAppStore';

const DEFAULT_ANDROID_ROOT = '/storage/emulated/0';

/**
 * Resolves the target media folder on the connected phone (`DCIM`, `Movies`, or `Music`).
 */
export function useMediaUploadDir(folder: 'DCIM' | 'Movies' | 'Music'): string {
  const rootPath = useAppStore((s) => s.deviceInfo?.rootPath);
  return joinRemotePath(rootPath || DEFAULT_ANDROID_ROOT, folder);
}

export interface FileDropZoneProps {
  targetDir: string;
  disabled?: boolean;
  overlayInset?: number | string;
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
}

/**
 * Wraps a view with Tauri native file/folder drag-and-drop upload handling into `targetDir`.
 * Handles StrictMode async unlisten races and deduplicates identical native drop bursts.
 */
export function FileDropZone({
  targetDir,
  disabled = false,
  overlayInset = 56,
  className,
  style,
  children,
}: FileDropZoneProps) {
  const enqueueUploads = useAppStore((s) => s.enqueueUploads);
  const { t } = useI18n();
  const [isDragOver, setIsDragOver] = useState(false);

  const targetDirRef = useRef(targetDir);
  targetDirRef.current = targetDir;
  const disabledRef = useRef(disabled);
  disabledRef.current = disabled;

  useEffect(() => {
    if (disabled) {
      setIsDragOver(false);
    }
  }, [disabled]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    let lastDropKey = '';
    let lastDropAt = 0;

    try {
      void getCurrentWebview()
        .onDragDropEvent((ev) => {
          if (disposed) return;
          if (disabledRef.current) {
            setIsDragOver(false);
            return;
          }
          if (ev.payload.type === 'enter' || ev.payload.type === 'over') {
            setIsDragOver(true);
          } else if (ev.payload.type === 'leave') {
            setIsDragOver(false);
          } else if (ev.payload.type === 'drop') {
            setIsDragOver(false);
            const uniquePaths = Array.from(new Set(ev.payload.paths));
            if (uniquePaths.length > 0) {
              const now = Date.now();
              const dropKey = [...uniquePaths].sort().join('\0');
              if (dropKey === lastDropKey && now - lastDropAt < 350) {
                return;
              }
              lastDropKey = dropKey;
              lastDropAt = now;
              void enqueueUploads(uniquePaths, targetDirRef.current);
            }
          }
        })
        .then((u) => {
          if (disposed) {
            void Promise.resolve(u()).catch(() => {});
          } else {
            unlisten = u;
          }
        })
        .catch(() => {});
    } catch {
      // Outside Tauri runtime (Vitest)
    }

    return () => {
      disposed = true;
      if (unlisten) {
        void Promise.resolve(unlisten()).catch(() => {});
      }
    };
  }, [enqueueUploads]);

  return (
    <div
      className={className}
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100%',
        minHeight: 0,
        position: 'relative',
        ...style,
      }}
    >
      {children}
      {isDragOver && !disabled && (
        <div
          style={{
            position: 'absolute',
            inset: overlayInset,
            zIndex: 10,
            border: '2px dashed var(--accent)',
            borderRadius: 12,
            background: 'color-mix(in srgb, var(--bg-surface) 88%, transparent)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            fontSize: 16,
            fontWeight: 600,
            pointerEvents: 'none',
            padding: 24,
            textAlign: 'center',
          }}
        >
          {t.dropZone.overlay(targetDir)}
        </div>
      )}
    </div>
  );
}
