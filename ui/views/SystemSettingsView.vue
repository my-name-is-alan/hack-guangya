<script setup lang="ts">
import { defineAsyncComponent, shallowRef, watch } from 'vue'
import { useRoute } from 'vue-router'
import {
  CloudServerOutlined,
  DatabaseOutlined,
  DownloadOutlined,
  FolderOpenOutlined,
  GlobalOutlined,
  KeyOutlined,
  LockOutlined,
  ReloadOutlined,
  SendOutlined,
  SettingOutlined,
  SwapOutlined,
  UserOutlined,
} from '@antdv-next/icons'
import { isTauri } from '../bridge.js'

const AccessCodeSettingsPanel = defineAsyncComponent(() => import('../components/settings/AccessCodeSettingsPanel.vue'))
const AccountSettingsPanel = defineAsyncComponent(() => import('../components/settings/AccountSettingsPanel.vue'))
const CacheSettingsPanel = defineAsyncComponent(() => import('../components/settings/CacheSettingsPanel.vue'))
const DeveloperSettingsPanel = defineAsyncComponent(() => import('../components/settings/DeveloperSettingsPanel.vue'))
const HdhiveSettingsPanel = defineAsyncComponent(() => import('../components/settings/HdhiveSettingsPanel.vue'))
const MountSettingsPanel = defineAsyncComponent(() => import('../components/settings/MountSettingsPanel.vue'))
const OfflineSettingsPanel = defineAsyncComponent(() => import('../components/settings/OfflineSettingsPanel.vue'))
const PreferenceSettingsPanel = defineAsyncComponent(() => import('../components/settings/PreferenceSettingsPanel.vue'))
const TransferSettingsPanel = defineAsyncComponent(() => import('../components/settings/TransferSettingsPanel.vue'))
const UpdateSettingsPanel = defineAsyncComponent(() => import('../components/settings/UpdateSettingsPanel.vue'))
const OrganizerSettingsPanel = defineAsyncComponent(() => import('../components/settings/OrganizerSettingsPanel.vue'))
const NetworkSettingsPanel = defineAsyncComponent(() => import('../components/settings/NetworkSettingsPanel.vue'))
const TelegramSettingsPanel = defineAsyncComponent(() => import('../components/settings/TelegramSettingsPanel.vue'))

const activeTab = shallowRef('account')
const route = useRoute()
watch(() => route.query.tab, (value) => { if (typeof value === 'string' && value) activeTab.value = value }, { immediate: true })
</script>

<template>
  <div class="settings-view">
    <a-tabs v-model:active-key="activeTab" tab-position="left" class="settings-tabs">
      <a-tab-pane key="account">
        <template #tab><span class="setting-tab"><UserOutlined />账号</span></template>
        <AccountSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="developerTransfer">
        <template #tab><span class="setting-tab"><KeyOutlined />多号秒传</span></template>
        <DeveloperSettingsPanel />
      </a-tab-pane>

      <a-tab-pane v-if="!isTauri" key="access">
        <template #tab><span class="setting-tab"><LockOutlined />访问码</span></template>
        <AccessCodeSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="preference">
        <template #tab><span class="setting-tab"><SettingOutlined />偏好</span></template>
        <PreferenceSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="transfer">
        <template #tab><span class="setting-tab"><SwapOutlined />传输</span></template>
        <TransferSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="offline">
        <template #tab><span class="setting-tab"><DownloadOutlined />离线下载</span></template>
        <OfflineSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="network">
        <template #tab><span class="setting-tab"><GlobalOutlined />网络偏好</span></template>
        <NetworkSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="organizer">
        <template #tab><span class="setting-tab"><FolderOpenOutlined />整理与刮削</span></template>
        <OrganizerSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="mount">
        <template #tab><span class="setting-tab"><FolderOpenOutlined />挂载</span></template>
        <MountSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="hdhive">
        <template #tab><span class="setting-tab"><CloudServerOutlined />HDHive</span></template>
        <HdhiveSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="telegram">
        <template #tab><span class="setting-tab"><SendOutlined />Telegram</span></template>
        <TelegramSettingsPanel />
      </a-tab-pane>

      <a-tab-pane key="cache">
        <template #tab><span class="setting-tab"><DatabaseOutlined />缓存</span></template>
        <CacheSettingsPanel />
      </a-tab-pane>

      <a-tab-pane v-if="isTauri" key="update">
        <template #tab><span class="setting-tab"><ReloadOutlined />更新</span></template>
        <UpdateSettingsPanel />
      </a-tab-pane>
    </a-tabs>
  </div>
</template>

<style scoped>
.settings-view { width: 100%; min-width: 0; max-width: 100%; min-height: calc(100vh - 96px); overflow: hidden; }
.settings-tabs { display: flex; width: 100%; min-width: 0; max-width: 100%; }
.settings-tabs > :deep(.ant-tabs-nav) { width: 154px; min-width: 154px; flex: 0 0 154px; }
.settings-tabs > :deep(.ant-tabs-body-holder) { min-width: 0; max-width: 100%; flex: 1 1 auto; overflow: hidden; }
.settings-tabs > :deep(.ant-tabs-body-holder > .ant-tabs-body),
.settings-tabs > :deep(.ant-tabs-body-holder > .ant-tabs-body > .ant-tabs-content) { min-width: 0; max-width: 100%; }
.setting-tab { display: inline-flex; align-items: center; gap: 9px; }
</style>
