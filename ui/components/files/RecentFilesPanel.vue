<script setup>
import { computed, nextTick, onMounted, ref } from 'vue';
import { message } from 'antdv-next';
import { EyeOutlined, FileOutlined, FolderOutlined, ReloadOutlined } from '@antdv-next/icons';
import { bridge } from '../../bridge.js';
import { errorText, fileId, formatSize, formatTime, isFolder, pick, unwrapData } from '../../formatters.js';
import FileDetailsDrawer from './FileDetailsDrawer.vue';

const actions = ref([]);
const loading = ref(false);
const loadError = ref('');
const total = ref(0);
const hasMore = ref(false);
const nextCursor = ref('');
const cursorHistory = ref([]);
const currentCursor = ref('');
const focusedRowKey = ref('');
const detailsOpen = ref(false);
const detailsRecord = ref(null);

const rows = computed(() => actions.value.flatMap((action, actionIndex) => {
  const details = Array.isArray(action.actionDetails) ? action.actionDetails : [];
  return details.map((record, detailIndex) => ({
    ...record,
    _rowKey: `${action.collectionId || action.id || actionIndex}-${fileId(record) || detailIndex}`,
    _actionTime: action.ctime,
  }));
}));
const columns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '位置', key: 'location', ellipsis: true, width: 220 },
  { title: '大小', key: 'size', width: 110 },
  { title: '最近活动', key: 'time', width: 170 },
  { title: '操作', key: 'actions', width: 80 },
];

async function loadRecent(cursor = '', remember = false) {
  loading.value = true;
  loadError.value = '';
  try {
    const data = unwrapData(await bridge.invoke('list_recent_actions', { cursor, page_size: 50 }));
    const list = data.list || data.actions || data.items || [];
    actions.value = Array.isArray(list) ? list : [];
    total.value = Number(data.total ?? actions.value.length) || 0;
    hasMore.value = data.hasMore === true || data.has_more === true;
    nextCursor.value = String(data.cursor || data.nextCursor || data.next_cursor || '');
    if (remember && cursor !== currentCursor.value) cursorHistory.value.push(currentCursor.value);
    currentCursor.value = cursor;
    focusedRowKey.value = String(rows.value[0]?._rowKey || '');
  } catch (error) {
    loadError.value = errorText(error);
    message.error(loadError.value);
  } finally {
    loading.value = false;
  }
}

function nextPage() {
  if (!hasMore.value || !nextCursor.value) return;
  void loadRecent(nextCursor.value, true);
}

function previousPage() {
  const cursor = cursorHistory.value.pop();
  if (cursor === undefined) return;
  void loadRecent(cursor, false);
}

function openDetails(record) {
  detailsRecord.value = record;
  detailsOpen.value = true;
}

async function restoreRowFocus() {
  await nextTick();
  document.querySelector(`[data-recent-row="${CSS.escape(focusedRowKey.value)}"]`)?.focus?.();
}

function rowProps(record, index) {
  return {
    tabindex: focusedRowKey.value ? (focusedRowKey.value === record._rowKey ? 0 : -1) : (index === 0 ? 0 : -1),
    'data-recent-row': record._rowKey,
    onFocus: () => { focusedRowKey.value = record._rowKey; },
    onKeydown: (event) => {
      if (event.key === 'Enter') {
        event.preventDefault();
        openDetails(record);
      }
      if (!['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) return;
      event.preventDefault();
      const current = rows.value.findIndex((item) => item._rowKey === record._rowKey);
      const target = event.key === 'Home' ? 0 : event.key === 'End' ? rows.value.length - 1 : current + (event.key === 'ArrowUp' ? -1 : 1);
      const next = rows.value[Math.max(0, Math.min(rows.value.length - 1, target))];
      if (!next) return;
      focusedRowKey.value = next._rowKey;
      void nextTick(() => document.querySelector(`[data-recent-row="${CSS.escape(next._rowKey)}"]`)?.focus?.());
    },
  };
}

onMounted(() => loadRecent());
</script>

<template>
  <section class="files-panel" aria-label="云端最近文件">
    <div class="plain-toolbar">
      <div class="panel-summary">
        <strong>云端最近</strong>
        <span>按光鸭云端行为记录展示，共 {{ total }} 组活动</span>
      </div>
      <a-button :loading="loading" aria-label="刷新云端最近" @click="loadRecent(currentCursor)">
        <template #icon><ReloadOutlined /></template>刷新
      </a-button>
    </div>

    <a-alert v-if="loadError" class="panel-alert" type="error" show-icon :message="loadError">
      <template #action><a-button size="small" @click="loadRecent(currentCursor)">重试</a-button></template>
    </a-alert>
    <a-table
      :columns="columns"
      :data-source="rows"
      :loading="loading"
      :row-key="(record) => record._rowKey"
      :on-row="rowProps"
      :pagination="false"
      size="small"
    >
      <template #emptyText><a-empty description="暂无云端最近记录" /></template>
      <template #bodyCell="{ column, record }">
        <template v-if="column.key === 'name'">
          <a-flex align="center" gap="small">
            <a-avatar class="list-avatar"><FolderOutlined v-if="isFolder(record)" /><FileOutlined v-else /></a-avatar>
            <strong class="recent-name" :title="record.fileName">{{ record.fileName || '未命名文件' }}</strong>
          </a-flex>
        </template>
        <template v-else-if="column.key === 'location'">{{ record.parentName || '全部文件' }}</template>
        <template v-else-if="column.key === 'size'">{{ isFolder(record) ? '—' : formatSize(record.fileSize) }}</template>
        <template v-else-if="column.key === 'time'">{{ formatTime(record._actionTime || record.utime) }}</template>
        <template v-else-if="column.key === 'actions'">
          <a-button size="small" type="text" aria-label="查看文件详情" @click="openDetails(record)"><template #icon><EyeOutlined /></template></a-button>
        </template>
      </template>
    </a-table>

    <div class="panel-footer">
      <span>本页 {{ rows.length }} 个文件</span>
      <a-space>
        <a-button size="small" :disabled="loading || !cursorHistory.length" @click="previousPage">上一页</a-button>
        <a-button size="small" :disabled="loading || !hasMore || !nextCursor" @click="nextPage">下一页</a-button>
      </a-space>
    </div>

    <FileDetailsDrawer v-model:open="detailsOpen" :record="detailsRecord" @closed="restoreRowFocus" />
  </section>
</template>

<style scoped>
.files-panel { min-height: 0; }
.panel-summary strong, .panel-summary span { display: block; }
.panel-summary span { margin-top: 2px; color: var(--text-3, #737373); font-size: 12px; }
.panel-alert { margin-bottom: 10px; }
.recent-name { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.panel-footer { display: flex; min-height: 42px; align-items: center; justify-content: space-between; color: var(--text-3, #737373); font-size: 12px; }
</style>
