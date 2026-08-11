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
  const page = shallowRef(0)
  const total = shallowRef(0)
  const pageSize = 100
  const cache = new Map<string, { files: any[], total: number }>()
  let latestRequestId = 0
  let foregroundRequests = 0
  let invalidationRefreshTimer: ReturnType<typeof setTimeout> | null = null

  function remember(key: string, value: { files: any[], total: number }) {
    cache.delete(key)
    cache.set(key, value)
    while (cache.size > 50) cache.delete(cache.keys().next().value as string)
  }

  const currentFolderId = computed(() => currentPath.value.at(-1)?.id || '')
  const currentFolderName = computed(() => currentPath.value.at(-1)?.name || '全部文件')

  async function loadFiles(nextPage = 0, options: FileLoadOptions = {}) {
    const normalizedPage = Math.max(0, Math.floor(Number(nextPage) || 0))
    const requestedFolderId = currentFolderId.value
    const cacheKey = fileListCacheKey(requestedFolderId, normalizedPage)
    const cached = cache.get(cacheKey)
    const requestId = ++latestRequestId
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
      remember(cacheKey, { files: nextFiles, total: nextTotal })
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
    for (const key of [...cache.keys()]) {
      if (shouldInvalidateFileListCache(key, invalidation)) cache.delete(key)
    }
    if (!invalidation.all && !invalidation.parentIds.includes(currentFolderId.value)) return
    if (invalidationRefreshTimer) clearTimeout(invalidationRefreshTimer)
    invalidationRefreshTimer = setTimeout(() => {
      invalidationRefreshTimer = null
      void loadFiles(page.value, { background: true, preserveCurrent: true }).catch(() => {})
    }, 150)
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
    cache.clear()
    if (invalidationRefreshTimer) clearTimeout(invalidationRefreshTimer)
    invalidationRefreshTimer = null
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
    currentFolderId,
    currentFolderName,
    loadFiles,
    handleDirectoryInvalidation,
    enterFolder,
    goBack,
    jumpTo,
    reset,
  }
})
