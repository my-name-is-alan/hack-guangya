<script setup lang="ts">
import { shallowRef, watch } from 'vue'
import { useRoute } from 'vue-router'
import {
  CloudServerOutlined,
  DatabaseOutlined,
  FolderOpenOutlined,
  GlobalOutlined,
  KeyOutlined,
  LockOutlined,
  ReloadOutlined,
  SettingOutlined,
  SwapOutlined,
  UserOutlined,
} from '@antdv-next/icons'
import { isTauri } from '../bridge.js'
import AccessCodeSettingsPanel from '../components/settings/AccessCodeSettingsPanel.vue'
import AccountSettingsPanel from '../components/settings/AccountSettingsPanel.vue'
import CacheSettingsPanel from '../components/settings/CacheSettingsPanel.vue'
import DeveloperSettingsPanel from '../components/settings/DeveloperSettingsPanel.vue'
import HdhiveSettingsPanel from '../components/settings/HdhiveSettingsPanel.vue'
import MountSettingsPanel from '../components/settings/MountSettingsPanel.vue'
import PreferenceSettingsPanel from '../components/settings/PreferenceSettingsPanel.vue'
import TransferSettingsPanel from '../components/settings/TransferSettingsPanel.vue'
import UpdateSettingsPanel from '../components/settings/UpdateSettingsPanel.vue'
import OrganizerSettingsPanel from '../components/settings/OrganizerSettingsPanel.vue'
import NetworkSettingsPanel from '../components/settings/NetworkSettingsPanel.vue'

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
.settings-view { min-height: calc(100vh - 96px); min-width: 0; }
.settings-tabs { display: flex; min-width: 0; }
.settings-tabs :deep(.ant-tabs-nav) { width: 154px; min-width: 154px; flex: 0 0 154px; }
.settings-tabs :deep(.ant-tabs-content-holder) { min-width: 0; flex: 1 1 auto; }
.settings-tabs :deep(.ant-tabs-content) { min-width: 0; }
.setting-tab { display: inline-flex; align-items: center; gap: 9px; }
</style>
