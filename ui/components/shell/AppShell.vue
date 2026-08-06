<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, shallowRef } from 'vue'
import { storeToRefs } from 'pinia'
import { RouterView, useRoute, useRouter } from 'vue-router'
import {
  CloudOutlined,
  CloudUploadOutlined,
  DownloadOutlined,
  FolderOpenOutlined,
  SearchOutlined,
  SettingOutlined,
  ShareAltOutlined,
  SyncOutlined,
  ToolOutlined,
  UserOutlined,
} from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText } from '../../formatters.js'
import { message } from 'antdv-next'
import { formatSize } from '../../formatters.js'
import { formatUploadSpeed } from '../../uploadProgress.js'
import { useSessionStore } from '../../stores/session'
import { useTransfersStore } from '../../stores/transfers'
import GlobalSearch from '../search/GlobalSearch.vue'

const route = useRoute()
const router = useRouter()
const session = useSessionStore()
const transfers = useTransfersStore()
const { userName, userAvatar, usedSpace, totalSpace, quotaPercent } = storeToRefs(session)
const { activeUploads, activeDownloads, uploadSpeed, overallPercent } = storeToRefs(transfers)
const searchOpen = shallowRef(false)

const navigation = [
  { name: 'backup', label: '备份', icon: CloudUploadOutlined },
  { name: 'organizer', label: '整理', icon: FolderOpenOutlined },
  { name: 'transfers', label: '传输', icon: SyncOutlined },
  { name: 'offline', label: '离线', icon: DownloadOutlined },
  { name: 'shares', label: '分享', icon: ShareAltOutlined },
]

const activeName = computed(() => String(route.name || 'files'))
const activeTransferCount = computed(() => activeUploads.value.length + activeDownloads.value.length)
const quotaLabel = computed(() => totalSpace.value
  ? `${formatSize(usedSpace.value)} / ${formatSize(totalSpace.value)}`
  : '— / —')
const accountMenu = {
  items: [
    { key: 'settings', label: '账号与设置' },
    { key: 'relogin', label: '重新登录' },
  ],
  onClick: ({ key }: { key: string }) => {
    if (key === 'settings') void router.push({ name: 'settings' })
    if (key === 'relogin') void bridgeRelogin()
  },
}

const networkMenu = {
  items: [
    { key: 'tmdb', label: '测试 TMDB' },
    { key: 'tg', label: '测试 Telegram' },
    { key: 'github', label: '测试 GitHub' },
    { type: 'divider' },
    { key: 'settings', label: '打开网络偏好' },
  ],
  onClick: async ({ key }: { key: string }) => {
    if (key === 'settings') { void router.push({ name: 'settings', query: { tab: 'network' } }); return }
    try {
      const result = await bridge.invoke('test_network', { target: key })
      if (result?.reachable) message.success(`${key.toUpperCase()}：${result.message || '网络可达'}（${result.latency_ms || 0} ms）`)
      else message.error(`${key.toUpperCase()}：${result?.message || '网络不可达'}`)
    } catch (error) { message.error(errorText(error)) }
  },
}

async function bridgeRelogin() {
  session.requestRelogin()
}

function handleGlobalKeydown(event: KeyboardEvent) {
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'f') {
    event.preventDefault()
    searchOpen.value = true
  }
}

function openTransfers() {
  void router.push({ name: 'transfers', query: { tab: activeUploads.value.length ? 'upload' : 'download' } })
}

onMounted(() => window.addEventListener('keydown', handleGlobalKeydown))
onBeforeUnmount(() => window.removeEventListener('keydown', handleGlobalKeydown))
</script>

<template>
  <div class="app-frame">
    <aside class="nav-rail" aria-label="主导航">
      <RouterLink class="home-mark" :class="{ active: activeName === 'files' }" :to="{ name: 'files' }" aria-label="云盘文件首页">
        <CloudOutlined />
      </RouterLink>

      <nav class="rail-links">
        <RouterLink v-for="item in navigation" :key="item.name" :to="{ name: item.name }" class="rail-link" :class="{ active: activeName === item.name }">
          <component :is="item.icon" />
          <span>{{ item.label }}</span>
        </RouterLink>
      </nav>

      <RouterLink :to="{ name: 'settings' }" class="rail-link rail-settings" :class="{ active: activeName === 'settings' }">
        <SettingOutlined />
        <span>设置</span>
      </RouterLink>
    </aside>

    <section class="workspace">
      <header class="app-topbar">
        <button type="button" class="search-trigger" @click="searchOpen = true">
          <SearchOutlined />
          <span>搜索整个云盘</span>
          <kbd>Ctrl F</kbd>
        </button>

        <div class="top-actions">
          <button v-if="activeTransferCount" type="button" class="transfer-summary" @click="openTransfers">
            <span class="transfer-label">{{ activeTransferCount }} 个任务</span>
            <span class="transfer-bar"><i :style="{ width: `${overallPercent}%` }" /></span>
            <small>{{ uploadSpeed ? formatUploadSpeed(uploadSpeed) : `${overallPercent}%` }}</small>
          </button>

          <a-dropdown :trigger="['click']" :menu="networkMenu">
            <button type="button" class="network-tools-trigger" aria-label="网络工具"><ToolOutlined /></button>
          </a-dropdown>

          <a-dropdown :trigger="['click']" :menu="accountMenu">
            <button type="button" class="account-trigger">
              <a-avatar class="account-avatar" :size="32" :src="userAvatar || undefined"><template #icon><UserOutlined /></template></a-avatar>
              <span class="account-copy">
                <span class="account-meta"><strong>{{ userName }}</strong><small>{{ quotaLabel }}</small></span>
                <span
                  class="quota-bar"
                  role="progressbar"
                  aria-label="存储空间使用率"
                  aria-valuemin="0"
                  aria-valuemax="100"
                  :aria-valuenow="quotaPercent"
                ><i :style="{ width: `${quotaPercent}%` }" /></span>
              </span>
            </button>
          </a-dropdown>
        </div>
      </header>

      <main class="route-content">
        <RouterView v-slot="{ Component }">
          <Transition name="route-fade" mode="out-in">
            <component :is="Component" />
          </Transition>
        </RouterView>
      </main>
    </section>

    <GlobalSearch v-model:open="searchOpen" />
  </div>
</template>

<style scoped>
.app-frame { display: grid; min-width: 0; min-height: 100vh; grid-template-columns:74px minmax(0,1fr); color: var(--text-1, #20242c); background: var(--app-bg, #f7f7f8); }
.nav-rail { display: flex; position: sticky; z-index: 20; top: 0; height: 100vh; flex-direction: column; border-right: 1px solid var(--line, #e4e7ec); background: var(--sidebar-bg, #fff); }
.home-mark { display: grid; height: 62px; place-items: center; border-bottom: 1px solid var(--line, #e4e7ec); color: var(--text-2, #667085); font-size: 22px; }
.home-mark.active { color: var(--primary-strong, #171717); }
.rail-links { display: flex; flex: 1; flex-direction: column; gap: 4px; padding: 10px 7px; }
.rail-link { display: flex; height: 50px; align-items: center; flex-direction: column; justify-content: center; gap: 3px; border-radius: 10px; color: var(--text-2, #667085); font-size: 11px; font-weight: 600; }
.rail-link :deep(.anticon) { font-size: 18px; }
.rail-link:hover { color: var(--text-1, #20242c); background: var(--surface-hover, #f3f4f6); }
.rail-link.active { color: var(--primary-strong, #171717); background: var(--primary-soft, #f5f5f5); }
.rail-settings { margin: 0 7px 10px; }
.workspace { display: grid; min-width: 0; height: 100vh; grid-template-rows:62px minmax(0,1fr); }
.app-topbar { display: flex; z-index: 15; align-items: center; justify-content: space-between; gap: 20px; padding: 0 18px; border-bottom: 1px solid var(--line, #e4e7ec); background: color-mix(in srgb, var(--surface, #fff) 94%, transparent); backdrop-filter: blur(12px); }
.search-trigger { display: flex; width: min(520px, 48vw); height: 36px; align-items: center; gap: 9px; padding: 0 11px; border: 1px solid var(--line, #d9dde5); border-radius: 9px; color: var(--text-3, #98a2b3); background: var(--surface-muted, #f8f9fa); text-align: left; cursor: text; }
.search-trigger span { flex: 1; }
.search-trigger kbd { padding: 2px 6px; border: 1px solid var(--line, #d9dde5); border-radius: 4px; background: var(--surface, #fff); font-size: 10px; }
.top-actions { display: flex; align-items: center; gap: 14px; }
.network-tools-trigger { display: grid; width: 30px; height: 30px; place-items: center; border: 0; border-radius: 8px; color: var(--text-2, #667085); background: transparent; font-size: 17px; cursor: pointer; }
.network-tools-trigger:hover { color: var(--text-1, #20242c); background: var(--surface-hover, #f3f4f6); }
.transfer-summary { display: grid; min-width: 136px; grid-template-columns:1fr auto; align-items: center; gap: 3px 8px; padding: 3px 0; border: 0; color: var(--text-2, #667085); background: transparent; cursor: pointer; }
.transfer-label { font-size: 11px; text-align: left; }
.transfer-summary small { grid-column:2; grid-row:1 / span 2; color: var(--text-3, #98a2b3); font-size: 10px; }
.transfer-bar { display: block; height: 4px; overflow: hidden; border-radius: 99px; background: var(--line, #e4e7ec); }
.transfer-bar i { display: block; height: 100%; border-radius: inherit; background: var(--primary, #262626); transition: width .2s ease; }
.account-trigger { display: flex; align-items: center; gap: 8px; padding: 3px 0; border: 0; background: transparent; text-align: left; cursor: pointer; }
.account-avatar { width: 32px !important; min-width: 32px; height: 32px !important; flex: 0 0 32px; overflow: hidden; border-radius: 50%; }
.account-avatar :deep(img) { width: 100%; height: 100%; object-fit: cover; }
.account-copy { display: grid; width: 132px; min-width: 132px; gap: 4px; }
.account-meta { display: flex; min-width: 0; align-items: baseline; justify-content: space-between; gap: 8px; }
.account-trigger strong, .account-trigger small { display: block; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.account-trigger strong { font-size: 12px; }
.account-trigger small { flex: 0 1 auto; color: var(--text-3, #737373); font-size: 9px; font-variant-numeric: tabular-nums; }
.quota-bar { display: block; width: 100%; height: 4px; overflow: hidden; border-radius: 999px; background: var(--line-soft, #f5f5f5); }
.quota-bar i { display: block; height: 100%; border-radius: inherit; background: var(--primary, #262626); transition: width .2s ease; }
.route-content { min-height: 0; overflow: auto; padding: 14px 18px 18px; }
.route-fade-enter-active, .route-fade-leave-active { transition: opacity .12s ease, transform .12s ease; }
.route-fade-enter-from, .route-fade-leave-to { opacity: 0; transform: translateY(3px); }
@media (prefers-reduced-motion: reduce) { .route-fade-enter-active, .route-fade-leave-active { transition: none; } }
@media (max-width: 720px) {
  .app-frame { grid-template-columns:64px minmax(0,1fr); }
  .app-topbar { gap: 10px; padding: 0 10px; }
  .search-trigger { width: auto; min-width: 0; flex: 1; }
  .search-trigger kbd, .account-copy { display: none; }
  .top-actions { gap: 8px; }
  .transfer-summary { min-width: 92px; }
  .route-content { padding-inline: 10px; }
}
</style>
