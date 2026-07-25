<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue';
import { message } from 'antdv-next';
import {
  CheckCircleOutlined,
  ClearOutlined,
  CloseCircleOutlined,
  CloudDownloadOutlined,
  DeleteOutlined,
  DownloadOutlined,
  LoadingOutlined,
  PauseCircleOutlined,
  PlayCircleOutlined,
  RedoOutlined,
} from '@antdv-next/icons';
import { isTauri } from '../bridge.js';
import { appState, formatUploadSpeed } from '../store.js';
import { errorText } from '../formatters.js';

const downloads = ref([]);
const activeDownloads = computed(() => downloads.value.filter((item) => ['pending', 'downloading'].includes(item.status)).length);
const finishedDownloads = computed(() => downloads.value.filter((item) => item.status === 'done').length);
const downloadSpeed = computed(() => downloads.value.reduce((total, item) => total + Number(item.speed_bps || 0), 0));

let unlistenDownload = null;

async function startDownload(item) {
  if (!isTauri) {
    window.open(item.url, '_blank');
    item.status = 'done';
    item.progress = 100;
    return;
  }
  item.status = 'downloading';
  item.error = '';
  try {
    const { invoke } = window.__TAURI__.core;
    await invoke('download_file', { taskId: item.id, url: item.url, fileName: item.name });
  } catch (error) {
    item.status = 'error';
    item.error = errorText(error);
  }
}
function pauseDownload(item) {
  if (!isTauri) return;
  window.__TAURI__.core.invoke('pause_download', { taskId: item.id }).catch(() => {});
  item.status = 'paused';
}
async function resumeDownload(item) {
  if (!isTauri) return;
  item.status = 'downloading';
  try {
    await window.__TAURI__.core.invoke('resume_download', { taskId: item.id });
  } catch (error) {
    item.status = 'error';
    item.error = errorText(error);
  }
}
function removeDownload(item) {
  if (['pending', 'downloading'].includes(item.status) && isTauri) {
    window.__TAURI__.core.invoke('cancel_download', { taskId: item.id }).catch(() => {});
  }
  downloads.value = downloads.value.filter((entry) => entry.id !== item.id);
}
function clearFinishedDownloads() {
  downloads.value = downloads.value.filter((item) => ['pending', 'downloading', 'paused'].includes(item.status));
}
function addDownloads(items) {
  const list = Array.isArray(items) ? items : [];
  if (!list.length) return;
  downloads.value = [...list, ...downloads.value];
  list.forEach((item) => startDownload(item));
}

function handleAddDownloads(event) {
  addDownloads(event.detail);
}

onMounted(async () => {
  window.addEventListener('guangya:add-downloads', handleAddDownloads);
  if (isTauri) {
    const { listen } = window.__TAURI__.event;
    unlistenDownload = await listen('download-event', ({ payload }) => {
      const item = downloads.value.find((entry) => entry.id === payload.task_id);
      if (!item) return;
      if (payload.event === 'progress') {
        item.progress = payload.progress;
        item.speed_bps = payload.speed_bps || 0;
      }
      if (payload.event === 'done') {
        item.status = 'done';
        item.progress = 100;
        item.speed_bps = 0;
        message.success(`「${item.name}」下载完成`);
      }
      if (payload.event === 'error') {
        item.status = 'error';
        item.error = payload.message || '下载失败';
        item.speed_bps = 0;
      }
    });
  }
});
onBeforeUnmount(() => {
  window.removeEventListener('guangya:add-downloads', handleAddDownloads);
  unlistenDownload?.();
});
</script>

<template>
  <div class="view-section">
    <div class="section-toolbar">
      <div class="section-title">
        <div class="section-icon"><DownloadOutlined /></div>
        <div><h2>下载管理</h2><p>{{ activeDownloads }} 个进行中 · {{ finishedDownloads }} 个已完成<span v-if="downloadSpeed"> · {{ formatUploadSpeed(downloadSpeed) }}</span></p></div>
      </div>
      <a-space>
        <a-tag v-if="appState.paused" color="warning">队列已暂停</a-tag>
        <a-button :disabled="!finishedDownloads" @click="clearFinishedDownloads"><template #icon><ClearOutlined /></template>清除已完成</a-button>
      </a-space>
    </div>

    <a-empty v-if="!downloads.length" class="section-empty" description="暂无下载任务" />
    <div v-else class="task-list">
      <a-card v-for="item in downloads" :key="item.id" class="task-card" :bordered="false">
        <a-flex align="center" gap="middle">
          <div class="task-icon download"><CloudDownloadOutlined /></div>
          <div class="task-body">
            <div class="task-title">
              <strong :title="item.name">{{ item.name }}</strong>
              <a-tag v-if="item.status === 'downloading'" color="processing"><LoadingOutlined /> 下载中</a-tag>
              <a-tag v-else-if="item.status === 'done'" color="success"><CheckCircleOutlined /> 已完成</a-tag>
              <a-tag v-else-if="item.status === 'error'" color="error"><CloseCircleOutlined /> 失败</a-tag>
              <a-tag v-else-if="item.status === 'paused'" color="warning"><PauseCircleOutlined /> 已暂停</a-tag>
              <a-tag v-else>等待中</a-tag>
            </div>
            <div class="task-meta">
              <a-progress :percent="Math.round(item.progress || 0)" size="small" :status="item.status === 'error' ? 'exception' : item.status === 'done' ? 'success' : 'active'" style="max-width: 320px" />
              <span v-if="item.speed_bps && item.status === 'downloading'">{{ formatUploadSpeed(item.speed_bps) }}</span>
              <span v-if="item.error" class="error-text">{{ item.error }}</span>
            </div>
          </div>
          <a-flex class="task-actions" align="center" gap="small">
            <a-button v-if="item.status === 'downloading'" size="small" @click="pauseDownload(item)"><template #icon><PauseCircleOutlined /></template>暂停</a-button>
            <a-button v-if="item.status === 'paused'" size="small" @click="resumeDownload(item)"><template #icon><PlayCircleOutlined /></template>继续</a-button>
            <a-button v-if="item.status === 'error'" size="small" @click="startDownload(item)"><template #icon><RedoOutlined /></template>重试</a-button>
            <a-button size="small" danger type="text" @click="removeDownload(item)"><template #icon><DeleteOutlined /></template></a-button>
          </a-flex>
        </a-flex>
      </a-card>
    </div>
  </div>
</template>
