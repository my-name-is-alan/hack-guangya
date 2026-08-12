<script setup lang="ts">
import { computed, ref } from 'vue'
import { storeToRefs } from 'pinia'
import { ReloadOutlined, UserOutlined } from '@antdv-next/icons'
import { errorText, formatSize, pick } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'

const session = useSessionStore()
const { userName, userAvatar, usedSpace, totalSpace, quotaPercent } = storeToRefs(session)
const loading = ref(false)
const loadError = ref('')

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

async function refreshAccountData() {
  if (!session.state.logged_in || loading.value) return
  loading.value = true
  loadError.value = ''
  try {
    await session.loadOverview()
  } catch (reason) {
    loadError.value = errorText(reason)
  } finally {
    loading.value = false
  }
}

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
        <a-button :loading="loading" aria-label="刷新账号信息" @click="refreshAccountData"><ReloadOutlined />刷新</a-button>
        <a-button @click="session.requestRelogin">重新登录</a-button>
      </a-space>
    </div>

    <a-alert v-if="loadError" class="account-alert" type="warning" show-icon :message="`账号信息刷新失败：${loadError}`">
      <template #action><a-button size="small" @click="refreshAccountData">重试</a-button></template>
    </a-alert>

    <a-divider />
    <a-descriptions :column="1" size="small" bordered class="account-details">
      <a-descriptions-item label="账号 ID">{{ accountId }}</a-descriptions-item>
      <a-descriptions-item label="手机号">{{ phoneNumber }}</a-descriptions-item>
    </a-descriptions>

    <a-divider />
    <div class="setting-row">
      <div>
        <strong>云盘空间</strong>
        <span>{{ formatSize(usedSpace) }} / {{ formatSize(totalSpace) }}</span>
      </div>
      <a-progress :percent="quotaPercent" :show-info="false" class="quota-progress" />
    </div>

  </section>
</template>

<style scoped>
/* 骨架样式已提升为全局类；账号详情较宽，仅覆盖最大宽度。 */
.setting-section { max-width: 980px; }
.account-line { display: flex; align-items: center; gap: 14px; }
.account-summary { min-width: 0; flex: 1; }
.account-summary strong, .account-summary span { display: block; }
.account-summary strong { font-size: var(--fs-lg, 15px); }
.account-summary span { margin-top: 4px; color: var(--text-3, #737373); font-size: var(--fs-sm, 12px); }
.account-alert { margin-top: 16px; }
.account-details { max-width: 620px; }
.quota-progress { width: min(320px, 42vw); }
@media (max-width: 760px) {
  .account-line { align-items: flex-start; flex-wrap: wrap; }
  .account-line > .ant-space { width: 100%; }
  .setting-row { align-items: flex-start; flex-direction: column; }
  .quota-progress { width: 100%; }
}
</style>
