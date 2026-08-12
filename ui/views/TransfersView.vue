<script setup lang="ts">
import { computed, shallowRef, watch } from 'vue'
import { message } from 'antdv-next'
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
  ReloadOutlined,
} from '@antdv-next/icons'
import { bridge } from '../bridge.js'
import PageHeader from '../components/layout/PageHeader.vue'
import { errorText, formatSize } from '../formatters.js'
import { useSessionStore } from '../stores/session'
import { useTransfersStore } from '../stores/transfers'
import { formatUploadSpeed, uploadProgressStatus } from '../uploadProgress.js'

const route = useRoute()
const router = useRouter()
const session = useSessionStore()
const transfers = useTransfersStore()
const { orderedUploads, downloads } = storeToRefs(transfers)
const tab = shallowRef(String(route.query.tab || 'upload'))
const queueToggling = shallowRef(false)

const uploadCounts = computed(() => ({
  active: orderedUploads.value.filter(item => !['done', 'error', 'cancelled'].includes(item.state)).length,
  finished: orderedUploads.value.filter(item => ['done', 'error', 'cancelled'].includes(item.state)).length,
}))
const downloadCounts = computed(() => ({
  active: downloads.value.filter(item => ['queued', 'preparing', 'downloading', 'paused'].includes(item.status)).length,
  finished: downloads.value.filter(item => ['completed', 'failed', 'cancelled'].includes(item.status)).length,
}))

async function pauseDownload(id: string) {
  try { await transfers.pauseDownload(id) }
  catch (error) { message.error(errorText(error)) }
}

async function resumeDownload(id: string) {
  try { await transfers.resumeDownload(id) }
  catch (error) { message.error(errorText(error)) }
}

async function cancelDownload(id: string) {
  try { await transfers.cancelDownload(id) }
  catch (error) { message.error(errorText(error)) }
}

async function cancelUpload(filePath: string) {
  try { await transfers.cancelUpload(filePath) }
  catch (error) { message.error(errorText(error)) }
}

async function pauseUpload(filePath: string) {
  try { await transfers.pauseUpload(filePath) }
  catch (error) { message.error(errorText(error)) }
}

async function resumeUpload(filePath: string) {
  try { await transfers.resumeUpload(filePath) }
  catch (error) { message.error(errorText(error)) }
}

async function retryUpload(filePath: string) {
  try { await transfers.retryUpload(filePath) }
  catch (error) { message.error(errorText(error)) }
}

async function toggleQueue() {
  if (queueToggling.value) return
  queueToggling.value = true
  const resume = session.state.paused
  try {
    await bridge.invoke(resume ? 'resume_queue' : 'pause_queue')
    await session.refreshState()
    message.success(resume ? '传输队列已恢复' : '传输队列已暂停')
  }
  catch (error) { message.error(errorText(error)) }
  finally { queueToggling.value = false }
}

watch(() => route.query.tab, value => {
  if (value === 'upload' || value === 'download') tab.value = value
})
watch(tab, value => void router.replace({ query: { ...route.query, tab: value } }))
</script>

<template>
  <div class="view-section transfers-view">
    <PageHeader title="传输任务" description="上传与下载队列的进度、暂停恢复与断点续传。" />
    <a-tabs v-model:active-key="tab" class="page-tabs">
      <template #rightExtra>
        <a-space>
          <a-button :loading="queueToggling" @click="toggleQueue">
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
              <div class="transfer-name"><strong :title="item.filePath">{{ item.fileName }}</strong><span>{{ item.stage }}</span></div>
              <a-progress :percent="item.percent" :status="uploadProgressStatus(item.state)" :show-info="false" size="small" />
            </div>
            <span class="transfer-speed">
              <template v-if="item.totalBytes">{{ formatSize(item.uploadedBytes) }} / {{ formatSize(item.totalBytes) }}</template>
              <template v-if="item.bytesPerSecond"><br />{{ formatUploadSpeed(item.bytesPerSecond) }}</template>
            </span>
            <a-tag v-if="item.state === 'done'" color="success"><CheckCircleOutlined /> 完成</a-tag>
            <a-tag v-else-if="item.state === 'error'" color="error"><CloseCircleOutlined /> 失败</a-tag>
            <a-tag v-else-if="item.state === 'cancelled'"><CloseCircleOutlined /> 已取消</a-tag>
            <a-tag v-else-if="item.state === 'paused'" color="warning"><PauseCircleOutlined /> 已暂停</a-tag>
            <a-tag v-else color="processing"><LoadingOutlined spin /> {{ item.percent }}%</a-tag>
            <a-space class="transfer-actions" size="small">
              <a-button v-if="['queued', 'waiting-login', 'waiting-file', 'preparing', 'uploading'].includes(item.state)" size="small" title="暂停上传" @click="pauseUpload(item.filePath)"><PauseCircleOutlined /></a-button>
              <a-button v-if="item.state === 'paused'" size="small" title="继续上传" @click="resumeUpload(item.filePath)"><PlayCircleOutlined /></a-button>
              <a-button v-if="item.state === 'error'" size="small" type="primary" title="重试上传" @click="retryUpload(item.filePath)"><ReloadOutlined /> 重试</a-button>
              <a-popconfirm v-if="!['done', 'error', 'cancelled'].includes(item.state)" title="确定取消这个上传任务？未完成的上传断点会被清理。" ok-text="取消上传" cancel-text="返回" @confirm="cancelUpload(item.filePath)">
                <a-button size="small" danger title="取消上传" aria-label="取消上传"><CloseCircleOutlined /></a-button>
              </a-popconfirm>
            </a-space>
          </div>
        </div>
      </a-tab-pane>

      <a-tab-pane key="download" :tab="`下载 ${downloadCounts.active ? `(${downloadCounts.active})` : ''}`">
        <a-empty v-if="!downloads.length" class="section-empty" description="暂无下载任务" />
        <div v-else class="transfer-list">
          <div v-for="item in downloads" :key="item.id" class="transfer-row">
            <span class="transfer-icon download"><CloudDownloadOutlined /></span>
            <div class="transfer-main">
              <div class="transfer-name"><strong :title="item.fileName">{{ item.fileName }}</strong><span>{{ item.filePath || item.destination }}</span></div>
              <a-progress :percent="Math.round(item.progress || 0)" :status="['failed', 'cancelled'].includes(item.status) ? 'exception' : item.status === 'completed' ? 'success' : 'normal'" :show-info="false" size="small" />
            </div>
            <span class="transfer-speed">
              {{ item.bytesPerSecond ? formatUploadSpeed(item.bytesPerSecond) : item.totalBytes ? `${formatSize(item.downloadedBytes)} / ${formatSize(item.totalBytes)}` : '' }}
              <template v-if="item.segmented && item.connections > 1"><br />{{ item.connections }} 路分片</template>
            </span>
            <a-tag v-if="item.status === 'completed'" color="success"><CheckCircleOutlined /> 完成</a-tag>
            <a-tag v-else-if="item.status === 'failed'" color="error"><CloseCircleOutlined /> 失败</a-tag>
            <a-tag v-else-if="item.status === 'cancelled'"><CloseCircleOutlined /> 已取消</a-tag>
            <a-tag v-else-if="item.status === 'paused'" color="warning"><PauseCircleOutlined /> 已暂停</a-tag>
            <a-tag v-else color="processing"><LoadingOutlined spin /> {{ Math.round(item.progress || 0) }}%</a-tag>
            <a-space class="transfer-actions" size="small">
              <a-button v-if="['queued', 'preparing', 'downloading'].includes(item.status)" size="small" title="暂停下载" @click="pauseDownload(item.id)"><PauseCircleOutlined /></a-button>
              <a-button v-if="item.status === 'paused'" size="small" title="继续下载" @click="resumeDownload(item.id)"><PlayCircleOutlined /></a-button>
              <a-popconfirm v-if="['queued', 'preparing', 'downloading', 'paused'].includes(item.status)" title="确定取消这个下载任务？临时分片会被清理。" ok-text="取消下载" cancel-text="返回" @confirm="cancelDownload(item.id)">
                <a-button size="small" danger title="取消下载"><CloseCircleOutlined /></a-button>
              </a-popconfirm>
            </a-space>
          </div>
        </div>
      </a-tab-pane>
    </a-tabs>
  </div>
</template>

<style scoped>
.transfer-list { display: grid; }
.transfer-row { display: grid; grid-template-columns:38px minmax(260px,1fr) 150px 100px auto; align-items: center; gap: 12px; min-height: 64px; padding: 8px 4px; border-bottom: 1px solid var(--line, #e5e5e5); }
.transfer-icon { display: grid; width: 34px; height: 34px; place-items: center; border-radius: 9px; }
.transfer-icon.upload { color: var(--primary-strong, #171717); background: var(--primary-soft, #f5f5f5); }
.transfer-icon.download { color: var(--text-2, #525252); background: var(--bg-hover, #f5f5f5); }
.transfer-main { min-width: 0; }
.transfer-name { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 6px; }
.transfer-name strong, .transfer-name span { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.transfer-name span, .transfer-speed { color: var(--text-3, #737373); font-size: 11px; }
.transfer-speed { text-align: right; }
.transfer-actions { justify-self: end; }
</style>
