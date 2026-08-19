<script setup>
import { computed, onMounted, reactive, ref } from 'vue';
import { message, Modal } from 'antdv-next';
import {
  CloudOutlined,
  CopyOutlined,
  DeleteOutlined,
  EditOutlined,
  LinkOutlined,
  ReloadOutlined,
  SearchOutlined,
} from '@antdv-next/icons';
import { bridge } from '../bridge.js';
import PageHeader from '../components/layout/PageHeader.vue';
import FavoriteLinkDialog from '../components/shares/FavoriteLinkDialog.vue';
import ReceiveShareDialog from '../components/shares/ReceiveShareDialog.vue';
import { parseGuangyaShareLink } from '../shareLink.js';
import { shareDisplayName } from '../shareRecord.js';
import { appState, refreshState } from '../store.js';
import { cloudShareStatus, copyText, errorText, formatSize, formatTime, isFolder, pick, unwrapData } from '../formatters.js';

const MAX_SHARE_TRAFFIC_BYTES = 1024n * (1024n ** 4n);
const trafficUnitOptions = [
  { label: 'B', value: 'B' },
  { label: 'MB', value: 'MB' },
  { label: 'GB', value: 'GB' },
  { label: 'TB', value: 'TB' },
];
const trafficUnitBytes = {
  B: 1n,
  MB: 1024n ** 2n,
  GB: 1024n ** 3n,
  TB: 1024n ** 4n,
};
const durationOptions = [
  { label: '永久有效', value: 0 },
  { label: '1 天', value: 86_400 },
  { label: '7 天', value: 604_800 },
  { label: '30 天', value: 2_592_000 },
];

const shareTab = ref('cloud');
const cloudShares = ref([]);
const cloudSharesLoading = ref(false);
const cloudShareActionBusy = ref(false);
const cloudShareQuery = ref('');
const cloudShareStatusFilter = ref('all');
const shareStatusFilterOptions = [
  { value: 'all', label: '全部状态' },
  { value: 'valid', label: '有效' },
  { value: 'invalid', label: '已失效' },
];
const cloudSharePage = ref(1);
const cloudSharePageSize = ref(20);
const selectedKeys = ref([]);
const editForm = reactive({
  open: false,
  saving: false,
  record: null,
  validateDuration: 604_800,
  downloadType: 0,
  trafficLimitValue: '0',
  trafficLimitUnit: 'GB',
});

function shareRecordId(record) {
  return String(pick(record, ['id', 'shareId', 'share_id'], '') || '');
}

function shareStatusValue(record) {
  const value = Number(pick(record, ['shareStatus', 'share_status', 'status'], 0));
  return Number.isFinite(value) ? value : 0;
}

function isInvalidShare(record) {
  return [2, 3, 4].includes(shareStatusValue(record));
}

function validityLabel(record) {
  if (Number(pick(record, ['validateDuration', 'validate_duration'], -1)) === 0) return '永久有效';
  if (isInvalidShare(record)) return cloudShareStatus(record)[0];
  const seconds = Number(pick(record, ['leftTime', 'left_time'], 0));
  if (seconds < 0) return '永久有效';
  if (!seconds) return '即将到期';
  const days = Math.floor(seconds / 86_400);
  const hours = Math.floor((seconds % 86_400) / 3_600);
  if (days) return hours ? `${days} 天 ${hours} 小时` : `${days} 天`;
  if (hours) return `${hours} 小时`;
  return `${Math.max(1, Math.ceil(seconds / 60))} 分钟`;
}

function shareCounts(record) {
  return {
    browse: Number(pick(record, ['browseCount', 'browse_count'], 0)) || 0,
    restore: Number(pick(record, ['restoreCount', 'restore_count'], 0)) || 0,
    download: Number(pick(record, ['downloadCount', 'download_count'], 0)) || 0,
  };
}

function trafficLabel(record) {
  const type = Number(pick(record, ['downloadType', 'download_type'], 0));
  if (type === 1) return '流量下载';
  const used = Number(pick(record, ['usedTraffic', 'used_traffic'], 0)) || 0;
  const limit = Number(pick(record, ['trafficLimit', 'traffic_limit'], 0)) || 0;
  return limit > 0 ? `免登录 · ${formatSize(used)} / ${formatSize(limit)}` : '免登录下载 · 不限额';
}

const filteredCloudShares = computed(() => {
  const query = cloudShareQuery.value.trim().toLowerCase();
  return cloudShares.value.filter((record) => {
    if (cloudShareStatusFilter.value === 'valid' && shareStatusValue(record) !== 1) return false;
    if (cloudShareStatusFilter.value === 'invalid' && !isInvalidShare(record)) return false;
    if (!query) return true;
    return [
      shareDisplayName(record),
      record.shareUrl,
      record.share_url,
      record.code,
      record.id,
      record.shareId,
    ].some((value) => String(value ?? '').toLowerCase().includes(query));
  });
});
const selectedRecords = computed(() => {
  const ids = new Set(selectedKeys.value.map(String));
  return cloudShares.value.filter((record) => ids.has(shareRecordId(record)));
});
const invalidShareCount = computed(() => cloudShares.value.filter(isInvalidShare).length);
const normalShareCount = computed(() => cloudShares.value.filter((record) => shareStatusValue(record) === 1).length);
const cloudSharePagination = computed(() => ({
  current: cloudSharePage.value,
  pageSize: cloudSharePageSize.value,
  total: filteredCloudShares.value.length,
  hideOnSinglePage: false,
  showSizeChanger: true,
  pageSizeOptions: ['20', '50', '100'],
  showTotal: (total) => `共 ${total} 条分享`,
}));
const rowSelection = computed(() => ({
  selectedRowKeys: selectedKeys.value,
  onChange: (keys) => { selectedKeys.value = keys; },
}));

const shareColumns = [
  { title: '名称', key: 'name', ellipsis: true, width: 260 },
  { title: '状态', key: 'status', width: 95 },
  { title: '有效期', key: 'validity', width: 130 },
  { title: '访问数据', key: 'counts', width: 190 },
  { title: '下载方式 / 流量', key: 'traffic', width: 190 },
  { title: '创建时间', key: 'time', width: 170 },
  { title: '操作', key: 'actions', width: 130, fixed: 'right' },
];
const linkColumns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '链接', key: 'url', ellipsis: true },
  { title: '操作', key: 'actions', width: 90 },
];

async function loadCloudShares() {
  if (!appState.logged_in) return;
  cloudSharesLoading.value = true;
  try {
    const data = unwrapData(await bridge.invoke('list_shares'));
    const list = data.list || data.shares || [];
    cloudShares.value = Array.isArray(list) ? list : [];
    selectedKeys.value = [];
    cloudSharePage.value = 1;
  } catch (error) {
    message.error(errorText(error));
  } finally {
    cloudSharesLoading.value = false;
  }
}

async function refreshActiveTab() {
  if (shareTab.value === 'cloud') await loadCloudShares();
  else await refreshState();
}

async function copyCloudShare(record) {
  const url = String(pick(record, ['shareUrl', 'share_url', 'url'], '')).trim();
  if (!url) {
    message.warning('分享链接不可用');
    return;
  }
  let code = String(pick(record, ['code', 'extractCode'], '')).trim();
  if (!code) {
    try { code = parseGuangyaShareLink(url).code; } catch { /* 旧链接仍复制原 URL。 */ }
  }
  await copyText(code ? `${url} 提取码：${code}` : url, message);
}

async function deleteShareRecords(records) {
  const ids = [...new Set(records.map(shareRecordId).filter(Boolean))];
  if (!ids.length) throw new Error('当前分享缺少记录 ID，请刷新后重试');
  cloudShareActionBusy.value = true;
  try {
    await bridge.invoke('delete_shares', { ids });
    await loadCloudShares();
    message.success(`已取消 ${ids.length} 条分享`);
  } finally {
    cloudShareActionBusy.value = false;
  }
}

function confirmDeleteShares(records) {
  const targets = (Array.isArray(records) ? records : []).filter(Boolean);
  if (!targets.length) return;
  Modal.confirm({
    title: targets.length === 1 ? '取消分享' : `批量取消 ${targets.length} 条分享`,
    content: targets.length === 1
      ? `确定取消「${shareDisplayName(targets[0])}」吗？原分享链接会立即失效。`
      : `选中的 ${targets.length} 条分享链接会立即失效，是否继续？`,
    okText: '取消分享',
    okButtonProps: { danger: true },
    cancelText: '关闭',
    async onOk() {
      try { await deleteShareRecords(targets); } catch (error) { message.error(errorText(error)); throw error; }
    },
  });
}

function confirmDeleteInvalidShares() {
  if (!invalidShareCount.value) return;
  Modal.confirm({
    title: '清理失效分享',
    content: `将删除账号下全部失效、已删除或被禁用的分享记录（当前列表 ${invalidShareCount.value} 条）。`,
    okText: '清理失效记录',
    okButtonProps: { danger: true },
    cancelText: '取消',
    async onOk() {
      cloudShareActionBusy.value = true;
      try {
        await bridge.invoke('delete_invalid_shares');
        await loadCloudShares();
        message.success('失效分享已清理');
      } catch (error) {
        message.error(errorText(error));
        throw error;
      } finally {
        cloudShareActionBusy.value = false;
      }
    },
  });
}

function openEditShare(record) {
  const duration = Number(pick(record, ['validateDuration', 'validate_duration'], 604_800));
  const rawTraffic = String(pick(record, ['trafficLimit', 'traffic_limit'], '0') || '0').trim();
  const trafficBytes = /^\d+$/.test(rawTraffic) ? BigInt(rawTraffic) : 0n;
  const unit = ['TB', 'GB', 'MB'].find((candidate) => trafficBytes > 0n && trafficBytes % trafficUnitBytes[candidate] === 0n) || (trafficBytes > 0n ? 'B' : 'GB');
  editForm.record = record;
  editForm.validateDuration = durationOptions.some((item) => item.value === duration) ? duration : 604_800;
  editForm.downloadType = Number(pick(record, ['downloadType', 'download_type'], 0)) === 1 ? 1 : 0;
  editForm.trafficLimitUnit = unit;
  editForm.trafficLimitValue = String(trafficBytes / trafficUnitBytes[unit]);
  editForm.open = true;
}

async function submitEditShare() {
  const id = shareRecordId(editForm.record);
  if (!id) {
    message.error('当前分享缺少记录 ID，请刷新后重试');
    return;
  }
  const rawTrafficValue = String(editForm.trafficLimitValue || '').trim();
  if (!/^\d+$/.test(rawTrafficValue)) {
    message.warning('流量上限必须是非负整数');
    return;
  }
  const unitBytes = trafficUnitBytes[editForm.trafficLimitUnit] || 1n;
  const trafficLimit = editForm.downloadType === 0 ? BigInt(rawTrafficValue) * unitBytes : 0n;
  if (trafficLimit > MAX_SHARE_TRAFFIC_BYTES) {
    message.warning('免登录流量上限最大为 1024 TB');
    return;
  }
  editForm.saving = true;
  try {
    await bridge.invoke('update_share', {
      id,
      validate_duration: editForm.validateDuration,
      download_type: editForm.downloadType,
      traffic_limit: trafficLimit.toString(),
    });
    editForm.open = false;
    await loadCloudShares();
    message.success('分享设置已更新');
  } catch (error) {
    message.error(errorText(error));
  } finally {
    editForm.saving = false;
  }
}

function handleCloudShareTableChange(pagination) {
  cloudSharePage.value = Number(pagination?.current || 1);
  cloudSharePageSize.value = Number(pagination?.pageSize || 20);
}

async function removeShareLink(record) {
  try {
    await bridge.invoke('remove_share_link', { id: record.id });
    await refreshState();
    message.success('已删除收藏');
  } catch (error) {
    message.error(errorText(error));
  }
}

defineExpose({ loadCloudShares });
onMounted(loadCloudShares);
</script>

<template>
  <div class="view-section">
    <PageHeader title="分享管理" description="管理分享链接的状态、有效期与流量，收藏常用分享。" />
    <a-tabs v-model:active-key="shareTab" class="page-tabs" :animated="false">
      <template #rightExtra>
        <a-space wrap>
          <a-select v-if="shareTab === 'cloud'" v-model:value="cloudShareStatusFilter" :options="shareStatusFilterOptions" aria-label="按状态筛选分享" class="share-status-filter" @change="cloudSharePage = 1" />
          <a-input v-if="shareTab === 'cloud'" v-model:value="cloudShareQuery" allow-clear aria-label="搜索分享" placeholder="搜索分享" class="share-search" @change="cloudSharePage = 1">
            <template #prefix><SearchOutlined /></template>
          </a-input>
          <ReceiveShareDialog v-if="shareTab === 'cloud'" />
          <FavoriteLinkDialog v-else />
          <a-button :loading="cloudSharesLoading" @click="refreshActiveTab"><template #icon><ReloadOutlined /></template>刷新</a-button>
        </a-space>
      </template>

      <a-tab-pane key="cloud" tab="分享">
        <div class="share-toolbar">
          <div v-if="selectedKeys.length" class="selection-summary" role="status">
            已选 {{ selectedKeys.length }} 条
            <a-button type="link" size="small" @click="selectedKeys = []">取消选择</a-button>
          </div>
          <a-space v-else wrap>
            <a-tag color="success">有效 {{ normalShareCount }}</a-tag>
            <a-tag :color="invalidShareCount ? 'warning' : 'default'">失效 {{ invalidShareCount }}</a-tag>
          </a-space>
          <a-space wrap>
            <a-button v-if="selectedKeys.length" danger :loading="cloudShareActionBusy" @click="confirmDeleteShares(selectedRecords)"><template #icon><DeleteOutlined /></template>批量取消</a-button>
            <a-button danger :disabled="!invalidShareCount || cloudShareActionBusy" @click="confirmDeleteInvalidShares">清理失效分享</a-button>
          </a-space>
        </div>

        <a-table
          :columns="shareColumns"
          :data-source="filteredCloudShares"
          :loading="cloudSharesLoading"
          :row-key="shareRecordId"
          :row-selection="rowSelection"
          :pagination="cloudSharePagination"
          :scroll="{ x: 1160 }"
          size="small"
          @change="handleCloudShareTableChange"
        >
          <template #emptyText><a-empty :description="appState.logged_in ? '暂无分享' : '登录后查看分享'" /></template>
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'name'">
              <a-flex align="center" gap="small">
                <div class="file-icon" :class="isFolder(record) ? 'folder' : 'other'"><CloudOutlined /></div>
                <div class="file-name-wrap">
                  <span class="file-name" :title="shareDisplayName(record)">{{ shareDisplayName(record) }}</span>
                  <small v-if="record.fileSize">{{ formatSize(record.fileSize) }}</small>
                </div>
              </a-flex>
            </template>
            <template v-else-if="column.key === 'status'"><a-tag :color="cloudShareStatus(record)[1]">{{ cloudShareStatus(record)[0] }}</a-tag></template>
            <template v-else-if="column.key === 'validity'">{{ validityLabel(record) }}</template>
            <template v-else-if="column.key === 'counts'">
              <span class="share-counts">浏览 {{ shareCounts(record).browse }} · 转存 {{ shareCounts(record).restore }} · 下载 {{ shareCounts(record).download }}</span>
            </template>
            <template v-else-if="column.key === 'traffic'">{{ trafficLabel(record) }}</template>
            <template v-else-if="column.key === 'time'">{{ formatTime(record.createTime || record.createdAt) }}</template>
            <template v-else-if="column.key === 'actions'">
              <a-space :size="4">
                <a-button size="small" type="text" aria-label="复制分享链接" @click="copyCloudShare(record)"><template #icon><CopyOutlined /></template></a-button>
                <a-button v-if="shareStatusValue(record) === 1" size="small" type="text" aria-label="编辑分享设置" @click="openEditShare(record)"><template #icon><EditOutlined /></template></a-button>
                <a-button size="small" type="text" danger aria-label="取消分享" @click="confirmDeleteShares([record])"><template #icon><DeleteOutlined /></template></a-button>
              </a-space>
            </template>
          </template>
        </a-table>
      </a-tab-pane>

      <a-tab-pane key="links" tab="收藏">
        <a-table :columns="linkColumns" :data-source="appState.share_links" :row-key="(item) => item.id" :pagination="false" size="small">
          <template #emptyText><a-empty description="暂无收藏的链接" /></template>
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'name'">
              <a-flex align="center" gap="small">
                <div class="file-icon other"><LinkOutlined /></div>
                <div class="file-name-wrap"><span class="file-name">{{ record.label || record.name || record.url }}</span></div>
              </a-flex>
            </template>
            <template v-else-if="column.key === 'url'"><a :href="record.url" target="_blank" rel="noreferrer" class="link-ellipsis">{{ record.url }}</a></template>
            <template v-else-if="column.key === 'actions'"><a-button size="small" type="text" danger aria-label="删除收藏" @click="removeShareLink(record)"><template #icon><DeleteOutlined /></template></a-button></template>
          </template>
        </a-table>
      </a-tab-pane>
    </a-tabs>

    <a-modal v-model:open="editForm.open" title="编辑分享设置" :confirm-loading="editForm.saving" ok-text="保存" cancel-text="取消" @ok="submitEditShare">
      <a-form layout="vertical" @submit.prevent="submitEditShare">
        <a-form-item label="有效期">
          <a-select v-model:value="editForm.validateDuration" aria-label="分享有效期" :options="durationOptions" />
        </a-form-item>
        <a-form-item label="下载方式">
          <a-radio-group v-model:value="editForm.downloadType" aria-label="分享下载方式">
            <a-radio :value="0">免登录下载</a-radio>
            <a-radio :value="1">流量下载</a-radio>
          </a-radio-group>
        </a-form-item>
        <a-form-item v-if="editForm.downloadType === 0" label="免登录流量上限">
          <a-space-compact block>
            <a-input v-model:value="editForm.trafficLimitValue" inputmode="numeric" aria-label="免登录流量上限数值" placeholder="0" />
            <a-select v-model:value="editForm.trafficLimitUnit" aria-label="免登录流量上限单位" :options="trafficUnitOptions" style="width: 100px" />
          </a-space-compact>
          <span class="form-help">只接受非负整数，0 表示不限额，最大 1024 TB；实际可用量仍受账号权益限制。</span>
        </a-form-item>
        <a-alert type="info" show-icon message="修改只影响这条分享，不会更改原文件。" />
      </a-form>
    </a-modal>
  </div>
</template>

<style scoped>
.share-search { width: 220px; }
.share-status-filter { width: 110px; }
.share-toolbar { display: flex; min-height: var(--toolbar-h, 42px); align-items: center; justify-content: space-between; gap: 16px; }
.selection-summary { color: var(--text-2, #525252); }
.share-counts { color: var(--text-2, #525252); font-size: 12px; white-space: nowrap; }
.form-help { display: block; margin-top: 6px; color: var(--text-3, #737373); font-size: 12px; }
@media (max-width: 900px) {
  .share-toolbar { align-items: flex-start; flex-direction: column; padding: 8px 0; }
  .share-search { width: 180px; }
}
</style>
