<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { message } from 'antdv-next'
import { DownloadOutlined } from '@antdv-next/icons'
import { formatSize } from '../../formatters.js'
import { useUpdaterStore } from '../../stores/updater'

const updater = useUpdaterStore()
const {
  availableUpdate,
  currentVersion,
  downloadedBytes,
  error,
  installing,
  progressPercent,
  promptOpen,
  totalBytes,
} = storeToRefs(updater)

async function installNow() {
  try {
    await updater.installUpdate()
  } catch (reason) {
    message.error(error.value || String(reason))
  }
}
</script>

<template>
  <a-modal
    :open="promptOpen"
    :closable="!installing"
    :keyboard="!installing"
    :mask-closable="false"
    :footer="null"
    width="520px"
    title="发现新版本"
    @cancel="updater.dismissPrompt"
  >
    <div v-if="availableUpdate" class="update-prompt">
      <div class="version-line">
        <span class="version-badge">v{{ availableUpdate.version }}</span>
        <span>当前版本 v{{ currentVersion || availableUpdate.current_version }}</span>
      </div>
      <div v-if="availableUpdate.notes" class="release-notes">{{ availableUpdate.notes }}</div>
      <div v-else class="release-notes release-notes--empty">本次更新未提供额外说明。</div>
      <div v-if="installing" class="download-progress">
        <a-progress :percent="progressPercent" :show-info="totalBytes > 0" />
        <span v-if="totalBytes > 0">{{ formatSize(downloadedBytes) }} / {{ formatSize(totalBytes) }}</span>
        <span v-else>正在准备更新包…</span>
      </div>
      <a-alert v-if="error" type="error" :message="error" show-icon />
      <div class="update-actions">
        <a-button :disabled="installing" @click="updater.dismissPrompt">稍后提醒</a-button>
        <a-button type="primary" :loading="installing" @click="installNow">
          <template #icon><DownloadOutlined /></template>
          {{ installing ? '正在下载并安装' : '立即更新' }}
        </a-button>
      </div>
      <p class="restart-tip">下载完成后安装程序会自动接管，Windows 客户端将退出并完成更新。</p>
    </div>
  </a-modal>
</template>

<style scoped>
.update-prompt { display: grid; gap: 16px; }
.version-line { display: flex; align-items: center; gap: 12px; color: var(--text-3, #737373); font-size: 13px; }
.version-badge { padding: 5px 10px; border-radius: 999px; color: #fff; background: #1677ff; font-weight: 700; }
.release-notes { max-height: 220px; overflow: auto; padding: 14px 16px; border-radius: 10px; color: var(--text-2, #525252); background: var(--fill-2, #f7f8fa); line-height: 1.7; white-space: pre-wrap; }
.release-notes--empty { color: var(--text-3, #737373); }
.download-progress { display: grid; gap: 5px; color: var(--text-3, #737373); font-size: 12px; }
.update-actions { display: flex; justify-content: flex-end; gap: 10px; }
.restart-tip { margin: -7px 0 0; color: var(--text-3, #737373); font-size: 12px; text-align: right; }
</style>
