import { useCallback, useMemo, useState } from 'react';
import type { DeleteTargetItem } from '../components/DeleteFilesDialog';
import { getMessages, resolveLanguage } from './i18n';
import { ipc } from './ipc';
import { useAppStore } from '../store/useAppStore';

export interface SelectableMediaItem {
  path: string;
  fileSize: number;
  name?: string;
  title?: string;
}

/**
 * Shared multi-select state and bulk actions (Download, Copy to folder, Delete with confirmation)
 * for Photos, Videos, and Music views.
 */
export function useMediaSelection<T extends SelectableMediaItem>(items: T[]) {
  const enqueueDownloads = useAppStore((s) => s.enqueueDownloads);
  const deleteFiles = useAppStore((s) => s.deleteFiles);
  const setToastError = useAppStore((s) => s.setToastError);
  const setStatusNotice = useAppStore((s) => s.setStatusNotice);

  const [multiSelect, setMultiSelect] = useState(false);
  const [selectedPaths, setSelectedPaths] = useState<string[]>([]);
  const [deleteTargets, setDeleteTargets] = useState<DeleteTargetItem[]>([]);

  const visiblePathSet = useMemo(() => new Set(items.map((it) => it.path)), [items]);

  const selectedSet = useMemo(() => {
    const next = new Set<string>();
    for (const p of selectedPaths) {
      if (visiblePathSet.has(p)) next.add(p);
    }
    return next;
  }, [selectedPaths, visiblePathSet]);

  const selectedItems = useMemo(
    () => items.filter((it) => selectedSet.has(it.path)),
    [items, selectedSet],
  );

  const allSelected = items.length > 0 && selectedItems.length === items.length;

  const toggleMultiSelect = useCallback(() => {
    setMultiSelect((prev) => !prev);
    setSelectedPaths([]);
  }, []);

  const exitMultiSelect = useCallback(() => {
    setMultiSelect(false);
    setSelectedPaths([]);
  }, []);

  const clearSelection = useCallback(() => {
    setSelectedPaths([]);
  }, []);

  const toggleSelectPath = useCallback((path: string) => {
    setSelectedPaths((cur) =>
      cur.includes(path) ? cur.filter((p) => p !== path) : [...cur, path],
    );
  }, []);

  const selectAll = useCallback(
    (checked: boolean) => {
      if (!checked) {
        setSelectedPaths([]);
        return;
      }
      setSelectedPaths(Array.from(new Set(items.map((it) => it.path))));
    },
    [items],
  );

  const downloadSelected = useCallback(async () => {
    if (selectedItems.length === 0) return;
    await enqueueDownloads(
      selectedItems.map((it) => ({
        remotePath: it.path,
        isDirectory: false,
        fileSize: it.fileSize,
      })),
    );
  }, [enqueueDownloads, selectedItems]);

  const copySelectedToFolder = useCallback(async () => {
    if (selectedItems.length === 0) return;
    try {
      const dirs = await ipc.pickLocalPaths(true, false);
      const destDir = dirs[0];
      if (!destDir) return;
      const targets = selectedItems.map((it) => ({
        remotePath: it.path,
        isDirectory: false,
        fileSize: it.fileSize,
      }));
      await enqueueDownloads(targets, destDir);
      const t = getMessages(resolveLanguage(useAppStore.getState().settings.language));
      setStatusNotice(t.mediaSelect.copiedToFolderNotice(targets.length, destDir));
    } catch (e) {
      setToastError(String(e));
    }
  }, [enqueueDownloads, selectedItems, setStatusNotice, setToastError]);

  const requestDeleteSelected = useCallback(() => {
    if (selectedItems.length === 0) return;
    setDeleteTargets(
      selectedItems.map((it) => ({
        path: it.path,
        name: it.title ?? it.name,
      })),
    );
  }, [selectedItems]);

  const requestDeleteSingle = useCallback((item: DeleteTargetItem) => {
    setDeleteTargets([item]);
  }, []);

  const closeDeleteDialog = useCallback(() => {
    setDeleteTargets([]);
  }, []);

  const confirmDelete = useCallback(
    async (paths: string[], isTrash: boolean) => {
      const ok = await deleteFiles(paths, isTrash);
      if (ok) {
        const deletedSet = new Set(paths);
        setSelectedPaths((cur) => cur.filter((p) => !deletedSet.has(p)));
      }
    },
    [deleteFiles],
  );

  return {
    multiSelect,
    selectedSet,
    selectedItems,
    allSelected,
    deleteTargets,
    toggleMultiSelect,
    exitMultiSelect,
    clearSelection,
    toggleSelectPath,
    selectAll,
    downloadSelected,
    copySelectedToFolder,
    requestDeleteSelected,
    requestDeleteSingle,
    closeDeleteDialog,
    confirmDelete,
  };
}
