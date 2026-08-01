<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { ReloadOutlined, UserOutlined } from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText, formatSize, formatTime, pick, unwrapData } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'
import { classifyVipStatus } from '../../vipStatus.js'

const session = useSessionStore()
const { userName, userAvatar, usedSpace, totalSpace, quotaPercent } = storeToRefs(session)
const assetsSnapshot = ref<Record<string, any>>({})
const globalConfig = ref<Record<string, any>>({})
const loading = ref(false)
const loadError = ref('')

const assets = computed(() => Object.keys(assetsSnapshot.value).length
  ? assetsSnapshot.value
  : session.overview.assets)
const phoneNumber = computed(() => pick(
  session.overview.profile,
  ['phone_number', 'phoneNumber', 'phone', 'mobile'],
  '未绑定手机号',
))
const accountId = computed(() => pick(
  session.overview.profile,
  ['sub', 'userId', 'user_id', 'id'],
  '—',
))
const vipStatus = computed(() => Number(pick(assets.value, ['vipStatus', 'vip_status'], '1')))
const svipStatus = computed(() => Number(pick(assets.value, ['svipStatus', 'svip_status'], '1')))
const vipExpireTime = computed(() => Number(pick(
  assets.value,
  ['vipExpireTime', 'vip_expire_time', 'vipEndTime', 'vip_end_time'],
  '0',
)))
const vipState = computed(() => classifyVipStatus(vipStatus.value))
const svipState = computed(() => classifyVipStatus(svipStatus.value))
const vipExpireLabel = computed(() => {
  if (vipStatus.value === 1) return '未开通'
  if (!vipExpireTime.value) return vipStatus.value === 3 ? '已过期' : '未返回到期时间'
  const label = formatTime(vipExpireTime.value)
  return vipStatus.value === 3 ? `${label}（已过期）` : label
})
const trafficCards = computed(() => {
  const highSpeed = assets.value.highSpeedTraffic || assets.value.high_speed_traffic || {}
  const records = [
    {
      key: 'high-speed',
      label: '高速流量',
      total: Number(pick(highSpeed, ['total'], '0')) || 0,
      remained: Number(pick(highSpeed, ['remained', 'remaining'], '0')) || 0,
    },
    {
      key: 'direct-link',
      label: '直链流量',
      total: Number(pick(assets.value, ['totalDirectLinkTraffic', 'total_direct_link_traffic'], '0')) || 0,
      remained: Number(pick(assets.value, ['freeDirectLinkTraffic', 'free_direct_link_traffic'], '0')) || 0,
    },
    {
      key: 'share-guest',
      label: '免登录分享流量',
      total: Number(pick(assets.value, ['totalShareGuestTraffic', 'total_share_guest_traffic'], '0')) || 0,
      remained: Number(pick(assets.value, ['freeShareGuestTraffic', 'free_share_guest_traffic'], '0')) || 0,
    },
  ]
  return records.filter((record) => record.total > 0 || record.remained > 0)
})
const vipRights = computed(() => {
  const config = unwrapData(globalConfig.value)
  const list = config?.common?.vipRights || config?.common?.vip_rights || config?.vipRights || []
  return Array.isArray(list) ? list : []
})
const rightsColumns = [
  { title: '权益', dataIndex: 'name', key: 'name', width: 180 },
  { title: '当前账号', key: 'current', width: 190 },
  { title: '普通账号', key: 'regular', width: 190 },
  { title: 'VIP', key: 'vip', width: 220 },
]

function rightValue(value: unknown) {
  if (value === 'iconCheck' || value === true) return '支持'
  if (value === '-' || value === false || value == null || value === '') return '—'
  return String(value)
}

function currentRightValue(record: Record<string, any>) {
  return rightValue(vipStatus.value === 2 ? record.vip : record.regular)
}

function rightRowKey(record: Record<string, any>) {
  return String(record.name || '')
}

async function refreshAccountData() {
  if (!session.state.logged_in || loading.value) return
  loading.value = true
  loadError.value = ''
  const [assetResult, configResult] = await Promise.allSettled([
    bridge.invoke('get_assets'),
    bridge.invoke('get_global_config'),
  ])
  if (assetResult.status === 'fulfilled') {
    const next = unwrapData(assetResult.value)
    assetsSnapshot.value = next && typeof next === 'object' ? next : {}
    session.overview.assets = assetsSnapshot.value
  }
  if (configResult.status === 'fulfilled') {
    const next = unwrapData(configResult.value)
    globalConfig.value = next && typeof next === 'object' ? next : {}
  }
  const errors = [assetResult, configResult]
    .filter((result): result is PromiseRejectedResult => result.status === 'rejected')
    .map((result) => errorText(result.reason))
  loadError.value = errors.join('；')
  loading.value = false
}

onMounted(refreshAccountData)
</script>

<template>
  <section class="setting-section">
    <div class="account-line">
      <a-avatar :size="52" :src="userAvatar || undefined">
        <template #icon><UserOutlined /></template>
      </a-avatar>
      <div class="account-summary">
        <strong>{{ userName }}</strong>
        <span>{{ phoneNumber }}</span>
      </div>
      <a-space>
        <a-button :loading="loading" aria-label="刷新账号资产" @click="refreshAccountData"><ReloadOutlined />刷新资产</a-button>
        <a-button @click="session.requestRelogin">重新登录</a-button>
      </a-space>
    </div>

    <a-alert v-if="loadError" class="account-alert" type="warning" show-icon :message="`部分账号数据刷新失败：${loadError}`">
      <template #action><a-button size="small" @click="refreshAccountData">重试</a-button></template>
    </a-alert>

    <a-divider />
    <a-descriptions :column="1" size="small" bordered class="account-details">
      <a-descriptions-item label="账号 ID">{{ accountId }}</a-descriptions-item>
      <a-descriptions-item label="手机号">{{ phoneNumber }}</a-descriptions-item>
      <a-descriptions-item label="VIP"><a-tag :color="vipState.color">{{ vipState.label }}</a-tag></a-descriptions-item>
      <a-descriptions-item label="SVIP"><a-tag :color="svipState.color">{{ svipState.label }}</a-tag></a-descriptions-item>
      <a-descriptions-item label="VIP 到期">{{ vipExpireLabel }}</a-descriptions-item>
    </a-descriptions>

    <a-divider />
    <div class="setting-row">
      <div>
        <strong>云盘空间</strong>
        <span>{{ formatSize(usedSpace) }} / {{ formatSize(totalSpace) }}</span>
      </div>
      <a-progress :percent="quotaPercent" :show-info="false" class="quota-progress" />
    </div>

    <div v-if="trafficCards.length" class="asset-grid">
      <div v-for="record in trafficCards" :key="record.key" class="asset-card">
        <span>{{ record.label }}</span>
        <strong>{{ formatSize(record.remained) }}</strong>
        <small>剩余 / 共 {{ formatSize(record.total) }}</small>
      </div>
    </div>

    <template v-if="vipRights.length">
      <a-divider />
      <div class="rights-heading">
        <div><strong>当前权益规则</strong><span>来自光鸭全局配置，实际可用量仍以服务端校验为准</span></div>
      </div>
      <a-table :columns="rightsColumns" :data-source="vipRights" :row-key="rightRowKey" :pagination="false" :scroll="{ x: 780 }" size="small" class="rights-table">
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'current'"><strong>{{ currentRightValue(record) }}</strong></template>
          <template v-else-if="column.key === 'regular'">{{ rightValue(record.regular) }}</template>
          <template v-else-if="column.key === 'vip'">{{ rightValue(record.vip) }}</template>
        </template>
      </a-table>
    </template>
  </section>
</template>

<style scoped>
.setting-section { max-width: 980px; padding: 8px 18px 36px 24px; }
.account-line { display: flex; align-items: center; gap: 14px; }
.account-summary { min-width: 0; flex: 1; }
.account-summary strong, .account-summary span, .setting-row strong, .setting-row span { display: block; }
.account-summary strong { font-size: 16px; }
.account-summary span, .setting-row span, .rights-heading span { margin-top: 4px; color: var(--text-3, #98a2b3); font-size: 12px; }
.account-alert { margin-top: 16px; }
.account-details { max-width: 620px; }
.setting-row { display: flex; align-items: center; justify-content: space-between; gap: 24px; min-height: 54px; }
.quota-progress { width: min(320px, 42vw); }
.asset-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); gap: 10px; margin-top: 14px; }
.asset-card { padding: 14px 16px; border: 1px solid var(--border, #e5e7eb); border-radius: 10px; background: var(--surface, #fff); }
.asset-card span, .asset-card strong, .asset-card small { display: block; }
.asset-card span, .asset-card small { color: var(--text-3, #98a2b3); font-size: 12px; }
.asset-card strong { margin: 6px 0 2px; font-size: 17px; }
.rights-heading { display: flex; align-items: center; justify-content: space-between; margin-bottom: 12px; }
.rights-heading strong, .rights-heading span { display: block; }
.rights-table { max-width: 900px; }
@media (max-width: 760px) {
  .account-line { align-items: flex-start; flex-wrap: wrap; }
  .account-line > .ant-space { width: 100%; }
  .setting-row { align-items: flex-start; flex-direction: column; }
  .quota-progress { width: 100%; }
}
</style>
