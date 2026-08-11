function normalizedIds(values) {
  return new Set((Array.isArray(values) ? values : [values])
    .filter((value) => value !== undefined && value !== null)
    .map((value) => String(value)));
}

export function invalidateRemoteDirectoryIds(cache, fileIds) {
  const ids = normalizedIds(fileIds);
  if (!ids.size) return 0;
  const roots = [...cache.entries()]
    .filter(([key, value]) => key !== '' && ids.has(String(value)))
    .map(([key]) => key);
  let removed = 0;
  for (const key of [...cache.keys()]) {
    if (key === '') continue;
    if (roots.some((root) => key === root || key.startsWith(`${root}/`))) {
      cache.delete(key);
      removed += 1;
    }
  }
  return removed;
}

export function reconcileRemoteDirectoryCache(
  cache,
  parentId,
  records,
  { complete = false, onConfirmed = () => {} } = {},
) {
  const normalizedParentId = String(parentId || '');
  const directories = new Map((Array.isArray(records) ? records : [])
    .filter((record) => Number(record?.resType ?? record?.type) === 2 || record?.isDirectory === true)
    .map((record) => [String(record.fileId ?? record.id ?? ''), String(record.fileName ?? record.name ?? '')])
    .filter(([id]) => id));
  const staleIds = [];
  for (const [key, cachedId] of cache) {
    if (key === '') continue;
    const separator = key.indexOf('::');
    if (separator < 0) continue;
    const baseParentId = key.slice(0, separator);
    const relativePath = key.slice(separator + 2);
    const lastSlash = relativePath.lastIndexOf('/');
    const name = relativePath.slice(lastSlash + 1);
    const cachedParentId = lastSlash < 0
      ? baseParentId
      : String(cache.get(`${baseParentId}::${relativePath.slice(0, lastSlash)}`) || '');
    if (cachedParentId !== normalizedParentId) continue;
    if (directories.get(String(cachedId)) === name) {
      try { onConfirmed(key, String(cachedId)); } catch { /* metadata hooks are best effort */ }
    } else if (complete) {
      staleIds.push(String(cachedId));
    }
  }
  return invalidateRemoteDirectoryIds(cache, staleIds);
}
