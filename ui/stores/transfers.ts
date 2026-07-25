import { computed, ref } from 'vue'
import { defineStore } from 'pinia'
import { message } from 'antdv-next'
import { bridge, isTauri } from '../bridge.js'
import { errorText, fileId, isFolder, newDownloadId, pick, unwrapData, uploadFileName } from '../formatters.js'
import { createConcurrencyQueue } from '../transferQueue.js'
import { nextUploadProgress, orderUploadProgress } from '../uploadProgress.js'

export interface UploadTask {
  filePath: string
  fileName: string
  state: string
  stage: string
  percent: number
  bytesPerSecond: number
  startedAt: number
  updatedAt: number
}

export interface DownloadTask {
  id: string
  fileName: string
  destination: string
  source: string
  packaged: boolean
  status: 'queued' | 'preparing' | 'downloading' | 'completed' | 'failed'
  progress: number
  downloadedBytes: number
  totalBytes: number
  bytesPerSecond: number
  filePath: string
  error: string
  createdAt: number
  updatedAt: number
}

export const useTransfersStore = defineStore('transfers', () => {
  const uploads = ref<Record<string, UploadTask>>({})
  const downloads = ref<DownloadTask[]>([])
  const downloadConcurrency = ref(2)
  const queue = createConcurrencyQueue(() => downloadConcurrency.value)

  const orderedUploads = computed(() => orderUploadProgress(Object.values(uploads.value)) as UploadTask[])
  const activeUploads = computed(() => orderedUploads.value.filter(item => !['done', 'error'].includes(item.state)))
  const uploadSpeed = computed(() => activeUploads.value.reduce((sum, item) => sum + Number(item.bytesPerSecond || 0), 0))
  const activeDownloads = computed(() => downloads.value.filter(item => ['queued', 'preparing', 'downloading'].includes(item.status)))
  const overallPercent = computed(() => {
    const activeCount = activeUploads.value.length + activeDownloads.value.length
    if (!activeCount) return 0
    const uploadedPercent = activeUploads.value.reduce((sum, item) => sum + Number(item.percent || 0), 0)
    const downloadedPercent = activeDownloads.value.reduce((sum, item) => sum + Number(item.progress || 0), 0)
    return Math.round((uploadedPercent + downloadedPercent) / activeCount)
  })

  function handleSyncEvent(payload: any) {
    if ((payload?.type === 'progress' || payload?.type === 'file') && payload.file_path) {
      const key = String(payload.file_path)
      const previous = uploads.value[key]
      const next = nextUploadProgress(previous, payload)
      if (next !== previous) {
        uploads.value = {
          ...uploads.value,
          [key]: {
            filePath: key,
            fileName: uploadFileName(key),
            startedAt: previous?.startedAt || next.updatedAt,
            ...next,
          },
        }
      }
    }
    if (payload?.type === 'download' && payload.download_id) {
      updateDownload(String(payload.download_id), {
        status: payload.state === 'done' ? 'completed' : payload.state === 'error' ? 'failed' : 'downloading',
        progress: payload.percent == null ? undefined : Number(payload.percent),
        downloadedBytes: payload.downloaded_bytes == null ? undefined : Number(payload.downloaded_bytes),
        totalBytes: payload.total_bytes == null ? undefined : Number(payload.total_bytes),
        bytesPerSecond: payload.bytes_per_second == null ? undefined : Number(payload.bytes_per_second),
        filePath: payload.file_path || undefined,
        error: payload.error || undefined,
      })
    }
  }

  function updateDownload(id: string, changes: Partial<DownloadTask> & Record<string, any>) {
    downloads.value = downloads.value.map(task => task.id === id
      ? { ...task, ...Object.fromEntries(Object.entries(changes).filter(([, value]) => value !== undefined)), updatedAt: Date.now() }
      : task)
  }

  function localDownloadName(records: any[], packaged: boolean) {
    const name = String(pick(records[0], ['fileName', 'name'], '') || '').trim()
    if (!packaged) return name || '光鸭下载'
    if (records.length === 1 && name) return /\.zip$/i.test(name) ? name : `${name}.zip`
    return `光鸭批量下载-${new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19)}.zip`
  }

  function enqueueLocalDownload(command: string, args: Record<string, unknown>, task: DownloadTask) {
    downloads.value = [task, ...downloads.value]
    queue.enqueue(async () => {
      updateDownload(task.id, { status: 'preparing' })
      try {
        const data = unwrapData(await bridge.invoke(command, {
          ...args,
          file_name: task.fileName,
          destination_dir: task.destination,
          download_id: task.id,
        }))
        updateDownload(task.id, {
          status: 'completed',
          progress: 100,
          filePath: data.file_path || '',
          downloadedBytes: Number(data.bytes || 0),
          bytesPerSecond: 0,
        })
        message.success(`下载完成：${data.file_path || task.fileName}`)
      }
      catch (error) {
        updateDownload(task.id, { status: 'failed', error: errorText(error), bytesPerSecond: 0 })
        message.error(errorText(error))
      }
    })
  }

  async function downloadRecords(records: any[]) {
    const targets = (Array.isArray(records) ? records : []).filter(Boolean)
    if (!targets.length) throw new Error('请先选择要下载的文件或文件夹')
    const packaged = targets.length !== 1 || isFolder(targets[0])
    const ids = targets.map(fileId).filter(Boolean)
    const fileName = localDownloadName(targets, packaged)

    if (!isTauri) {
      const data = unwrapData(await bridge.invoke('get_cloud_download', { file_ids: ids, packaged }))
      const url = String(data.download_url || data.downloadUrl || data.url || '')
      if (!url) throw new Error('未获取到下载地址')
      window.open(url, '_blank', 'noopener,noreferrer')
      return true
    }

    const selected = await bridge.selectFolder()
    if (typeof selected !== 'string' || !selected) return false
    const destination = selected
    const now = Date.now()
    const task: DownloadTask = {
      id: newDownloadId(), fileName, destination, source: '我的文件', packaged,
      status: 'queued', progress: 0, downloadedBytes: 0, totalBytes: 0,
      bytesPerSecond: 0, filePath: '', error: '', createdAt: now, updatedAt: now,
    }
    enqueueLocalDownload('get_cloud_download', { file_ids: ids, packaged }, task)
    return true
  }

  async function downloadReceivedShare(records: any[], accessToken: string) {
    const targets = (Array.isArray(records) ? records : []).filter(Boolean)
    if (!targets.length) throw new Error('请先选择要下载的文件或文件夹')
    if (!accessToken.trim()) throw new Error('分享访问令牌已失效，请重新打开分享')
    const packaged = targets.length !== 1 || isFolder(targets[0])
    const ids = targets.map(fileId).filter(Boolean)
    const fileName = localDownloadName(targets, packaged)

    if (!isTauri) {
      const data = unwrapData(await bridge.invoke('get_received_share_download', {
        access_token: accessToken,
        file_ids: ids,
        packaged,
      }))
      const url = String(data.download_url || data.downloadUrl || data.url || '')
      if (!url) throw new Error('未获取到下载地址')
      window.open(url, '_blank', 'noopener,noreferrer')
      return true
    }

    const selected = await bridge.selectFolder()
    if (typeof selected !== 'string' || !selected) return false
    const now = Date.now()
    const task: DownloadTask = {
      id: newDownloadId(), fileName, destination: selected, source: '接收分享', packaged,
      status: 'queued', progress: 0, downloadedBytes: 0, totalBytes: 0,
      bytesPerSecond: 0, filePath: '', error: '', createdAt: now, updatedAt: now,
    }
    enqueueLocalDownload('get_received_share_download', {
      access_token: accessToken,
      file_ids: ids,
      packaged,
    }, task)
    return true
  }

  function clearFinished(kind: 'upload' | 'download' | 'all' = 'all') {
    if (kind !== 'download') {
      uploads.value = Object.fromEntries(Object.entries(uploads.value).filter(([, item]) => !['done', 'error'].includes(item.state)))
    }
    if (kind !== 'upload') {
      downloads.value = downloads.value.filter(item => !['completed', 'failed'].includes(item.status))
    }
  }

  return {
    uploads,
    downloads,
    downloadConcurrency,
    orderedUploads,
    activeUploads,
    uploadSpeed,
    activeDownloads,
    overallPercent,
    handleSyncEvent,
    downloadRecords,
    downloadReceivedShare,
    updateDownload,
    clearFinished,
  }
})
