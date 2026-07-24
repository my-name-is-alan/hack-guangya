<script setup>
import { reactive, ref } from 'vue';
import { message } from 'antdv-next';
import { DownloadOutlined, InboxOutlined, LinkOutlined, ReloadOutlined } from '@antdv-next/icons';
import { appState, currentFolderId } from '../store.js';
import { bridge } from '../bridge.js';
import { errorText, formatSize, offlineStatus, pick, unwrapData } from '../formatters.js';

const offlineTasks = ref([]);
const offlineLoading = ref(false);
const offlineSubmitting = ref(false);
const offlineForm = reactive({ url: '' });
const offlineColumns = [
  { title: '任务', key: 'name', ellipsis: true },
  { title: '大小', key: 'size', width: 120 },
  { title: '状态', key: 'status', width: 110 },
];

async function loadOffline() {
  if (!appState.logged_in) return;
  offlineLoading.value = true;
  try {
    const data = unwrapData(await bridge.invoke('list_offline_tasks'));
    const list = data.list || data.taskList || data.tasks || data.items || [];
    offlineTasks.value = Array.isArray(list) ? list : [];
  } catch (error) {
    message.error(errorText(error));
  } finally {
    offlineLoading.value = false;
  }
}

async function submitOffline() {
  if (!offlineForm.url.trim()) {
    message.warning('请输入下载链接');
    return;
  }
  offlineSubmitting.value = true;
  try {
    await bridge.invoke('create_offline_task', { url: offlineForm.url.trim(), parent_id: currentFolderId.value });
    offlineForm.url = '';
    await loadOffline();
    message.success('离线任务已创建');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    offlineSubmitting.value = false;
  }
}

defineExpose({ loadOffline });
</script>

<template>
  <div class="view-section">
    <div class="section-toolbar">
      <div class="section-title">
        <div class="section-icon"><LinkOutlined /></div>
        <div><h2>离线下载</h2><p>提交链接由云端直接下载到当前目录</p></div>
      </div>
      <a-button :loading="offlineLoading" :disabled="!appState.logged_in" @click="loadOffline"><template #icon><ReloadOutlined /></template>刷新</a-button>
    </div>

    <a-card class="content-card" :bordered="false">
      <a-form layout="vertical" @submit.prevent="submitOffline">
        <a-form-item label="下载链接">
          <a-textarea v-model:value="offlineForm.url" :rows="3" placeholder="支持 http/https/磁力链接，粘贴后点击开始云端下载" />
        </a-form-item>
        <a-button type="primary" html-type="submit" :loading="offlineSubmitting" :disabled="!appState.logged_in"><template #icon><DownloadOutlined /></template>开始云端下载</a-button>
      </a-form>
    </a-card>

    <a-card class="content-card" :bordered="false" title="任务列表">
      <a-table :columns="offlineColumns" :data-source="offlineTasks" :loading="offlineLoading" :row-key="(item) => pick(item, ['taskId', 'id', 'fileId'], item.fileName || item.name)" :pagination="false" size="small">
        <template #emptyText><a-empty :description="appState.logged_in ? '暂无离线任务' : '登录后查看离线任务'" /></template>
        <template #bodyCell="{ column, record }">
          <template v-if="column.key === 'name'"><a-flex align="center" gap="small"><a-avatar class="list-avatar"><InboxOutlined /></a-avatar><strong>{{ pick(record, ['fileName', 'name', 'taskName', 'title'], '离线任务') }}</strong></a-flex></template>
          <template v-else-if="column.key === 'size'">{{ formatSize(pick(record, ['totalSize', 'fileSize', 'size'], 0)) }}</template>
          <template v-else-if="column.key === 'status'"><a-tag :color="offlineStatus(record)[1]">{{ offlineStatus(record)[0] }}</a-tag></template>
        </template>
      </a-table>
    </a-card>
  </div>
</template>
