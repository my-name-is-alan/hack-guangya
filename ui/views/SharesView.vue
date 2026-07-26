<script setup>
import { computed, onMounted, ref } from 'vue';
import { message, Modal } from 'antdv-next';
import {
  CloudOutlined,
  CopyOutlined,
  DeleteOutlined,
  LinkOutlined,
  ReloadOutlined,
  SearchOutlined,
} from '@antdv-next/icons';
import { bridge } from '../bridge.js';
import FavoriteLinkDialog from '../components/shares/FavoriteLinkDialog.vue';
import ReceiveShareDialog from '../components/shares/ReceiveShareDialog.vue';
import { parseGuangyaShareLink } from '../shareLink.js';
import { shareDisplayName } from '../shareRecord.js';
import { appState, refreshState } from '../store.js';
import { cloudShareStatus, copyText, errorText, formatSize, formatTime, isFolder, pick, unwrapData } from '../formatters.js';

const shareTab = ref('cloud');
const cloudShares = ref([]);
const cloudSharesLoading = ref(false);
const cloudShareQuery = ref('');
const cloudSharePage = ref(1);
const cloudSharePageSize = ref(20);
const filteredCloudShares = computed(() => {
  const query = cloudShareQuery.value.trim().toLowerCase();
  if (!query) return cloudShares.value;
  return cloudShares.value.filter((record) => [
    shareDisplayName(record),
    record.shareUrl,
    record.share_url,
    record.code,
    record.id,
    record.shareId,
  ].some((value) => String(value ?? '').toLowerCase().includes(query)));
});
const cloudSharePagination = computed(() => ({
  current: cloudSharePage.value,
  pageSize: cloudSharePageSize.value,
  total: filteredCloudShares.value.length,
  hideOnSinglePage: false,
  showSizeChanger: true,
  pageSizeOptions: ['20', '50', '100'],
  showTotal: (total) => `共 ${total} 条分享`,
}));

const shareColumns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '状态', key: 'status', width: 100 },
  { title: '创建时间', key: 'time', width: 170 },
  { title: '操作', key: 'actions', width: 170 },
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
    cloudShares.value = data.list || data.shares || [];
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
    try {
      code = parseGuangyaShareLink(url).code;
    } catch {
      // 列表可能包含旧格式链接；解析失败时仍允许复制原始 URL。
    }
  }
  await copyText(code ? `${url} 提取码：${code}` : url, message);
}
async function deleteShare(record) {
  const id = record.id ?? record.shareId ?? record.share_id;
  if (id === undefined || id === null || id === '') {
    message.error('当前分享缺少可取消的记录 ID，请刷新后重试');
    return;
  }
  Modal.confirm({
    title: '取消分享',
    content: `确定取消「${shareDisplayName(record)}」吗？`,
    okText: '取消分享',
    okButtonProps: { danger: true },
    cancelText: '关闭',
    async onOk() {
      try {
        await bridge.invoke('delete_shares', { ids: [id] });
        await loadCloudShares();
        message.success('已取消分享');
      } catch (error) {
        message.error(errorText(error));
      }
    },
  });
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
    <a-tabs v-model:active-key="shareTab" class="page-tabs">
      <template #rightExtra>
        <a-space>
          <a-input v-if="shareTab === 'cloud'" v-model:value="cloudShareQuery" allow-clear placeholder="搜索分享" style="width: 220px" @change="cloudSharePage = 1">
            <template #prefix><SearchOutlined /></template>
          </a-input>
          <ReceiveShareDialog v-if="shareTab === 'cloud'" />
          <FavoriteLinkDialog v-else />
          <a-button :loading="cloudSharesLoading" @click="refreshActiveTab"><template #icon><ReloadOutlined /></template>刷新</a-button>
        </a-space>
      </template>
      <a-tab-pane key="cloud" tab="分享">
        <a-table :columns="shareColumns" :data-source="filteredCloudShares" :loading="cloudSharesLoading" :row-key="(item) => item.id || item.shareId" :pagination="cloudSharePagination" size="small" @change="handleCloudShareTableChange">
          <template #emptyText><a-empty :description="appState.logged_in ? '暂无分享' : '登录后查看分享'" /></template>
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'name'">
              <a-flex align="center" gap="small">
                <div class="file-icon" :class="isFolder(record) ? 'folder' : 'other'"><CloudOutlined /></div>
                <div class="file-name-wrap">
                  <span class="file-name">{{ shareDisplayName(record) }}</span>
                  <small v-if="record.fileSize">{{ formatSize(record.fileSize) }}</small>
                </div>
              </a-flex>
            </template>
            <template v-else-if="column.key === 'status'"><a-tag :color="cloudShareStatus(record)[1]">{{ cloudShareStatus(record)[0] }}</a-tag></template>
            <template v-else-if="column.key === 'time'">{{ formatTime(record.createTime || record.createdAt) }}</template>
            <template v-else-if="column.key === 'actions'">
              <a-space :size="4">
                <a-button size="small" type="text" aria-label="复制分享链接" @click="copyCloudShare(record)"><template #icon><CopyOutlined /></template></a-button>
                <a-button size="small" type="text" danger aria-label="取消分享" @click="deleteShare(record)"><template #icon><DeleteOutlined /></template></a-button>
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
                <div class="file-name-wrap">
                  <span class="file-name">{{ record.label || record.name || record.url }}</span>
                </div>
              </a-flex>
            </template>
            <template v-else-if="column.key === 'url'"><a :href="record.url" target="_blank" class="link-ellipsis">{{ record.url }}</a></template>
            <template v-else-if="column.key === 'actions'">
              <a-button size="small" type="text" danger aria-label="删除收藏" @click="removeShareLink(record)"><template #icon><DeleteOutlined /></template></a-button>
            </template>
          </template>
        </a-table>
      </a-tab-pane>
    </a-tabs>
  </div>
</template>
