<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { message } from 'antdv-next'
import { ReloadOutlined } from '@antdv-next/icons'
import { useUpdaterStore } from '../../stores/updater'

const updater = useUpdaterStore()
const {
  autoCheckEnabled,
  availableUpdate,
  checking,
  currentVersion,
  error,
  installing,
  lastCheckedAt,
  progressPercent,
} = storeToRefs(updater)

async function checkNow() {
  try {
    const update = await updater.checkForUpdates()
    if (update) message.success(`发现新版本 v${update.version}`)
    else message.success('当前已经是最新版本')
  } catch (reason) {
    message.error(error.value || String(reason))
  }
}

async function installNow() {
  try {
    await updater.installUpdate()
  } catch (reason) {
    message.error(error.value || String(reason))
  }
}
</script>

<template>
  <section class="setting-section">
    <div class="setting-row">
      <div>
        <strong>启动时自动检查</strong>
        <span>仅检查 GitHub 最新正式版；确认后才会下载和安装。</span>
      </div>
      <a-switch
        :checked="autoCheckEnabled"
        aria-label="启动时自动检查更新"
        @change="updater.setAutoCheckEnabled"
      />
    </div>

    <a-descriptions bordered :column="1" size="small" class="version-details">
      <a-descriptions-item label="当前版本">v{{ currentVersion || '读取中…' }}</a-descriptions-item>
      <a-descriptions-item label="更新状态">
        <a-tag v-if="availableUpdate" color="blue">可更新至 v{{ availableUpdate.version }}</a-tag>
        <span v-else>{{ checking ? '正在检查…' : '未发现待安装更新' }}</span>
      </a-descriptions-item>
      <a-descriptions-item label="上次检查">
        {{ lastCheckedAt ? lastCheckedAt.toLocaleString() : '本次启动尚未检查' }}
      </a-descriptions-item>
    </a-descriptions>

    <a-alert v-if="error" class="update-alert" type="error" :message="error" show-icon />
    <a-progress v-if="installing" class="update-progress" :percent="progressPercent" />

    <div class="actions">
      <a-button :loading="checking" :disabled="installing" @click="checkNow">
        <template #icon><ReloadOutlined /></template>
        立即检查
      </a-button>
      <a-button v-if="availableUpdate" type="primary" :loading="installing" @click="installNow">
        下载并安装 v{{ availableUpdate.version }}
      </a-button>
    </div>
  </section>
</template>

<style scoped>
/* 骨架样式（setting-section / setting-row）已提升为全局类。 */
.version-details { max-width: 580px; }
.update-alert, .update-progress { max-width: 580px; margin-top: 16px; }
.actions { display: flex; gap: 10px; margin-top: 18px; }
</style>
