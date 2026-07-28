import { computed, ref, shallowRef } from 'vue'
import { defineStore } from 'pinia'
import { bridge } from '../bridge.js'
import { unwrapData } from '../formatters.js'

export interface CloudPathItem {
  id: string
  name: string
}

export const useFilesStore = defineStore('files', () => {
  const files = ref<any[]>([])
  const currentPath = ref<CloudPathItem[]>([{ id: '', name: '全部文件' }])
  const loading = shallowRef(false)
  const page = shallowRef(0)
  const total = shallowRef(0)
  const pageSize = 100

  const currentFolderId = computed(() => currentPath.value.at(-1)?.id || '')
  const currentFolderName = computed(() => currentPath.value.at(-1)?.name || '全部文件')

  async function loadFiles(nextPage = 0) {
    loading.value = true
    try {
      const normalizedPage = Math.max(0, Math.floor(Number(nextPage) || 0))
      const data = unwrapData(await bridge.invoke('list_files', {
        page: normalizedPage,
        parent_id: currentFolderId.value,
      }))
      const nextFiles = Array.isArray(data.list) ? data.list : []
      files.value = nextFiles
      page.value = normalizedPage
      total.value = Math.max(nextFiles.length, Number(data.total ?? nextFiles.length) || 0)
      return files.value
    }
    finally {
      loading.value = false
    }
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
    enterFolder,
    goBack,
    jumpTo,
    reset,
  }
})
