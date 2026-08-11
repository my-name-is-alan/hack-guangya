export const FILE_LIST_UI_TIMEOUT_MS = 14_000

export function fileListCacheKey(folderId, page) {
  return `${String(folderId || '')}\u0000${Math.max(0, Math.floor(Number(page) || 0))}`
}

export function fileListCacheFolderId(cacheKey) {
  const separator = String(cacheKey || '').indexOf('\u0000')
  return separator < 0 ? String(cacheKey || '') : String(cacheKey || '').slice(0, separator)
}

export function normalizeDirectoryInvalidation(payload = {}) {
  const parentIds = Array.isArray(payload.parent_ids)
    ? payload.parent_ids
    : Array.isArray(payload.parentIds)
      ? payload.parentIds
      : []
  return {
    all: payload.all === true,
    parentIds: [...new Set(parentIds.map(value => String(value ?? '')))],
  }
}

export function shouldInvalidateFileListCache(cacheKey, invalidation) {
  return invalidation.all === true
    || invalidation.parentIds.includes(fileListCacheFolderId(cacheKey))
}

export function shouldApplyFileListResponse(requestId, latestRequestId, folderId, currentFolderId) {
  return requestId === latestRequestId && String(folderId || '') === String(currentFolderId || '')
}

export function withFileListTimeout(task, timeoutMs = FILE_LIST_UI_TIMEOUT_MS) {
  let timer
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`文件目录加载超过 ${Math.ceil(timeoutMs / 1000)} 秒，请重试`)), timeoutMs)
  })
  return Promise.race([Promise.resolve(task), timeout]).finally(() => clearTimeout(timer))
}
