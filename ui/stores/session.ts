import { computed, reactive, shallowRef } from 'vue'
import { defineStore } from 'pinia'
import { bridge, isTauri } from '../bridge.js'
import { formatSize, normalizeAvatarUrl, pick, unwrapData } from '../formatters.js'
import { useFilesStore } from './files'
import { useTransfersStore } from './transfers'

interface LogEntry {
  id: string
  level: string
  text: string
  time: string
}

const defaultState = () => ({
  logged_in: false,
  paused: false,
  pending: 0,
  active_uploads: 0,
  upload_concurrency: 2,
  download_concurrency: 2,
  multipart_part_size: 'auto',
  mappings: [] as any[],
  saved_shares: [] as any[],
  share_links: [] as any[],
  auto_share_receipts: [] as any[],
  auto_share_events: [] as any[],
  hdhive: { enabled: true, configured: false, base_url: '', instance_id: '' },
})

export const useSessionStore = defineStore('session', () => {
  const state = reactive(defaultState())
  const overview = reactive<{ profile: Record<string, any>, assets: Record<string, any> }>({ profile: {}, assets: {} })
  const logs = reactive<LogEntry[]>([])
  const bootLoading = shallowRef(true)
  const accessChecked = shallowRef(isTauri)
  const accessRequired = shallowRef(false)
  const accessGranted = shallowRef(isTauri)
  const accessUsername = shallowRef('admin')
  const initializing = shallowRef(false)
  const forceAuth = shallowRef(false)

  let unsubscribe: (() => void) | null = null

  const userName = computed(() => pick(overview.profile, [
    'nickname', 'nickName', 'name', 'userName', 'displayName',
  ], state.logged_in ? '光鸭用户' : '未登录'))
  const userAvatar = computed(() => normalizeAvatarUrl(pick(overview.profile, [
    'picture', 'avatar', 'avatarUrl', 'avatar_url', 'photoUrl', 'photo_url',
    'headImgUrl', 'headImageUrl', 'headPic',
  ], '')))
  const usedSpace = computed(() => Number(pick(overview.assets, [
    'usedSpaceSize', 'usedSpace', 'useSpace', 'used', 'usedSize',
  ], '0')))
  const totalSpace = computed(() => Number(pick(overview.assets, [
    'totalSpaceSize', 'totalSpace', 'capacity', 'total', 'totalSize',
  ], '0')))
  const remainingSpace = computed(() => Math.max(totalSpace.value - usedSpace.value, 0))
  const remainingSpaceLabel = computed(() => totalSpace.value ? formatSize(remainingSpace.value) : '—')
  const quotaPercent = computed(() => totalSpace.value
    ? Math.min(100, Math.round(usedSpace.value / totalSpace.value * 100))
    : 0)

  function appendLog(level: string, text: string) {
    logs.unshift({
      id: `${Date.now()}-${Math.random().toString(16).slice(2)}`,
      level,
      text,
      time: new Date().toLocaleTimeString(),
    })
    if (logs.length > 100) logs.splice(100)
  }

  function applyState(payload: any = {}) {
    const next = unwrapData(payload)
    Object.assign(state, next)
    if (Array.isArray(next.saved_shares)) state.share_links = next.saved_shares
    if (Array.isArray(next.share_links)) state.saved_shares = next.share_links
    if (Array.isArray(next.auto_share_receipts)) state.auto_share_events = next.auto_share_receipts
    if (Array.isArray(next.auto_share_events)) state.auto_share_receipts = next.auto_share_events
    if (next.hdhive) Object.assign(state.hdhive, next.hdhive)

    const transfers = useTransfersStore()
    transfers.downloadConcurrency = Number(state.download_concurrency || 2)
    if (!state.logged_in) {
      Object.assign(overview, { profile: {}, assets: {} })
      useFilesStore().reset()
    }
  }

  async function loadOverview() {
    if (!state.logged_in) return
    const data = unwrapData(await bridge.invoke('get_overview'))
    const profile = data.profile || {}
    const assets = data.assets || data.quota || {}
    overview.profile = profile.data || profile.user || profile
    overview.assets = assets.data || assets
  }

  async function refreshState() {
    applyState(await bridge.invoke('get_state'))
  }

  async function checkAccess() {
    if (isTauri) {
      accessChecked.value = true
      accessRequired.value = false
      accessGranted.value = true
      return true
    }
    try {
      const data = unwrapData(await bridge.invoke('get_access_status'))
      accessChecked.value = true
      accessRequired.value = data.required === true
      accessGranted.value = data.required !== true || data.authenticated === true || data.unlocked === true
      accessUsername.value = String(data.username || 'admin')
      return accessGranted.value
    }
    catch (reason) {
      accessChecked.value = true
      accessRequired.value = true
      accessGranted.value = false
      appendLog('error', `无法验证 Web 访问权限：${String(reason)}`)
      return false
    }
  }

  async function unlockAccess(code: string) {
    const data = unwrapData(await bridge.invoke('unlock_access', { code }))
    accessRequired.value = data.required !== false
    accessGranted.value = data.authenticated === true || data.unlocked === true
    if (!accessGranted.value) throw new Error(data.message || '访问码错误')
    await connect()
  }

  async function connect() {
    if (!accessGranted.value) return
    if (!unsubscribe) {
      const transfers = useTransfersStore()
      unsubscribe = await bridge.subscribe((payload: any) => {
        if (payload?.type === 'state') applyState(payload.state)
        if (payload?.type === 'status') appendLog(payload.level || 'info', String(payload.message || ''))
        transfers.handleSyncEvent(payload)
      })
    }
    await refreshState()
    if (state.logged_in) {
      forceAuth.value = false
      await Promise.allSettled([loadOverview(), useFilesStore().loadFiles()])
    }
  }

  function requestRelogin() {
    forceAuth.value = true
  }

  async function initialize() {
    if (initializing.value) return
    initializing.value = true
    bootLoading.value = true
    try {
      const allowed = await checkAccess()
      if (allowed) await connect()
    }
    finally {
      bootLoading.value = false
      initializing.value = false
    }
  }

  function dispose() {
    unsubscribe?.()
    unsubscribe = null
  }

  return {
    state,
    overview,
    logs,
    bootLoading,
    accessChecked,
    accessRequired,
    accessGranted,
    accessUsername,
    forceAuth,
    userName,
    userAvatar,
    usedSpace,
    totalSpace,
    remainingSpace,
    remainingSpaceLabel,
    quotaPercent,
    appendLog,
    applyState,
    loadOverview,
    refreshState,
    checkAccess,
    unlockAccess,
    connect,
    requestRelogin,
    initialize,
    dispose,
  }
})
