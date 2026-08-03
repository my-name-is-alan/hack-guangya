import { computed, shallowRef } from 'vue'
import { defineStore } from 'pinia'
import { bridge, isTauri } from '../bridge.js'
import { errorText, unwrapData } from '../formatters.js'

const AUTO_CHECK_KEY = 'guangya:auto-update'

export interface AppUpdateMetadata {
  version: string
  current_version: string
  notes: string
  published_at?: string | null
}

function loadAutoCheckPreference() {
  try {
    return localStorage.getItem(AUTO_CHECK_KEY) !== 'false'
  } catch {
    return true
  }
}

export const useUpdaterStore = defineStore('updater', () => {
  const currentVersion = shallowRef('')
  const availableUpdate = shallowRef<AppUpdateMetadata | null>(null)
  const autoCheckEnabled = shallowRef(loadAutoCheckPreference())
  const checking = shallowRef(false)
  const installing = shallowRef(false)
  const downloadedBytes = shallowRef(0)
  const totalBytes = shallowRef(0)
  const lastCheckedAt = shallowRef<Date | null>(null)
  const error = shallowRef('')
  const promptOpen = shallowRef(false)
  let initialized = false
  let unsubscribe: (() => void) | null = null

  const progressPercent = computed(() => totalBytes.value > 0
    ? Math.min(100, Math.round(downloadedBytes.value / totalBytes.value * 100))
    : 0)

  function handleUpdateEvent(payload: any) {
    if (payload?.type !== 'app-update') return
    if (payload.event === 'started') {
      installing.value = true
      downloadedBytes.value = 0
      totalBytes.value = 0
    }
    if (payload.event === 'progress') {
      downloadedBytes.value = Number(payload.downloaded || 0)
      totalBytes.value = Number(payload.total || 0)
    }
    if (payload.event === 'downloaded') {
      if (totalBytes.value > 0) downloadedBytes.value = totalBytes.value
    }
  }

  async function loadCurrentVersion() {
    if (!isTauri) return
    const data = unwrapData(await bridge.invoke('get_app_version'))
    currentVersion.value = String(data.version || '')
  }

  async function checkForUpdates(silent = false) {
    if (!isTauri || checking.value || installing.value) return null
    checking.value = true
    error.value = ''
    try {
      if (!currentVersion.value) await loadCurrentVersion()
      const update = unwrapData(await bridge.invoke('fetch_app_update')) as AppUpdateMetadata | null
      availableUpdate.value = update
      promptOpen.value = Boolean(update)
      lastCheckedAt.value = new Date()
      return update
    } catch (reason) {
      error.value = errorText(reason)
      if (!silent) throw reason
      return null
    } finally {
      checking.value = false
    }
  }

  async function installUpdate() {
    if (!isTauri || !availableUpdate.value || installing.value) return
    installing.value = true
    error.value = ''
    downloadedBytes.value = 0
    totalBytes.value = 0
    try {
      await bridge.invoke('install_app_update')
    } catch (reason) {
      installing.value = false
      error.value = errorText(reason)
      throw reason
    }
  }

  function setAutoCheckEnabled(enabled: boolean) {
    autoCheckEnabled.value = enabled
    try {
      localStorage.setItem(AUTO_CHECK_KEY, String(enabled))
    } catch {
      // 浏览器禁用持久化时保留本次会话内的选择。
    }
  }

  function dismissPrompt() {
    if (!installing.value) promptOpen.value = false
  }

  async function initialize() {
    if (!isTauri || initialized) return
    initialized = true
    unsubscribe = await bridge.subscribe(handleUpdateEvent)
    try {
      await loadCurrentVersion()
      if (autoCheckEnabled.value) await checkForUpdates(true)
    } catch (reason) {
      error.value = errorText(reason)
    }
  }

  function dispose() {
    unsubscribe?.()
    unsubscribe = null
    initialized = false
  }

  return {
    currentVersion,
    availableUpdate,
    autoCheckEnabled,
    checking,
    installing,
    downloadedBytes,
    totalBytes,
    lastCheckedAt,
    error,
    promptOpen,
    progressPercent,
    checkForUpdates,
    installUpdate,
    setAutoCheckEnabled,
    dismissPrompt,
    initialize,
    dispose,
  }
})
