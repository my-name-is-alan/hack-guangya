import crypto from 'node:crypto';
import path from 'node:path';

function replacementToken(randomUUID = crypto.randomUUID) {
  return String(randomUUID()).replaceAll('-', '');
}

export function createUploadReplacementContext({
  oldFileId,
  originalName,
  previousSize,
  previousModifiedMs,
  randomUUID,
}) {
  const normalizedOldFileId = String(oldFileId || '').trim();
  const normalizedOriginalName = path.posix.basename(String(originalName || '').replaceAll('\\', '/'));
  if (!normalizedOldFileId || !normalizedOriginalName) return null;
  return {
    oldFileId: normalizedOldFileId,
    originalName: normalizedOriginalName,
    temporaryName: `.__gy_replace_${replacementToken(randomUUID)}`,
    backupName: `.__gy_replace_backup_${replacementToken(randomUUID)}`,
    previousSize: Number(previousSize || 0),
    previousModifiedMs: String(previousModifiedMs || '0'),
  };
}

export function uploadRemoteName(item) {
  return String(item?.replacement?.temporaryName || path.basename(String(item?.file_path || '')));
}

function entryId(entry) {
  return String(entry?.id || entry?.fileId || '');
}

function entryName(entry) {
  return String(entry?.name || entry?.fileName || '');
}

function replacementState(entries, replacement, newFileId) {
  const oldEntry = entries.find((entry) => entryId(entry) === replacement.oldFileId) || null;
  const newEntry = entries.find((entry) => entryId(entry) === newFileId) || null;
  const originalEntry = entries.find((entry) => entryName(entry) === replacement.originalName) || null;
  if (originalEntry && entryId(originalEntry) !== replacement.oldFileId && entryId(originalEntry) !== newFileId) {
    return { kind: 'conflict', oldEntry, newEntry, originalEntry };
  }
  if (originalEntry && entryId(originalEntry) === newFileId) {
    return { kind: 'promoted', oldEntry, newEntry: originalEntry };
  }
  if (oldEntry && ![replacement.originalName, replacement.backupName].includes(entryName(oldEntry))) {
    return { kind: 'old-renamed-externally', oldEntry, newEntry, originalEntry };
  }
  if (oldEntry && entryName(oldEntry) === replacement.originalName) {
    return { kind: 'stage-old', oldEntry, newEntry, originalEntry };
  }
  return { kind: 'promote-new', oldEntry, newEntry, originalEntry };
}

export async function safelyReplaceUploadedFile({
  replacement,
  newFileId,
  listEntries,
  renameEntry,
  deleteEntry,
}) {
  if (!replacement) return { replaced: false };
  const normalizedNewFileId = String(newFileId || '').trim();
  if (!normalizedNewFileId) throw new Error('新文件已入库，但缺少文件 ID，无法安全覆盖');
  if (normalizedNewFileId === replacement.oldFileId) return { replaced: true, alreadyCurrent: true };

  let entries = await listEntries();
  let state = replacementState(entries, replacement, normalizedNewFileId);
  if (state.kind === 'conflict') {
    throw new Error(`云端“${replacement.originalName}”已被其他文件占用；新版本保留为临时文件，未覆盖现有文件`);
  }
  if (state.kind === 'old-renamed-externally') {
    throw new Error(`原云端文件已被改名为“${entryName(state.oldEntry)}”；新版本保留为临时文件，未覆盖外部改动`);
  }
  if (state.kind === 'stage-old') {
    await renameEntry(replacement.oldFileId, replacement.backupName);
    entries = await listEntries();
    state = replacementState(entries, replacement, normalizedNewFileId);
  }
  if (state.kind === 'conflict' || state.kind === 'old-renamed-externally') {
    throw new Error(`云端文件在替换期间发生变化；新版本保留为临时文件，未覆盖外部改动`);
  }
  if (state.kind !== 'promoted') {
    try {
      await renameEntry(normalizedNewFileId, replacement.originalName);
    } catch (error) {
      if (state.oldEntry && entryName(state.oldEntry) === replacement.backupName) {
        try {
          await renameEntry(replacement.oldFileId, replacement.originalName);
        } catch (rollbackError) {
          throw new Error(`${error.message}；恢复旧文件名也失败：${rollbackError.message}`);
        }
      }
      throw error;
    }
  }
  if (state.oldEntry || entries.some((entry) => entryId(entry) === replacement.oldFileId)) {
    await deleteEntry(replacement.oldFileId);
  }
  return { replaced: true };
}

export function restorePreviousUploadRecord(row) {
  let item = null;
  try { item = JSON.parse(row?.item_json || 'null'); } catch {}
  const replacement = item?.replacement;
  if (!replacement?.oldFileId) return null;
  return {
    remoteFileId: String(replacement.oldFileId),
    size: Number(replacement.previousSize || 0),
    modifiedMs: String(replacement.previousModifiedMs || '0'),
  };
}
