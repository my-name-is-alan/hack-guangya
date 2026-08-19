<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from 'vue';
import { message, Modal } from 'antdv-next';
import { DeleteOutlined, DownloadOutlined, FolderOpenOutlined, InboxOutlined, RedoOutlined, ReloadOutlined, SearchOutlined, StopOutlined } from '@antdv-next/icons';
import CloudFolderPicker from '../components/cloud/CloudFolderPicker.vue';
import PageHeader from '../components/layout/PageHeader.vue';
import { appState } from '../store.js';
import { bridge } from '../bridge.js';
import { useFilesStore } from '../stores/files.ts';
import { errorText, formatSize, offlineProgress, offlineStatus, pick, unwrapData } from '../formatters.js';

const pageSize = 100;
const offlineTasks = ref([]);
const offlineLoading = ref(false);
const offlineSubmitting = ref(false);
const offlineActionBusy = ref(false);
const offlineLoadError = ref('');
const offlineStatistics = ref(null);
const offlineTotal = ref(0);
const currentCursor = ref('');
const nextCursor = ref('');
const cursorHistory = ref([]);
const hasMore = ref(false);
const selectedKeys = ref([]);
const focusedTaskId = ref('');
const offlineObfuscationEnabled = ref(false);
let offlinePollTimer = null;
const offlineForm = reactive({
  open: false,
  url: '',
  resolving: false,
  resolvedFor: '',
  resolved: null,
  resolveError: '',
  newName: '',
  targetId: '',
  targetLabel: '全部文件',
  pickerOpen: false,
});
const offlineColumns = [
  { title: '任务', key: 'name', ellipsis: true, width: 260 },
  { title: '大小', key: 'size', width: 110 },
  { title: '状态', key: 'status', width: 110 },
  { title: '进度', key: 'progress', width: 180 },
  { title: '错误', key: 'error', ellipsis: true, width: 200 },
  { title: '操作', key: 'actions', width: 190, fixed: 'right' },
];

const offlineFilters = reactive({ status: 'all', keyword: '' });
const offlineStatusFilterOptions = [
  { label: '全部', value: 'all' },
  { label: '进行中', value: 'running' },
  { label: '需重试', value: 'retryable' },
  { label: '已完成', value: 'done' },
];
const filteredOfflineTasks = computed(() => {
  const keyword = offlineFilters.keyword.trim().toLowerCase();
  return offlineTasks.value.filter((record) => {
    if (offlineFilters.status === 'running' && !isRunningTask(record)) return false;
    if (offlineFilters.status === 'retryable' && !isRetryableTask(record)) return false;
    if (offlineFilters.status === 'done' && (!isTerminalTask(record) || isRetryableTask(record))) return false;
    if (!keyword) return true;
    return [pick(record, ['fileName', 'name', 'taskName', 'title'], ''), offlineError(record)]
      .some((value) => String(value || '').toLowerCase().includes(keyword));
  });
});

function taskId(record) {
  return pick(record, ['taskId', 'id', 'fileId'], '');
}

function rawStatus(record) {
  return pick(record, ['status', 'taskStatus', 'state'], '');
}

function statusToken(record) {
  return String(rawStatus(record)).trim().toLowerCase();
}

function numericStatus(record) {
  const token = statusToken(record);
  return /^-?\d+$/.test(token) ? Number(token) : null;
}

function isPartialTask(record) {
  const token = statusToken(record);
  if (numericStatus(record) === 5) return true;
  if (/partial|partly|部分/.test(token)) return true;
  const total = Number(pick(record, ['totalCount', 'fileCount', 'resourceCount'], 0));
  const succeeded = Number(pick(record, ['successCount', 'finishedCount', 'completeCount'], 0));
  const failed = Number(pick(record, ['failedCount', 'failCount'], 0));
  return total > 0 && succeeded > 0 && failed > 0;
}

function isRunningTask(record) {
  const number = numericStatus(record);
  if (number !== null) return number === 0 || number === 1;
  const token = statusToken(record);
  if (!token) return !offlineError(record);
  return /pending|waiting|queue|queued|running|download|process/.test(token);
}

function isRetryableTask(record) {
  if (isPartialTask(record)) return true;
  const number = numericStatus(record);
  if (number !== null) return number === 3 || number === 4;
  const token = statusToken(record);
  return /fail|error|cancel/.test(token) || Boolean(offlineError(record));
}

function isTerminalTask(record) {
  if (isRunningTask(record)) return false;
  const number = numericStatus(record);
  if (number !== null) return [2, 3, 4, 5].includes(number);
  return isPartialTask(record) || /success|done|complete|finish|fail|error|cancel|forbid|violation/.test(statusToken(record));
}

function isCompletedTask(record) {
  if (!isTerminalTask(record)) return false;
  const number = numericStatus(record);
  if (number !== null) return number === 2 || number === 5;
  const token = statusToken(record);
  return isPartialTask(record) || (/success|done|complete|finish/.test(token) && !/fail|error|cancel/.test(token));
}

// 离线任务在云端异步落盘，整条链路没有目录失效回调；这里在轮询观察到任务
// 从"进行中"转为"完成"时，主动失效其目标目录，让文件列表和缓存及时更新。
const filesStore = useFilesStore();
const observedTaskPhases = new Map();
function reconcileCompletedOfflineTasks(tasks) {
  const completedParents = new Set();
  for (const record of tasks) {
    const id = String(taskId(record) || '');
    if (!id) continue;
    const phase = isTerminalTask(record) ? 'terminal' : 'running';
    const previous = observedTaskPhases.get(id);
    observedTaskPhases.set(id, phase);
    if (previous === 'running' && phase === 'terminal' && isCompletedTask(record)) {
      completedParents.add(String(pick(record, ['parentId', 'parent_id', 'targetParentId'], '')));
    }
  }
  if (completedParents.size) {
    filesStore.handleDirectoryInvalidation({ parent_ids: [...completedParents] });
  }
}

function displayStatus(record) {
  if (record?.nameRestoreStatus === 'failed') return ['名称恢复失败', 'error'];
  if (record?.nameRestoreStatus === 'pending' && numericStatus(record) === 2) return ['恢复名称中', 'processing'];
  return isPartialTask(record) ? ['部分完成', 'warning'] : offlineStatus(record);
}

function offlineError(record) {
  return String(pick(record, ['nameRestoreError', 'errMsg', 'errorMsg', 'errorMessage', 'msg', 'message', 'error'], '') || '').trim();
}

const selectedRecords = computed(() => {
  const ids = new Set(selectedKeys.value.map(String));
  return offlineTasks.value.filter((record) => ids.has(String(taskId(record))));
});
const selectedRunning = computed(() => selectedRecords.value.filter(isRunningTask));
const selectedRetryable = computed(() => selectedRecords.value.filter(isRetryableTask));
const selectedTerminal = computed(() => selectedRecords.value.filter(isTerminalTask));
const rowSelection = computed(() => ({
  selectedRowKeys: selectedKeys.value,
  onChange: (keys) => { selectedKeys.value = keys; },
}));

function offlineSourceName(source) {
  const value = String(source || '').trim();
  if (/^magnet:\?/i.test(value)) {
    try {
      return String(new URLSearchParams(value.slice(value.indexOf('?') + 1)).get('dn') || '').trim();
    } catch {
      return '';
    }
  }
  if (/^ed2k:\/\/\|file\|/i.test(value)) {
    const encoded = value.split('|')[2] || '';
    try { return decodeURIComponent(encoded.replaceAll('+', '%20')).trim(); }
    catch { return encoded.trim(); }
  }
  return '';
}

const resolvedResource = computed(() => {
  const data = unwrapData(offlineForm.resolved);
  const info = data.urlResInfo || data.btResInfo || data.emuleResInfo || data.resourceInfo || data;
  return info && typeof info === 'object' ? info : {};
});
const resolvedResourceOriginalName = computed(() => String(pick(resolvedResource.value, ['fileName', 'name', 'title'], '') || '').trim()
  || offlineSourceName(offlineForm.url));
const resolvedResourceName = computed(() => resolvedResourceOriginalName.value || '资源已识别');
const resolvedResourceSize = computed(() => Number(pick(resolvedResource.value, ['totalSize', 'fileSize', 'size'], 0)) || 0);
const shouldObfuscateCurrentSource = computed(() => offlineObfuscationEnabled.value
  && /^(?:magnet:|ed2k:\/\/)/i.test(offlineForm.url.trim()));
const statisticsSummary = computed(() => {
  const data = unwrapData(offlineStatistics.value);
  const total = Number(pick(data, ['totalTimes', 'total_times'], 0));
  const used = Number(pick(data, ['createTimes', 'create_times'], 0));
  const remaining = Number(pick(data, ['remainingTimes', 'remaining_times'], Math.max(total - used, 0)));
  if (!total && !used && !remaining) return '';
  return `今日已用 ${used} 次 · 剩余 ${remaining} 次${total ? ` · 共 ${total} 次` : ''}`;
});

function collectResolvedFileIndexes(resolved) {
  const btInfo = resolved?.btResInfo;
  if (!btInfo || !Array.isArray(btInfo.subfiles)) return [];
  const excluded = new Set((Array.isArray(btInfo.excludeIndices) ? btInfo.excludeIndices : []).map(Number));
  const indexes = [];
  const visit = (items) => {
    for (const item of items || []) {
      if (Array.isArray(item?.subfiles) && item.subfiles.length) visit(item.subfiles);
      const index = Number(item?.fileIndex);
      if (item?.isDir !== true && Number.isSafeInteger(index) && index >= 0 && !excluded.has(index)) indexes.push(index);
    }
  };
  visit(btInfo.subfiles);
  return [...new Set(indexes)];
}

function resetResolvedResource() {
  offlineForm.resolvedFor = '';
  offlineForm.resolved = null;
  offlineForm.resolveError = '';
}

async function resolveOfflineResource({ showMessage = true } = {}) {
  const url = offlineForm.url.trim();
  if (!url) {
    if (showMessage) message.warning('请先输入下载链接');
    return null;
  }
  if (offlineForm.resolvedFor === url && offlineForm.resolved) return offlineForm.resolved;
  offlineForm.resolving = true;
  offlineForm.resolveError = '';
  try {
    const result = await bridge.invoke('resolve_offline_resource', { url });
    offlineForm.resolved = result;
    offlineForm.resolvedFor = url;
    return result;
  } catch (error) {
    offlineForm.resolveError = errorText(error);
    if (showMessage) message.error(offlineForm.resolveError);
    throw error;
  } finally {
    offlineForm.resolving = false;
  }
}

async function loadOffline(cursor = currentCursor.value, { remember = false, preferredFocus = '' } = {}) {
  if (!appState.logged_in) return;
  const normalizedCursor = String(cursor || '');
  offlineLoading.value = true;
  offlineLoadError.value = '';
  try {
    const data = unwrapData(await bridge.invoke('list_offline_tasks', { cursor: normalizedCursor, page_size: pageSize }));
    const list = data.list || data.taskList || data.tasks || data.items || [];
    offlineTasks.value = Array.isArray(list) ? list : [];
    reconcileCompletedOfflineTasks(offlineTasks.value);
    offlineTotal.value = Number(data.total ?? data.totalCount ?? offlineTasks.value.length) || 0;
    nextCursor.value = String(data.cursor ?? data.nextCursor ?? data.next_cursor ?? '');
    hasMore.value = data.hasMore === true || data.has_more === true;
    if (remember && normalizedCursor !== currentCursor.value) cursorHistory.value.push(currentCursor.value);
    currentCursor.value = normalizedCursor;
    selectedKeys.value = [];
    const visible = offlineTasks.value.map(taskId).filter(Boolean).map(String);
    focusedTaskId.value = visible.includes(String(preferredFocus)) ? String(preferredFocus) : (visible[0] || '');
    await restoreTaskFocus();
  } catch (error) {
    offlineLoadError.value = errorText(error);
    message.error(offlineLoadError.value);
  } finally {
    offlineLoading.value = false;
  }
}

async function loadOfflineStatistics() {
  if (!appState.logged_in) return;
  try {
    offlineStatistics.value = await bridge.invoke('get_offline_statistics');
  } catch {
    // 统计不应阻止任务列表使用；列表错误仍由 loadOffline 单独呈现。
    offlineStatistics.value = null;
  }
}

async function loadOfflineSettings() {
  try {
    const data = unwrapData(await bridge.invoke('get_offline_settings'));
    offlineObfuscationEnabled.value = data.filename_obfuscation_enabled === true;
  } catch {
    offlineObfuscationEnabled.value = false;
  }
}

async function refreshOffline(cursor = currentCursor.value) {
  await Promise.all([loadOffline(cursor), loadOfflineStatistics()]);
}

function nextOfflinePage() {
  if (!hasMore.value || !nextCursor.value) return;
  void loadOffline(nextCursor.value, { remember: true });
}

function previousOfflinePage() {
  const cursor = cursorHistory.value.pop();
  if (cursor === undefined) return;
  void loadOffline(cursor);
}

async function restoreTaskFocus() {
  await nextTick();
  if (!focusedTaskId.value) return;
  document.querySelector(`[data-offline-id="${CSS.escape(focusedTaskId.value)}"]`)?.focus?.({ preventScroll: true });
}

async function submitOffline() {
  if (!offlineForm.url.trim()) {
    message.warning('请输入下载链接');
    return;
  }
  offlineSubmitting.value = true;
  try {
    const protectedSubmission = shouldObfuscateCurrentSource.value;
    const resolved = protectedSubmission ? {} : unwrapData(await resolveOfflineResource({ showMessage: false }));
    const info = resolved.urlResInfo || resolved.btResInfo || resolved.emuleResInfo || resolved.resourceInfo || resolved;
    const resolvedUrl = String(pick(resolved, ['url'], '') || pick(info, ['url', 'downloadUrl', 'download_url', 'resourceUrl'], '') || '').trim();
    const submittedSource = resolvedUrl || offlineForm.url.trim();
    const shouldRestoreName = offlineObfuscationEnabled.value && /^(?:magnet:|ed2k:\/\/)/i.test(submittedSource);
    const restoreName = offlineForm.newName.trim() || resolvedResourceOriginalName.value;
    if (shouldRestoreName && !restoreName) throw new Error('链接中没有可识别的原名称，请填写“恢复名称”后再提交');
    const fileIndexes = collectResolvedFileIndexes(resolved);
    await bridge.invoke('create_offline_task', {
      url: submittedSource,
      parent_id: offlineForm.targetId,
      new_name: offlineForm.newName.trim() || undefined,
      restore_name: restoreName || undefined,
      file_indexes: fileIndexes.length ? fileIndexes : undefined,
    });
    offlineForm.url = '';
    offlineForm.newName = '';
    offlineForm.open = false;
    resetResolvedResource();
    cursorHistory.value = [];
    currentCursor.value = '';
    await refreshOffline('');
    message.success(shouldRestoreName ? '离线任务已创建，将在完成后自动恢复原名称' : '离线任务已创建');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    offlineSubmitting.value = false;
  }
}

function openOfflineForm() {
  Object.assign(offlineForm, {
    open: true,
    url: '',
    newName: '',
    targetId: '',
    targetLabel: '全部文件',
    pickerOpen: false,
    resolving: false,
    resolvedFor: '',
    resolved: null,
    resolveError: '',
  });
}

function selectTarget(target) {
  offlineForm.targetId = target.id;
  offlineForm.targetLabel = target.label;
}

async function invokeTaskAction(command, records, successText) {
  const ids = [...new Set(records.map(taskId).filter(Boolean))];
  if (!ids.length) {
    message.error('任务缺少可操作的 taskId，请刷新后重试');
    return;
  }
  const index = offlineTasks.value.findIndex((record) => String(taskId(record)) === String(ids[0]));
  const preferred = taskId(offlineTasks.value[index + 1] || offlineTasks.value[index - 1]);
  offlineActionBusy.value = true;
  try {
    await bridge.invoke(command, { task_ids: ids });
    let targetCursor = currentCursor.value;
    if (offlineTasks.value.length <= ids.length && cursorHistory.value.length) targetCursor = cursorHistory.value.pop();
    await loadOffline(targetCursor, { preferredFocus: preferred });
    message.success(successText.replace('{count}', String(ids.length)));
  } catch (error) {
    message.error(errorText(error));
    throw error;
  } finally {
    offlineActionBusy.value = false;
  }
}

function confirmDeleteTasks(records, mode) {
  if (!records.length) return;
  const canceling = mode === 'cancel';
  Modal.confirm({
    title: canceling ? '取消离线任务' : '清理任务记录',
    content: canceling
      ? `确定取消选中的 ${records.length} 个运行中任务吗？`
      : `确定清理选中的 ${records.length} 条终态记录吗？`,
    okText: canceling ? '取消任务' : '清理记录',
    okButtonProps: { danger: true },
    cancelText: '关闭',
    async onOk() {
      // 两个语义命令在后端都映射到官方 PC 使用的 v2/delete_task。
      await invokeTaskAction(canceling ? 'cancel_offline_tasks' : 'delete_offline_tasks', records, canceling ? '已取消 {count} 个任务' : '已清理 {count} 条记录');
    },
  });
}

function retryTasks(records) {
  if (!records.length) return;
  void invokeTaskAction('retry_offline_tasks', records, '已重试 {count} 个任务').catch(() => {});
}

function taskRowProps(record, index) {
  const id = String(taskId(record));
  return {
    tabindex: focusedTaskId.value ? (focusedTaskId.value === id ? 0 : -1) : (index === 0 ? 0 : -1),
    'data-offline-id': id,
    'aria-selected': selectedKeys.value.map(String).includes(id),
    onFocus: () => { focusedTaskId.value = id; },
    onKeydown: (event) => {
      if (event.key === ' ') {
        event.preventDefault();
        const selected = new Set(selectedKeys.value.map(String));
        if (selected.has(id)) selected.delete(id); else selected.add(id);
        selectedKeys.value = offlineTasks.value.filter((item) => selected.has(String(taskId(item)))).map(taskId);
      }
      if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      const current = offlineTasks.value.findIndex((item) => String(taskId(item)) === id);
      const target = event.key === 'Home' ? 0 : event.key === 'End' ? offlineTasks.value.length - 1 : current + (event.key === 'ArrowUp' ? -1 : 1);
      const next = offlineTasks.value[Math.max(0, Math.min(offlineTasks.value.length - 1, target))];
      if (!next) return;
      focusedTaskId.value = String(taskId(next));
      void restoreTaskFocus();
    },
  };
}

defineExpose({ loadOffline });
onMounted(() => {
  void Promise.all([refreshOffline(), loadOfflineSettings()]);
  offlinePollTimer = window.setInterval(() => {
    if (offlineLoading.value || offlineActionBusy.value || offlineForm.open || selectedKeys.value.length) return;
    if (!offlineTasks.value.some((record) => isRunningTask(record) || record?.nameRestoreStatus === 'pending' || record?.nameRestoreStatus === 'failed')) return;
    void refreshOffline();
  }, 5_000);
});
onBeforeUnmount(() => {
  if (offlinePollTimer !== null) window.clearInterval(offlinePollTimer);
  offlinePollTimer = null;
});
</script>

<template>
  <div class="view-section">
    <PageHeader title="云添加" description="解析磁力 / HTTP / ED2K 资源并直接离线下载到云盘目录。" />
    <div class="plain-toolbar">
      <div v-if="selectedKeys.length" class="offline-selection" role="status">
        已选 {{ selectedKeys.length }} 项
        <a-button type="link" size="small" @click="selectedKeys = []">取消选择</a-button>
      </div>
      <div v-else class="offline-summary">
        <strong>云添加</strong>
        <span>{{ statisticsSummary || '支持 HTTP、磁力和 ED2K 资源' }}</span>
      </div>
      <a-space wrap>
        <a-button v-if="selectedRunning.length" danger :disabled="offlineActionBusy" @click="confirmDeleteTasks(selectedRunning, 'cancel')"><template #icon><StopOutlined /></template>取消任务 ({{ selectedRunning.length }})</a-button>
        <a-button v-if="selectedRetryable.length" :loading="offlineActionBusy" @click="retryTasks(selectedRetryable)"><template #icon><RedoOutlined /></template>重试 ({{ selectedRetryable.length }})</a-button>
        <a-button v-if="selectedTerminal.length" danger :disabled="offlineActionBusy" @click="confirmDeleteTasks(selectedTerminal, 'cleanup')"><template #icon><DeleteOutlined /></template>清理记录 ({{ selectedTerminal.length }})</a-button>
        <a-button :loading="offlineLoading" :disabled="offlineActionBusy" @click="refreshOffline()"><template #icon><ReloadOutlined /></template>刷新</a-button>
        <a-button type="primary" @click="openOfflineForm"><template #icon><DownloadOutlined /></template>新建任务</a-button>
      </a-space>
    </div>

    <a-alert v-if="offlineLoadError" class="offline-alert" type="error" show-icon :message="offlineLoadError">
      <template #action><a-button size="small" @click="loadOffline()">重试</a-button></template>
    </a-alert>
    <div class="table-filter-bar">
      <a-segmented v-model:value="offlineFilters.status" :options="offlineStatusFilterOptions" aria-label="按状态筛选离线任务" />
      <a-input v-model:value="offlineFilters.keyword" allow-clear placeholder="搜索任务名 / 错误信息" class="filter-keyword" aria-label="搜索离线任务">
        <template #prefix><SearchOutlined /></template>
      </a-input>
      <span v-if="offlineFilters.status !== 'all' || offlineFilters.keyword" class="filter-count">{{ filteredOfflineTasks.length }} / {{ offlineTasks.length }} 条</span>
    </div>
    <a-table
      :columns="offlineColumns"
      :data-source="filteredOfflineTasks"
      :loading="offlineLoading"
      :row-key="taskId"
      :row-selection="rowSelection"
      :on-row="taskRowProps"
      :pagination="false"
      :scroll="{ x: 1050 }"
      size="small"
    >
      <template #emptyText><a-empty description="暂无离线任务" /></template>
      <template #bodyCell="{ column, record }">
        <template v-if="column.key === 'name'"><a-flex align="center" gap="small"><a-avatar class="list-avatar"><InboxOutlined /></a-avatar><strong class="offline-name">{{ pick(record, ['fileName', 'name', 'taskName', 'title'], '离线任务') }}</strong></a-flex></template>
        <template v-else-if="column.key === 'size'">{{ formatSize(pick(record, ['totalSize', 'fileSize', 'size'], 0)) }}</template>
        <template v-else-if="column.key === 'status'"><a-tag :color="displayStatus(record)[1]">{{ displayStatus(record)[0] }}</a-tag></template>
        <template v-else-if="column.key === 'progress'">
          <a-progress v-if="offlineProgress(record) !== null" :percent="Math.round(offlineProgress(record))" :status="isRunningTask(record) ? 'active' : (isRetryableTask(record) ? 'exception' : 'normal')" size="small" />
          <span v-else>—</span>
        </template>
        <template v-else-if="column.key === 'error'"><span class="offline-error" :title="offlineError(record)">{{ offlineError(record) || '—' }}</span></template>
        <template v-else-if="column.key === 'actions'">
          <a-space :size="4">
            <a-button v-if="isRunningTask(record)" size="small" type="link" danger :disabled="offlineActionBusy" @click="confirmDeleteTasks([record], 'cancel')">取消任务</a-button>
            <a-button v-if="isRetryableTask(record)" size="small" type="link" :disabled="offlineActionBusy" @click="retryTasks([record])">重试</a-button>
            <a-button v-if="isTerminalTask(record)" size="small" type="link" danger :disabled="offlineActionBusy" @click="confirmDeleteTasks([record], 'cleanup')">清理</a-button>
          </a-space>
        </template>
      </template>
    </a-table>

    <div class="offline-footer">
      <span>本页 {{ offlineTasks.length }} 个任务<span v-if="offlineTotal"> · 共 {{ offlineTotal }} 个</span></span>
      <a-space>
        <a-button size="small" :disabled="offlineLoading || !cursorHistory.length" @click="previousOfflinePage">上一页</a-button>
        <a-button size="small" :disabled="offlineLoading || !hasMore || !nextCursor" @click="nextOfflinePage">下一页</a-button>
      </a-space>
    </div>

    <a-modal v-model:open="offlineForm.open" title="新建离线任务" :confirm-loading="offlineSubmitting || offlineForm.resolving" :ok-text="shouldObfuscateCurrentSource ? '保护名称并开始下载' : '解析并开始下载'" cancel-text="取消" @ok="submitOffline">
      <a-form layout="vertical" @submit.prevent="submitOffline">
        <a-form-item label="下载链接">
          <a-textarea v-model:value="offlineForm.url" aria-label="离线下载链接" :rows="3" placeholder="支持 HTTP/HTTPS、磁力和 ED2K 链接" @input="resetResolvedResource" @blur="offlineForm.url.trim() && !shouldObfuscateCurrentSource && resolveOfflineResource({ showMessage: false }).catch(() => {})" />
          <div class="resolve-line">
            <span v-if="shouldObfuscateCurrentSource">保护模式跳过云端预解析，默认保存全部文件</span>
            <template v-else>
              <span>提交前会先由光鸭识别资源</span>
              <a-button type="link" size="small" :loading="offlineForm.resolving" @click="resolveOfflineResource().catch(() => {})">立即解析</a-button>
            </template>
          </div>
        </a-form-item>
        <a-alert v-if="offlineForm.resolveError && !shouldObfuscateCurrentSource" type="error" show-icon :message="offlineForm.resolveError" />
        <a-alert v-else-if="offlineForm.resolved" type="success" show-icon message="资源解析完成">
          <template #description>
            <strong>{{ resolvedResourceName }}</strong><span v-if="resolvedResourceSize"> · {{ formatSize(resolvedResourceSize) }}</span>
          </template>
        </a-alert>
        <a-alert
          v-if="shouldObfuscateCurrentSource"
          class="obfuscation-alert"
          type="info"
          show-icon
          message="已启用离线文件名混淆"
          :description="resolvedResourceOriginalName
            ? `将以随机安全名称提交，成功后恢复为：${resolvedResourceOriginalName}`
            : '将以随机安全名称提交；请在下方填写任务成功后需要恢复的名称。'"
        />
        <a-form-item label="保存目录">
          <a-input :value="offlineForm.targetLabel" aria-label="离线任务保存目录" readonly @click="offlineForm.pickerOpen = true">
            <template #suffix><FolderOpenOutlined /></template>
          </a-input>
        </a-form-item>
        <a-form-item :label="shouldObfuscateCurrentSource ? '恢复名称' : '重命名（可选）'">
          <a-input v-model:value="offlineForm.newName" aria-label="离线任务新名称" maxlength="255" :placeholder="shouldObfuscateCurrentSource ? '留空则使用链接中的原名称' : '留空则使用资源原名称'" @press-enter="submitOffline" />
        </a-form-item>
      </a-form>
    </a-modal>
    <CloudFolderPicker v-model:open="offlineForm.pickerOpen" title="选择离线下载保存目录" @select="selectTarget" />
  </div>
</template>

<style scoped>
.offline-alert { margin-bottom: 10px; }
.offline-selection { color: var(--text-2, #525252); }
.offline-summary strong, .offline-summary span { display: block; }
.offline-summary span { margin-top: 2px; color: var(--text-3, #737373); font-size: 12px; }
.offline-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.offline-error { display: block; overflow: hidden; color: var(--danger, #ef4444); font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }
.offline-footer { display: flex; min-height: 42px; align-items: center; justify-content: space-between; color: var(--text-3, #737373); font-size: 12px; }
.resolve-line { display: flex; align-items: center; justify-content: space-between; color: var(--text-3, #737373); font-size: 12px; }
.obfuscation-alert { margin-top: 12px; }
</style>
