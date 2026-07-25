<script setup lang="ts">
import { computed } from 'vue'
import { storeToRefs } from 'pinia'
import { ReloadOutlined, UserOutlined } from '@antdv-next/icons'
import { formatSize, pick } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'

const session = useSessionStore()
const { userName, userAvatar, usedSpace, totalSpace, quotaPercent } = storeToRefs(session)
const phoneNumber = computed(() => pick(
  session.overview.profile,
  ['phone_number', 'phoneNumber', 'phone', 'mobile'],
  '未绑定手机号',
))
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
      <a-button @click="session.requestRelogin">
        <ReloadOutlined />重新登录
      </a-button>
    </div>
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
.setting-section { max-width: 760px; padding: 8px 18px 36px 24px; }
.account-line { display: flex; align-items: center; gap: 14px; }
.account-summary { min-width: 0; flex: 1; }
.account-summary strong, .account-summary span, .setting-row strong, .setting-row span { display: block; }
.account-summary strong { font-size: 16px; }
.account-summary span, .setting-row span { margin-top: 4px; color: var(--text-3, #98a2b3); font-size: 12px; }
.setting-row { display: flex; align-items: center; justify-content: space-between; gap: 24px; min-height: 54px; }
.quota-progress { width: 240px; }
</style>
