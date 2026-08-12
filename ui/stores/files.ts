import { computed, ref, shallowRef } from 'vue'
import { defineStore } from 'pinia'
import { bridge } from '../bridge.js'
import {
  fileListCacheKey,
  normalizeDirectoryInvalidation,
  shouldApplyFileListResponse,
  shouldInvalidateFileListCache,
  withFileListTimeout,
} from '../fileListRequest.js'
import { unwrapData } from '../formatters.js'

export interface CloudPathItem {
  id: string
  name: string
}

interface FileLoadOptions {
  background?: boolean
  force?: boolean
  preserveCurrent?: boolean
}

export const useFilesStore = defineStore('files', () => {
  const files = ref<any[]>([])
  const currentPath = ref<CloudPathItem[]>([{ id: '', name: '全部文件' }])
  const loading = shallowRef(false)
  // 回收站内容版本号：删除/恢复/彻底删除/清空后自增，回收站面板据此刷新。
  const recycleBinVersion = shallowRef(0)
  const page = shallowRef(0)
  const total = shallowRef(0)
  const pageSize = 100
  const cache = new Map<string, { files: any[], total: number }>()
  let latestRequestId = 0
  let foregroundRequests = 0
  let invalidationRefreshTimer: ReturnType<typeof setTimeout> | null = null
  // 失效代际：请求发出后若发生过目录失效，返回的旧数据不允许回写缓存。
  let invalidationGeneration = 0
  // 失效刷新去抖：批量写操作会连续发失效事件，纯 debounce 会把刷新无限推
  // 迟，这里带最大等待时间保证批量期间列表仍会周期性更新。
  let invalidationRefreshDeadline = 0
  const INVALIDATION_REFRESH_DELAY_MS = 200
  const INVALIDATION_REFRESH_MAX_WAIT_MS = 1500

  function remember(key: string, value: { files: any[], total: number }) {
    cache.delete(key)
    cache.set(key, value)
    while (cache.size > 50) cache.delete(cache.keys().next().value as string)
  }

  function cancelInvalidationRefresh() {
    if (invalidationRefreshTimer) clearTimeout(invalidationRefreshTimer)
    invalidationRefreshTimer = null
  }

  const currentFolderId = computed(() => currentPath.value.at(-1)?.id || '')
  const currentFolderName = computed(() => currentPath.value.at(-1)?.name || '全部文件')

  async function loadFiles(nextPage = 0, options: FileLoadOptions = {}) {
    const normalizedPage = Math.max(0, Math.floor(Number(nextPage) || 0))
    const requestedFolderId = currentFolderId.value
    const cacheKey = fileListCacheKey(requestedFolderId, normalizedPage)
    // 用户点"刷新"时不展示旧缓存，避免"刷新了但内容没变"的错觉。
    const cached = options.force ? undefined : cache.get(cacheKey)
    const requestId = ++latestRequestId
    const generationAtStart = invalidationGeneration
    if (!options.background) {
      foregroundRequests += 1
      loading.value = true
    }
    if (cached) {
      files.value = cached.files
      page.value = normalizedPage
      total.value = cached.total
    }
    else if (!options.preserveCurrent) {
      files.value = []
      page.value = normalizedPage
      total.value = 0
    }
    try {
      const data = unwrapData(await withFileListTimeout(bridge.invoke('list_files', {
        page: normalizedPage,
        parent_id: requestedFolderId,
        force_refresh: options.force === true,
      })))
      if (!shouldApplyFileListResponse(requestId, latestRequestId, requestedFolderId, currentFolderId.value)) {
        return files.value
      }
      const nextFiles = Array.isArray(data.list) ? data.list : []
      const nextTotal = Math.max(nextFiles.length, Number(data.total ?? nextFiles.length) || 0)
      files.value = nextFiles
      page.value = normalizedPage
      total.value = nextTotal
      // 请求在途期间目录被失效过：这份数据可能已经过期，只展示不回写缓存
      // （随后的失效刷新会拉到新数据并纠正显示）。
      if (invalidationGeneration === generationAtStart) {
        remember(cacheKey, { files: nextFiles, total: nextTotal })
      }
      return files.value
    }
    catch (error) {
      if (!shouldApplyFileListResponse(requestId, latestRequestId, requestedFolderId, currentFolderId.value)) {
        return files.value
      }
      if (cached) {
        const message = String((error as Error)?.message || error || '目录加载失败')
        throw new Error(`${message}；已显示最近一次成功内容`)
      }
      throw error
    }
    finally {
      if (!options.background) {
        foregroundRequests = Math.max(0, foregroundRequests - 1)
        loading.value = foregroundRequests > 0
      }
    }
  }

  function handleDirectoryInvalidation(payload: any = {}) {
    const invalidation = normalizeDirectoryInvalidation(payload)
    invalidationGeneration += 1
    for (const key of [...cache.keys()]) {
      if (shouldInvalidateFileListCache(key, invalidation)) cache.delete(key)
    }
    if (!invalidation.all && !invalidation.parentIds.includes(currentFolderId.value)) return
    const now = Date.now()
    if (!invalidationRefreshTimer) {
      invalidationRefreshDeadline = now + INVALIDATION_REFRESH_MAX_WAIT_MS
    }
    const delay = Math.min(INVALIDATION_REFRESH_DELAY_MS, Math.max(0, invalidationRefreshDeadline - now))
    cancelInvalidationRefresh()
    invalidationRefreshTimer = setTimeout(() => {
      invalidationRefreshTimer = null
      void loadFiles(page.value, { background: true, preserveCurrent: true }).catch(() => {})
    }, delay)
  }

  function handleRecycleBinChanged() {
    recycleBinVersion.value += 1
  }

  /// 写操作完成后的统一刷新入口：取消挂起的失效刷新（避免重复请求）、
  /// 保留当前内容避免闪空，并带 force 绕过前后端缓存。
  async function refreshAfterMutation(nextPage: number = page.value) {
    cancelInvalidationRefresh()
    invalidationGeneration += 1
    return loadFiles(nextPage, { force: true, preserveCurrent: true })
  }

  async function enterFolder(record: any) {
    currentPath.value = [...currentPath.value, {
      id: String(record.fileId || record.id || ''),
      name: String(record.fileName || record.name || '文件夹'),
    }]
    return loadFiles(0)
  }

  async function goBack() {
    if (currentPath.value.length <= 1) return
    currentPath.value = currentPath.value.slice(0, -1)
    return loadFiles(0)
  }

  async function jumpTo(index: number) {
    if (index < 0 || index >= currentPath.value.length) return
    currentPath.value = currentPath.value.slice(0, index + 1)
    return loadFiles(0)
  }

  function reset() {
    latestRequestId += 1
    invalidationGeneration += 1
    cache.clear()
    cancelInvalidationRefresh()
    foregroundRequests = 0
    loading.value = false
    files.value = []
    currentPath.value = [{ id: '', name: '全部文件' }]
    page.value = 0
    total.value = 0
  }

  return {
    files,
    currentPath,
    loading,
    page,
    total,
    pageSize,
    recycleBinVersion,
    currentFolderId,
    currentFolderName,
    loadFiles,
    handleDirectoryInvalidation,
    handleRecycleBinChanged,
    refreshAfterMutation,
    enterFolder,
    goBack,
    jumpTo,
    reset,
  }
})
