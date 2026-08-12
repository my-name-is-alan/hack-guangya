<script setup>
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { message, Modal } from 'antdv-next';
import { DeleteOutlined, FileOutlined, ReloadOutlined, RollbackOutlined } from '@antdv-next/icons';
import { bridge } from '../../bridge.js';
import { useFilesStore } from '../../stores/files.ts';
import { errorText, fileId, formatSize, formatTime, pick, unwrapData } from '../../formatters.js';
import {
  requestRecycleBinClear,
  subscribeRecycleBinClear,
  waitForRecycleBinClear,
} from '../../recycleBinClearOperation.js';

const pageSize = 100;
const records = ref([]);
const total = ref(0);
const page = ref(0);
const loading = ref(false);
const mutationBusy = ref(false);
const clearBusy = ref(false);
const clearUnknown = ref(false);
const actionBusy = computed(() => mutationBusy.value || clearBusy.value);
const loadError = ref('');
const selectedKeys = ref([]);
const focusedRowId = ref('');

const columns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '类型', key: 'ext', width: 90 },
  { title: '大小', key: 'size', width: 110 },
  { title: '删除时间', key: 'time', width: 170 },
  { title: '剩余时间', key: 'leftTime', width: 120 },
  { title: '操作', key: 'actions', width: 150 },
];
const pagination = computed(() => ({
  current: page.value + 1,
  pageSize,
  total: total.value,
  showSizeChanger: false,
  hideOnSinglePage: true,
  showQuickJumper: total.value > pageSize * 5,
  showTotal: (value) => `共 ${value} 项`,
}));
const rowSelection = computed(() => ({
  selectedRowKeys: selectedKeys.value,
  onChange: (keys) => { selectedKeys.value = keys; },
}));

function recordId(record) {
  return fileId(record);
}

function leftTimeLabel(record) {
  const seconds = Number(pick(record, ['leftTime', 'left_time'], 0));
  if (seconds < 0) return '长期保留';
  if (!seconds) return '即将清理';
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  if (days > 0) return hours ? `${days} 天 ${hours} 小时` : `${days} 天`;
  const minutes = Math.max(1, Math.ceil(seconds / 60));
  return minutes >= 60 ? `${Math.floor(minutes / 60)} 小时` : `${minutes} 分钟`;
}

async function loadRecycle(nextPage = page.value, preferredFocus = '') {
  const normalizedPage = Math.max(0, Math.floor(Number(nextPage) || 0));
  loading.value = true;
  loadError.value = '';
  try {
    await waitForRecycleBinClear();
    const data = unwrapData(await bridge.invoke('list_recycle_files', { page: normalizedPage, page_size: pageSize }));
    const list = data.list || data.files || data.items || data.restoreList || [];
    records.value = Array.isArray(list) ? list : [];
    total.value = Number(data.total ?? data.totalCount ?? (normalizedPage * pageSize + records.value.length)) || 0;
    page.value = normalizedPage;
    selectedKeys.value = [];
    const visibleIds = records.value.map(recordId).filter(Boolean).map(String);
    focusedRowId.value = visibleIds.includes(String(preferredFocus)) ? String(preferredFocus) : (visibleIds[0] || '');
    await restoreRowFocus();
  } catch (error) {
    loadError.value = errorText(error);
    message.error(loadError.value);
  } finally {
    loading.value = false;
  }
}

async function restoreRowFocus() {
  await nextTick();
  if (!focusedRowId.value) return;
  document.querySelector(`[data-recycle-id="${CSS.escape(focusedRowId.value)}"]`)?.focus?.({ preventScroll: true });
}

async function executeAction(command, ids, successText, preferredFocus = '') {
  mutationBusy.value = true;
  try {
    await bridge.invoke(command, { file_ids: [...new Set(ids.filter(Boolean))] });
    const shouldGoPrevious = records.value.length <= ids.length && page.value > 0;
    await loadRecycle(shouldGoPrevious ? page.value - 1 : page.value, preferredFocus);
    message.success(successText);
  } catch (error) {
    message.error(errorText(error));
    throw error;
  } finally {
    mutationBusy.value = false;
  }
}

function confirmRestore(targets) {
  const ids = targets.map(recordId).filter(Boolean);
  if (!ids.length) return;
  const label = targets.length === 1 ? `「${targets[0].fileName || '未命名文件'}」` : `选中的 ${targets.length} 项`;
  const index = records.value.findIndex((item) => String(recordId(item)) === String(ids[0]));
  const preferred = recordId(records.value[index + 1] || records.value[index - 1]);
  Modal.confirm({
    title: '恢复文件',
    content: `将 ${label} 恢复到原位置吗？`,
    okText: '恢复',
    cancelText: '取消',
    async onOk() { await executeAction('restore_files', ids, `已恢复 ${ids.length} 项`, preferred); },
  });
}

function confirmPermanentDelete(targets) {
  const ids = targets.map(recordId).filter(Boolean);
  if (!ids.length) return;
  const label = targets.length === 1 ? `「${targets[0].fileName || '未命名文件'}」` : `选中的 ${targets.length} 项`;
  const index = records.value.findIndex((item) => String(recordId(item)) === String(ids[0]));
  const preferred = recordId(records.value[index + 1] || records.value[index - 1]);
  Modal.confirm({
    title: '彻底删除',
    content: `${label} 将被永久删除，且无法恢复。是否继续？`,
    okText: '永久删除',
    okButtonProps: { danger: true },
    cancelText: '取消',
    async onOk() { await executeAction('permanently_delete_files', ids, `已彻底删除 ${ids.length} 项`, preferred); },
  });
}

async function runRecycleBinClear(forceRetry = false) {
  const request = requestRecycleBinClear(() => bridge.invoke('clear_recycle_bin', { force_retry: forceRetry }));
  try {
    const result = unwrapData(await request.promise);
    if (!request.started) return;
    const clearState = String(result?.status ?? result?.state ?? '');
    if (result?.pending || ['pending', 'unknown'].includes(clearState)) {
      clearUnknown.value = clearState === 'unknown';
      message.warning(result?.message || (clearState === 'unknown'
        ? '上一次清空请求结果未知，系统不会自动重复提交。请刷新确认；如仍需清空，可再次点击并进行强制确认。'
        : '清空任务仍在云端执行；再次点击清空时会继续查询同一个任务，不会重复提交。'));
      void loadRecycle(0);
      return;
    }
    records.value = [];
    total.value = 0;
    page.value = 0;
    selectedKeys.value = [];
    focusedRowId.value = '';
    clearUnknown.value = false;
    message.success('回收站已清空');
    void loadRecycle(0);
  } catch (error) {
    if (request.started) message.error(errorText(error));
  }
}

function clearRecycleBin() {
  if (!records.value.length && !total.value) return;
  const forceRetry = clearUnknown.value;
  Modal.confirm({
    title: forceRetry ? '强制重新提交清空' : '清空回收站',
    content: forceRetry
      ? '上一次清空请求是否已被云端接收仍无法确认。强制重新提交可能连同此后新进入回收站的文件一起永久删除。请先刷新列表确认；仍要强制提交吗？'
      : `回收站中的 ${total.value || records.value.length} 项将被永久删除，且无法恢复。`,
    okText: forceRetry ? '仍要强制提交' : '清空回收站',
    okButtonProps: { danger: true },
    cancelText: '取消',
    onOk() { void runRecycleBinClear(forceRetry); },
  });
}

function selectedRecords() {
  const ids = new Set(selectedKeys.value.map(String));
  return records.value.filter((record) => ids.has(String(recordId(record))));
}

function tableChange(tablePagination) {
  const nextPage = Math.max(0, Number(tablePagination?.current || 1) - 1);
  if (nextPage !== page.value) void loadRecycle(nextPage);
}

function rowProps(record, index) {
  const id = String(recordId(record));
  return {
    tabindex: focusedRowId.value ? (focusedRowId.value === id ? 0 : -1) : (index === 0 ? 0 : -1),
    'data-recycle-id': id,
    'aria-selected': selectedKeys.value.map(String).includes(id),
    onFocus: () => { focusedRowId.value = id; },
    onKeydown: (event) => {
      if (event.key === ' ') {
        event.preventDefault();
        const selected = new Set(selectedKeys.value.map(String));
        if (selected.has(id)) selected.delete(id); else selected.add(id);
        selectedKeys.value = records.value.filter((item) => selected.has(String(recordId(item)))).map(recordId);
      }
      if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      const current = records.value.findIndex((item) => String(recordId(item)) === id);
      const target = event.key === 'Home' ? 0 : event.key === 'End' ? records.value.length - 1 : current + (event.key === 'ArrowUp' ? -1 : 1);
      const next = records.value[Math.max(0, Math.min(records.value.length - 1, target))];
      if (!next) return;
      focusedRowId.value = String(recordId(next));
      void restoreRowFocus();
    },
  };
}

let unsubscribeClear = () => {};
onMounted(() => {
  unsubscribeClear = subscribeRecycleBinClear((active) => { clearBusy.value = active; });
  void loadRecycle();
});
onUnmounted(() => unsubscribeClear());

// 文件页删除/其它端清空回收站后，后端会广播回收站变化事件；
// 已打开的回收站面板据此自动刷新，不再显示旧列表。
const filesStore = useFilesStore();
watch(() => filesStore.recycleBinVersion, () => {
  if (!loading.value && !actionBusy.value) void loadRecycle(page.value);
});
</script>

<template>
  <section class="files-panel" aria-label="云盘回收站">
    <div class="plain-toolbar">
      <div v-if="selectedKeys.length" class="selection-summary" role="status">
        已选 {{ selectedKeys.length }} 项
        <a-button type="link" size="small" @click="selectedKeys = []">取消选择</a-button>
      </div>
      <div v-else class="panel-summary">
        <strong>回收站</strong>
        <span>到期项目会由光鸭自动清理</span>
      </div>
      <a-space wrap>
        <template v-if="selectedKeys.length">
          <a-button :loading="actionBusy" @click="confirmRestore(selectedRecords())"><template #icon><RollbackOutlined /></template>恢复</a-button>
          <a-button danger :loading="actionBusy" @click="confirmPermanentDelete(selectedRecords())"><template #icon><DeleteOutlined /></template>彻底删除</a-button>
        </template>
        <a-button danger :loading="clearBusy" :disabled="(!records.length && !total) || mutationBusy" @click="clearRecycleBin">{{ clearUnknown ? '强制重新清空' : '清空回收站' }}</a-button>
        <a-button :loading="loading" :disabled="actionBusy" aria-label="刷新回收站" @click="loadRecycle(page)"><template #icon><ReloadOutlined /></template>刷新</a-button>
      </a-space>
    </div>

    <a-alert v-if="loadError" class="panel-alert" type="error" show-icon :message="loadError">
      <template #action><a-button size="small" @click="loadRecycle(page)">重试</a-button></template>
    </a-alert>
    <a-table
      :columns="columns"
      :data-source="records"
      :loading="loading"
      :row-key="recordId"
      :row-selection="rowSelection"
      :on-row="rowProps"
      :pagination="pagination"
      size="small"
      @change="tableChange"
    >
      <template #emptyText><a-empty description="回收站为空" /></template>
      <template #bodyCell="{ column, record }">
        <template v-if="column.key === 'name'">
          <a-flex align="center" gap="small">
            <a-avatar class="list-avatar"><FileOutlined /></a-avatar>
            <strong class="recycle-name" :title="record.fileName">{{ record.fileName || '未命名文件' }}</strong>
          </a-flex>
        </template>
        <template v-else-if="column.key === 'ext'">{{ String(record.ext || '').replace(/^\./, '').toUpperCase() || '文件' }}</template>
        <template v-else-if="column.key === 'size'">{{ formatSize(record.fileSize) }}</template>
        <template v-else-if="column.key === 'time'">{{ formatTime(record.utime) }}</template>
        <template v-else-if="column.key === 'leftTime'"><a-tag :color="Number(record.leftTime) <= 86400 ? 'warning' : 'default'">{{ leftTimeLabel(record) }}</a-tag></template>
        <template v-else-if="column.key === 'actions'">
          <a-space :size="4">
            <a-button size="small" type="text" aria-label="恢复文件" @click="confirmRestore([record])"><template #icon><RollbackOutlined /></template></a-button>
            <a-button size="small" type="text" danger aria-label="彻底删除文件" @click="confirmPermanentDelete([record])"><template #icon><DeleteOutlined /></template></a-button>
          </a-space>
        </template>
      </template>
    </a-table>
  </section>
</template>

<style scoped>
.files-panel { min-height: 0; }
.panel-summary strong, .panel-summary span { display: block; }
.panel-summary span { margin-top: 2px; color: var(--text-3, #737373); font-size: 12px; }
.selection-summary { color: var(--text-2, #525252); }
.panel-alert { margin-bottom: 10px; }
.recycle-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
