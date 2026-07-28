<script setup>
import { onMounted, reactive, ref } from 'vue';
import { message } from 'antdv-next';
import { DownloadOutlined, InboxOutlined, LinkOutlined, ReloadOutlined } from '@antdv-next/icons';
import { appState, currentFolderId } from '../store.js';
import { bridge } from '../bridge.js';
import { errorText, formatSize, offlineStatus, pick, unwrapData } from '../formatters.js';

const offlineTasks = ref([]);
const offlineLoading = ref(false);
const offlineSubmitting = ref(false);
const offlineForm = reactive({ open: false, url: '' });
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
    await bridge.invoke('create_offline_task', { url: offlineForm.url.trim(), parent_id: currentFolderId.value, new_name: '' });
    offlineForm.url = '';
    offlineForm.open = false;
    await loadOffline();
    message.success('离线任务已创建');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    offlineSubmitting.value = false;
  }
}

defineExpose({ loadOffline });
onMounted(loadOffline);
</script>

<template>
  <div class="view-section">
    <div class="plain-toolbar">
      <span />
      <a-space>
        <a-button :loading="offlineLoading" @click="loadOffline"><template #icon><ReloadOutlined /></template>刷新</a-button>
        <a-button type="primary" @click="offlineForm.open = true"><template #icon><DownloadOutlined /></template>新建任务</a-button>
      </a-space>
    </div>

    <a-table :columns="offlineColumns" :data-source="offlineTasks" :loading="offlineLoading" :row-key="(item) => pick(item, ['taskId', 'id', 'fileId'], item.fileName || item.name)" :pagination="false" size="small">
      <template #emptyText><a-empty description="暂无离线任务" /></template>
      <template #bodyCell="{ column, record }">
        <template v-if="column.key === 'name'"><a-flex align="center" gap="small"><a-avatar class="list-avatar"><InboxOutlined /></a-avatar><strong>{{ pick(record, ['fileName', 'name', 'taskName', 'title'], '离线任务') }}</strong></a-flex></template>
        <template v-else-if="column.key === 'size'">{{ formatSize(pick(record, ['totalSize', 'fileSize', 'size'], 0)) }}</template>
        <template v-else-if="column.key === 'status'"><a-tag :color="offlineStatus(record)[1]">{{ offlineStatus(record)[0] }}</a-tag></template>
      </template>
    </a-table>

    <a-modal v-model:open="offlineForm.open" title="新建离线任务" :confirm-loading="offlineSubmitting" ok-text="开始下载" cancel-text="取消" @ok="submitOffline">
      <a-form layout="vertical" @submit.prevent="submitOffline">
        <a-form-item label="下载链接">
          <a-textarea v-model:value="offlineForm.url" :rows="3" placeholder="支持 http/https/磁力链接，粘贴后点击开始云端下载" />
        </a-form-item>
      </a-form>
    </a-modal>
  </div>
</template>
