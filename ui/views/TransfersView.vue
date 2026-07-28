<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import { storeToRefs } from 'pinia'
import { useRoute, useRouter } from 'vue-router'
import {
  CheckCircleOutlined,
  ClearOutlined,
  CloseCircleOutlined,
  CloudDownloadOutlined,
  CloudUploadOutlined,
  LoadingOutlined,
  PauseCircleOutlined,
  PlayCircleOutlined,
} from '@antdv-next/icons'
import { bridge } from '../bridge.js'
import { formatSize } from '../formatters.js'
import { useSessionStore } from '../stores/session'
import { useTransfersStore } from '../stores/transfers'
import { formatUploadSpeed, uploadProgressStatus } from '../uploadProgress.js'

const route = useRoute()
const router = useRouter()
const session = useSessionStore()
const transfers = useTransfersStore()
const { orderedUploads, downloads } = storeToRefs(transfers)
const tab = shallowRef(String(route.query.tab || 'upload'))

const uploadCounts = computed(() => ({
  active: orderedUploads.value.filter(item => !['done', 'error'].includes(item.state)).length,
  finished: orderedUploads.value.filter(item => ['done', 'error'].includes(item.state)).length,
}))
const downloadCounts = computed(() => ({
  active: downloads.value.filter(item => ['queued', 'preparing', 'downloading'].includes(item.status)).length,
  finished: downloads.value.filter(item => ['completed', 'failed'].includes(item.status)).length,
}))

function downloadDetail(item: (typeof downloads.value)[number]) {
  if (item.status === 'failed' && item.error) return `失败原因：${item.error}`
  return item.filePath || item.destination
}

async function toggleQueue() {
  await bridge.invoke(session.state.paused ? 'resume_queue' : 'pause_queue')
  await session.refreshState()
}

watch(() => route.query.tab, value => {
  if (value === 'upload' || value === 'download') tab.value = value
})
watch(tab, value => void router.replace({ query: { ...route.query, tab: value } }))
</script>

<template>
  <div class="view-section transfers-view">
    <a-tabs v-model:active-key="tab" class="page-tabs">
      <template #rightExtra>
        <a-space wrap class="transfer-toolbar-actions">
          <a-button @click="toggleQueue">
            <template #icon><PlayCircleOutlined v-if="session.state.paused" /><PauseCircleOutlined v-else /></template>
            {{ session.state.paused ? '恢复队列' : '暂停队列' }}
          </a-button>
          <a-button @click="transfers.clearFinished(tab === 'upload' ? 'upload' : 'download')"><template #icon><ClearOutlined /></template>清除已完成</a-button>
        </a-space>
      </template>

      <a-tab-pane key="upload" :tab="`上传 ${uploadCounts.active ? `(${uploadCounts.active})` : ''}`">
        <a-empty v-if="!orderedUploads.length" class="section-empty" description="暂无上传任务" />
        <div v-else class="transfer-list">
          <div v-for="item in orderedUploads" :key="item.filePath" class="transfer-row">
            <span class="transfer-icon upload"><CloudUploadOutlined /></span>
            <div class="transfer-main">
              <div class="transfer-name">
                <strong :title="item.filePath">{{ item.fileName }}</strong>
                <a-tooltip :title="item.stage" :trigger="['hover', 'focus']" placement="topLeft">
                  <span class="transfer-detail" tabindex="0" :aria-label="`任务详情：${item.stage}`">{{ item.stage }}</span>
                </a-tooltip>
              </div>
              <a-progress :percent="item.percent" :status="uploadProgressStatus(item.state)" :show-info="false" size="small" />
            </div>
            <span class="transfer-speed">
              <template v-if="item.totalBytes">{{ formatSize(item.uploadedBytes) }} / {{ formatSize(item.totalBytes) }}</template>
              <template v-if="item.bytesPerSecond"><br />{{ formatUploadSpeed(item.bytesPerSecond) }}</template>
            </span>
            <a-tag v-if="item.state === 'done'" class="transfer-status" color="success"><CheckCircleOutlined /> 完成</a-tag>
            <a-tag v-else-if="item.state === 'error'" class="transfer-status" color="error"><CloseCircleOutlined /> 失败</a-tag>
            <a-tag v-else class="transfer-status" color="processing"><LoadingOutlined spin /> {{ item.percent }}%</a-tag>
          </div>
        </div>
      </a-tab-pane>

      <a-tab-pane key="download" :tab="`下载 ${downloadCounts.active ? `(${downloadCounts.active})` : ''}`">
        <a-empty v-if="!downloads.length" class="section-empty" description="暂无下载任务" />
        <div v-else class="transfer-list">
          <div v-for="item in downloads" :key="item.id" class="transfer-row">
            <span class="transfer-icon download"><CloudDownloadOutlined /></span>
            <div class="transfer-main">
              <div class="transfer-name">
                <strong :title="item.fileName">{{ item.fileName }}</strong>
                <a-tooltip :title="downloadDetail(item)" :trigger="['hover', 'focus']" placement="topLeft">
                  <span class="transfer-detail" tabindex="0" :aria-label="`任务详情：${downloadDetail(item)}`">{{ downloadDetail(item) }}</span>
                </a-tooltip>
              </div>
              <a-progress :percent="Math.round(item.progress || 0)" :status="item.status === 'failed' ? 'exception' : item.status === 'completed' ? 'success' : 'normal'" :show-info="false" size="small" />
            </div>
            <span class="transfer-speed">{{ item.bytesPerSecond ? formatUploadSpeed(item.bytesPerSecond) : item.totalBytes ? `${formatSize(item.downloadedBytes)} / ${formatSize(item.totalBytes)}` : '' }}</span>
            <a-tag v-if="item.status === 'completed'" class="transfer-status" color="success"><CheckCircleOutlined /> 完成</a-tag>
            <a-tag v-else-if="item.status === 'failed'" class="transfer-status" color="error"><CloseCircleOutlined /> 失败</a-tag>
            <a-tag v-else class="transfer-status" color="processing"><LoadingOutlined spin /> {{ Math.round(item.progress || 0) }}%</a-tag>
          </div>
        </div>
      </a-tab-pane>
    </a-tabs>
  </div>
</template>

<style scoped>
.transfers-view, .transfer-list { min-width: 0; }
.transfer-list { display: grid; }
.transfer-row { display: grid; min-width: 0; grid-template-columns:38px minmax(0,1fr) minmax(100px,150px) minmax(80px,100px); align-items: center; gap: 12px; min-height: 64px; padding: 8px 4px; border-bottom: 1px solid var(--line, #e7e8eb); }
.transfer-icon { display: grid; width: 34px; height: 34px; place-items: center; border-radius: 9px; }
.transfer-icon.upload { color: var(--primary-strong, #237804); background: var(--primary-soft, #f1f8ed); }
.transfer-icon.download { color: #1769aa; background: #edf5ff; }
.transfer-main { min-width: 0; }
.transfer-name { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 6px; }
.transfer-name strong, .transfer-detail { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.transfer-name strong { flex: 1 1 42%; }
.transfer-detail { flex: 1 1 58%; color: var(--text-3, #98a2b3); cursor: help; font-size: 11px; outline: none; }
.transfer-detail:focus-visible { border-radius: 3px; box-shadow: 0 0 0 2px color-mix(in srgb, var(--primary, #262626) 24%, transparent); }
.transfer-speed { color: var(--text-3, #98a2b3); font-size: 11px; }
.transfer-speed { min-width: 0; overflow: hidden; text-align: right; text-overflow: ellipsis; white-space: nowrap; }
.transfer-status { justify-self: end; white-space: nowrap; }
@media (max-width: 720px) {
  .transfer-row { grid-template-columns:34px minmax(0,1fr) auto; gap: 6px 8px; }
  .transfer-icon { grid-column: 1; grid-row: 1 / span 2; }
  .transfer-main { grid-column: 2; grid-row: 1; }
  .transfer-speed { grid-column: 2; grid-row: 2; text-align: left; }
  .transfer-status { grid-column: 3; grid-row: 1 / span 2; }
}
</style>
