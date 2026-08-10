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
  mappingId: string
  state: string
  stage: string
  percent: number
  bytesPerSecond: number
  uploadedBytes: number
  totalBytes: number
  startedAt: number
  updatedAt: number
}

export interface DownloadTask {
  id: string
  fileName: string
  destination: string
  source: string
  packaged: boolean
  status: 'queued' | 'preparing' | 'downloading' | 'paused' | 'completed' | 'failed' | 'cancelled'
  started: boolean
  progress: number
  downloadedBytes: number
  totalBytes: number
  bytesPerSecond: number
  segmented: boolean
  connections: number
  filePath: string
  error: string
  createdAt: number
  updatedAt: number
}

export const useTransfersStore = defineStore('transfers', () => {
  const uploads = ref<Record<string, UploadTask>>({})
  const downloads = ref<DownloadTask[]>([])
  const downloadConcurrency = ref(2)
  const downloadPaused = ref(false)
  const queue = createConcurrencyQueue(() => downloadConcurrency.value, () => downloadPaused.value)
  const downloadJobs = new Map<string, () => Promise<void>>()
  const uploadCancelHandlers = new Map<string, () => void>()
  const uploadRetryHandlers = new Map<string, () => Promise<void> | void>()
  const queuePausedBrowserUploads = new Set<string>()
  const queuePausedDownloads = new Set<string>()
  const queueDownloadPausePromises = new Map<string, Promise<void>>()

  const orderedUploads = computed(() => orderUploadProgress(Object.values(uploads.value)) as UploadTask[])
  const activeUploads = computed(() => orderedUploads.value.filter(item => !['done', 'error', 'cancelled'].includes(item.state)))
  const uploadSpeed = computed(() => activeUploads.value.reduce((sum, item) => sum + Number(item.bytesPerSecond || 0), 0))
  const activeDownloads = computed(() => downloads.value.filter(item => ['queued', 'preparing', 'downloading', 'paused'].includes(item.status)))
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
      const restartsCancelled = payload.type === 'file'
        && ['queued', 'waiting-login', 'uploading'].includes(String(payload.state || ''))
      if (previous?.state === 'cancelled' && payload.state !== 'cancelled' && !restartsCancelled) return
      const next = nextUploadProgress(previous, payload)
      if (next !== previous) {
        uploads.value = {
          ...uploads.value,
          [key]: {
            filePath: key,
            fileName: uploadFileName(key),
            mappingId: String(payload.mapping_id || previous?.mappingId || ''),
            startedAt: previous?.startedAt || next.updatedAt,
            ...next,
          },
        }
      }
    }
    if (payload?.type === 'download' && payload.download_id) {
      const downloadId = String(payload.download_id)
      const current = downloads.value.find(item => item.id === downloadId)
      if (current?.status === 'cancelled' && payload.state !== 'cancelled') return
      if (current?.status === 'completed' && payload.state !== 'done') return
      if (current?.status === 'paused' && payload.state === 'downloading') return
      const status = payload.state === 'done'
        ? 'completed'
        : payload.state === 'error'
          ? 'failed'
          : payload.state === 'paused'
            ? 'paused'
            : payload.state === 'cancelled'
              ? 'cancelled'
              : 'downloading'
      if (['completed', 'failed', 'cancelled'].includes(status)) {
        queuePausedDownloads.delete(downloadId)
        queueDownloadPausePromises.delete(downloadId)
      }
      updateDownload(downloadId, {
        status,
        progress: payload.percent == null ? undefined : Number(payload.percent),
        downloadedBytes: payload.downloaded_bytes == null ? undefined : Number(payload.downloaded_bytes),
        totalBytes: payload.total_bytes == null ? undefined : Number(payload.total_bytes),
        bytesPerSecond: payload.bytes_per_second == null ? undefined : Number(payload.bytes_per_second),
        segmented: payload.segmented == null ? undefined : Boolean(payload.segmented),
        connections: payload.connections == null ? undefined : Number(payload.connections),
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
    const run = async () => {
      const current = downloads.value.find(item => item.id === task.id)
      if (!current || current.status === 'cancelled') return
      updateDownload(task.id, { status: 'preparing', started: true })
      try {
        const data = unwrapData(await bridge.invoke(command, {
          ...args,
          file_name: task.fileName,
          destination_dir: task.destination,
          download_id: task.id,
        }))
        const latest = downloads.value.find(item => item.id === task.id)
        if (latest?.status === 'cancelled') return
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
        const text = errorText(error)
        const latest = downloads.value.find(item => item.id === task.id)
        if (latest?.status === 'cancelled' || text.includes('下载已取消')) {
          updateDownload(task.id, { status: 'cancelled', error: '', bytesPerSecond: 0 })
        }
        else {
          updateDownload(task.id, { status: 'failed', error: text, bytesPerSecond: 0 })
          message.error(text)
        }
      }
      finally {
        const latest = downloads.value.find(item => item.id === task.id)
        if (!latest || ['completed', 'failed', 'cancelled'].includes(latest.status)) {
          downloadJobs.delete(task.id)
        }
      }
    }
    downloadJobs.set(task.id, run)
    queue.enqueue(task.id, run)
  }

  async function pauseDownload(id: string) {
    const task = downloads.value.find(item => item.id === id)
    if (!task || !['queued', 'preparing', 'downloading'].includes(task.status)) return
    if (task.status === 'queued' && queue.cancel(id)) {
      updateDownload(id, { status: 'paused', started: false, bytesPerSecond: 0 })
      return
    }
    await bridge.invoke('pause_download', { task_id: id })
    updateDownload(id, { status: 'paused', bytesPerSecond: 0 })
  }

  async function resumeDownload(id: string) {
    const task = downloads.value.find(item => item.id === id)
    if (!task || task.status !== 'paused') return
    if (downloadPaused.value) throw new Error('传输队列已暂停，请先恢复队列')
    if (!task.started) {
      const run = downloadJobs.get(id)
      if (!run) throw new Error('下载任务已经失效，请重新添加')
      updateDownload(id, { status: 'queued', error: '' })
      queue.enqueue(id, run)
      return
    }
    await bridge.invoke('resume_download', { task_id: id })
    updateDownload(id, { status: 'downloading', error: '' })
  }

  async function cancelDownload(id: string) {
    const task = downloads.value.find(item => item.id === id)
    if (!task || ['completed', 'failed', 'cancelled'].includes(task.status)) return
    queuePausedDownloads.delete(id)
    queueDownloadPausePromises.delete(id)
    if ((task.status === 'queued' && queue.cancel(id)) || (task.status === 'paused' && !task.started)) {
      downloadJobs.delete(id)
      updateDownload(id, { status: 'cancelled', bytesPerSecond: 0, error: '' })
      return
    }
    await bridge.invoke('cancel_download', { task_id: id })
    updateDownload(id, { status: 'cancelled', bytesPerSecond: 0, error: '' })
  }

  function registerUploadCancellation(filePath: string, cancel: () => void) {
    const key = String(filePath || '')
    if (!key || typeof cancel !== 'function') return () => {}
    uploadCancelHandlers.set(key, cancel)
    if (downloadPaused.value && uploads.value[key]?.state === 'uploading' && uploadRetryHandlers.has(key)) {
      queuePausedBrowserUploads.add(key)
      handleSyncEvent({
        type: 'file', state: 'paused', file_path: key,
        uploaded_bytes: uploads.value[key]?.uploadedBytes || 0,
        total_bytes: uploads.value[key]?.totalBytes || 0,
        stage: '队列已暂停，恢复后将重新传到服务器',
      })
      queueMicrotask(() => {
        if (uploadCancelHandlers.get(key) === cancel) cancel()
      })
    }
    return () => {
      if (uploadCancelHandlers.get(key) === cancel) uploadCancelHandlers.delete(key)
    }
  }

  function registerUploadRetry(filePath: string, retry: () => Promise<void> | void) {
    const key = String(filePath || '')
    if (!key || typeof retry !== 'function') return () => {}
    uploadRetryHandlers.set(key, retry)
    return () => {
      if (uploadRetryHandlers.get(key) === retry) uploadRetryHandlers.delete(key)
    }
  }

  function clearUploadRetry(filePath: string) {
    uploadRetryHandlers.delete(String(filePath || ''))
  }

  function isUploadPaused(filePath: string) {
    return uploads.value[String(filePath || '')]?.state === 'paused'
  }

  async function pauseUpload(filePath: string) {
    const key = String(filePath || '')
    const task = uploads.value[key]
    if (!task || !['queued', 'waiting-login', 'waiting-file', 'preparing', 'uploading'].includes(task.state)) return
    const localPause = uploadCancelHandlers.get(key)
    if (localPause) {
      handleSyncEvent({
        type: 'file', state: 'paused', file_path: key, mapping_id: task.mappingId,
        uploaded_bytes: task.uploadedBytes, total_bytes: task.totalBytes,
        stage: '已暂停，继续时将重新传到服务器',
      })
      localPause()
      return
    }
    const response = await bridge.invoke('pause_upload', { file_path: key, mapping_id: task.mappingId })
    const result = response && typeof response === 'object' && 'data' in response ? response.data : response
    if (result === false || result?.paused === false) throw new Error('上传任务已经结束或已失效')
  }

  async function resumeUpload(filePath: string) {
    const key = String(filePath || '')
    const task = uploads.value[key]
    if (!task || task.state !== 'paused') return
    if (downloadPaused.value) throw new Error('传输队列已暂停，请先恢复队列')
    const localRetry = uploadRetryHandlers.get(key)
    if (localRetry) {
      uploadRetryHandlers.delete(key)
      handleSyncEvent({
        type: 'file', state: 'queued', file_path: key, mapping_id: task.mappingId,
        uploaded_bytes: 0, total_bytes: task.totalBytes, stage: '正在恢复上传',
      })
      await localRetry()
      return
    }
    const response = await bridge.invoke('resume_upload', { file_path: key, mapping_id: task.mappingId })
    const result = response && typeof response === 'object' && 'data' in response ? response.data : response
    if (result === false || result?.resumed === false) throw new Error('暂停的上传任务已经失效')
  }

  async function retryUpload(filePath: string) {
    const key = String(filePath || '')
    const task = uploads.value[key]
    if (!task || task.state !== 'error') return
    const localRetry = uploadRetryHandlers.get(key)
    uploadRetryHandlers.delete(key)
    handleSyncEvent({
      type: 'file', state: 'queued', file_path: key, mapping_id: task.mappingId,
      uploaded_bytes: 0, total_bytes: task.totalBytes, stage: '正在重新上传',
    })
    try {
      if (localRetry) await localRetry()
      else await bridge.invoke('retry_upload', { file_path: key, mapping_id: task.mappingId })
    } catch (error) {
      const text = errorText(error)
      handleSyncEvent({
        type: 'file', state: 'error', file_path: key, mapping_id: task.mappingId,
        uploaded_bytes: task.uploadedBytes, total_bytes: task.totalBytes, error: text,
      })
      throw error
    }
  }

  async function cancelUpload(filePath: string) {
    const key = String(filePath || '')
    const task = uploads.value[key]
    if (!task || ['done', 'error', 'cancelled'].includes(task.state)) return
    uploadCancelHandlers.get(key)?.()
    handleSyncEvent({
      type: 'file',
      state: 'cancelled',
      file_path: key,
      mapping_id: task.mappingId,
      uploaded_bytes: task.uploadedBytes,
      total_bytes: task.totalBytes,
      stage: '已取消',
    })
    queuePausedBrowserUploads.delete(key)
    uploadRetryHandlers.delete(key)
    await bridge.invoke('cancel_upload', { file_path: key, mapping_id: task.mappingId })
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
      status: 'queued', started: false, progress: 0, downloadedBytes: 0, totalBytes: 0,
      bytesPerSecond: 0, segmented: false, connections: 1,
      filePath: '', error: '', createdAt: now, updatedAt: now,
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
      status: 'queued', started: false, progress: 0, downloadedBytes: 0, totalBytes: 0,
      bytesPerSecond: 0, segmented: false, connections: 1,
      filePath: '', error: '', createdAt: now, updatedAt: now,
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
      for (const [key, item] of Object.entries(uploads.value)) {
        if (['done', 'error', 'cancelled'].includes(item.state)) {
          uploadCancelHandlers.delete(key)
          uploadRetryHandlers.delete(key)
        }
      }
      uploads.value = Object.fromEntries(Object.entries(uploads.value).filter(([, item]) => !['done', 'error', 'cancelled'].includes(item.state)))
    }
    if (kind !== 'upload') {
      downloads.value = downloads.value.filter(item => !['completed', 'failed', 'cancelled'].includes(item.status))
    }
  }

  function setPaused(paused: boolean) {
    const wasPaused = downloadPaused.value
    downloadPaused.value = Boolean(paused)
    if (!wasPaused && downloadPaused.value) {
      for (const task of downloads.value) {
        if (!['queued', 'preparing', 'downloading'].includes(task.status)) continue
        queuePausedDownloads.add(task.id)
        const pausePromise = pauseDownload(task.id).catch(error => {
          queuePausedDownloads.delete(task.id)
          queueDownloadPausePromises.delete(task.id)
          message.error(errorText(error))
        })
        queueDownloadPausePromises.set(task.id, pausePromise)
      }
      for (const [key, cancel] of uploadCancelHandlers) {
        const task = uploads.value[key]
        if (!task || task.state !== 'uploading' || !uploadRetryHandlers.has(key)) continue
        queuePausedBrowserUploads.add(key)
        handleSyncEvent({
          type: 'file', state: 'paused', file_path: key, mapping_id: task.mappingId,
          uploaded_bytes: task.uploadedBytes, total_bytes: task.totalBytes,
          stage: '队列已暂停，恢复后将重新传到服务器',
        })
        cancel()
      }
    }
    if (!downloadPaused.value) queue.pump()
    if (wasPaused && !downloadPaused.value) {
      const resumable = [...queuePausedBrowserUploads]
      queuePausedBrowserUploads.clear()
      void (async () => {
        for (const key of resumable) {
          try { await resumeUpload(key) }
          catch (error) { message.error(errorText(error)) }
        }
      })()
      const downloadsToResume = [...queuePausedDownloads]
      queuePausedDownloads.clear()
      for (const id of downloadsToResume) {
        const pausePromise = queueDownloadPausePromises.get(id) || Promise.resolve()
        queueDownloadPausePromises.delete(id)
        void pausePromise
          .then(() => resumeDownload(id))
          .catch(error => message.error(errorText(error)))
      }
    }
  }

  return {
    uploads,
    downloads,
    downloadConcurrency,
    downloadPaused,
    orderedUploads,
    activeUploads,
    uploadSpeed,
    activeDownloads,
    overallPercent,
    handleSyncEvent,
    downloadRecords,
    downloadReceivedShare,
    cancelUpload,
    pauseUpload,
    resumeUpload,
    isUploadPaused,
    registerUploadCancellation,
    retryUpload,
    registerUploadRetry,
    clearUploadRetry,
    pauseDownload,
    resumeDownload,
    cancelDownload,
    updateDownload,
    setPaused,
    clearFinished,
  }
})
