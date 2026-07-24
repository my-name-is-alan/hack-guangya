<script setup>
import { computed, reactive, ref } from 'vue';
import { message, Modal } from 'antdv-next';
import {
  CloudOutlined,
  CopyOutlined,
  DeleteOutlined,
  LinkOutlined,
  ReloadOutlined,
  ShareAltOutlined,
} from '@antdv-next/icons';
import { bridge } from '../bridge.js';
import { appState, refreshState } from '../store.js';
import { cloudShareStatus, copyText, errorText, fileId, formatSize, formatTime, isFolder, pick, unwrapData } from '../formatters.js';

const shareTab = ref('cloud');
const cloudShares = ref([]);
const cloudSharesLoading = ref(false);
const shareCreate = reactive({ open: false, loading: false, record: null, period: 1, password: '' });

const shareColumns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '状态', key: 'status', width: 100 },
  { title: '创建时间', key: 'time', width: 170 },
  { title: '操作', key: 'actions', width: 170 },
];
const linkColumns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '链接', key: 'url', ellipsis: true },
  { title: '备注', key: 'remark', ellipsis: true },
  { title: '操作', key: 'actions', width: 90 },
];

const shareExpireText = computed(() => ({ 0: '永久有效', 1: '1 天', 2: '7 天', 3: '30 天' }));

async function loadCloudShares() {
  if (!appState.logged_in) return;
  cloudSharesLoading.value = true;
  try {
    const data = unwrapData(await bridge.invoke('list_shares'));
    cloudShares.value = data.list || data.shares || [];
  } catch (error) {
    message.error(errorText(error));
  } finally {
    cloudSharesLoading.value = false;
  }
}

function openShareCreate(records) {
  const list = (Array.isArray(records) ? records : []).filter(Boolean);
  if (!list.length) {
    message.warning('请先在文件列表中选择要分享的文件');
    return;
  }
  shareCreate.record = list;
  shareCreate.period = 1;
  shareCreate.password = '';
  shareCreate.open = true;
}
async function submitShareCreate() {
  if (!shareCreate.record?.length) return;
  shareCreate.loading = true;
  try {
    const data = unwrapData(await bridge.invoke('create_share', {
      file_ids: shareCreate.record.map((item) => fileId(item)),
      period: shareCreate.period,
      password: shareCreate.password.trim(),
    }));
    const url = pick(data, ['shareUrl', 'share_url', 'url'], '');
    shareCreate.open = false;
    if (url) {
      await copyText(shareCreate.password ? `${url} 提取码：${shareCreate.password}` : url, message);
    }
    message.success('分享已创建');
    loadCloudShares();
  } catch (error) {
    message.error(errorText(error));
  } finally {
    shareCreate.loading = false;
  }
}
async function deleteShare(record) {
  Modal.confirm({
    title: '取消分享',
    content: `确定取消「${record.shareName || record.fileName || '该分享'}」吗？`,
    okText: '取消分享',
    okButtonProps: { danger: true },
    cancelText: '关闭',
    async onOk() {
      try {
        await bridge.invoke('delete_shares', { share_ids: [record.shareId || record.id] });
        await loadCloudShares();
        message.success('已取消分享');
      } catch (error) {
        message.error(errorText(error));
      }
    },
  });
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

defineExpose({ loadCloudShares, openShareCreate });
</script>

<template>
  <div class="view-section">
    <div class="section-toolbar">
      <div class="section-title">
        <div class="section-icon"><ShareAltOutlined /></div>
        <div><h2>分享管理</h2><p>管理我创建的分享与收藏的分享链接</p></div>
      </div>
      <a-button :loading="cloudSharesLoading" :disabled="!appState.logged_in" @click="loadCloudShares"><template #icon><ReloadOutlined /></template>刷新</a-button>
    </div>

    <a-tabs v-model:active-key="shareTab">
      <a-tab-pane key="cloud" tab="我的分享">
        <a-table :columns="shareColumns" :data-source="cloudShares" :loading="cloudSharesLoading" :row-key="(item) => item.shareId || item.id" :pagination="false" size="small">
          <template #emptyText><a-empty :description="appState.logged_in ? '暂无分享' : '登录后查看分享'" /></template>
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'name'">
              <a-flex align="center" gap="small">
                <div class="file-icon" :class="isFolder(record) ? 'folder' : 'other'"><CloudOutlined /></div>
                <div class="file-name-wrap">
                  <span class="file-name">{{ record.shareName || record.fileName || '分享' }}</span>
                  <small v-if="record.fileSize">{{ formatSize(record.fileSize) }}</small>
                </div>
              </a-flex>
            </template>
            <template v-else-if="column.key === 'status'"><a-tag :color="cloudShareStatus(record)[1]">{{ cloudShareStatus(record)[0] }}</a-tag></template>
            <template v-else-if="column.key === 'time'">{{ formatTime(record.createTime || record.createdAt) }}</template>
            <template v-else-if="column.key === 'actions'">
              <a-space :size="4">
                <a-button size="small" type="text" @click="copyText(record.shareUrl || record.url, message)"><template #icon><CopyOutlined /></template></a-button>
                <a-button size="small" type="text" danger @click="deleteShare(record)"><template #icon><DeleteOutlined /></template></a-button>
              </a-space>
            </template>
          </template>
        </a-table>
      </a-tab-pane>
      <a-tab-pane key="links" tab="收藏的链接">
        <a-table :columns="linkColumns" :data-source="appState.share_links" :row-key="(item) => item.id" :pagination="false" size="small">
          <template #emptyText><a-empty description="暂无收藏的链接" /></template>
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'name'">
              <a-flex align="center" gap="small">
                <div class="file-icon other"><LinkOutlined /></div>
                <div class="file-name-wrap">
                  <span class="file-name">{{ record.name || record.url }}</span>
                  <small v-if="record.password">提取码：{{ record.password }}</small>
                </div>
              </a-flex>
            </template>
            <template v-else-if="column.key === 'url'"><a :href="record.url" target="_blank" class="link-ellipsis">{{ record.url }}</a></template>
            <template v-else-if="column.key === 'remark'">{{ record.remark || '—' }}</template>
            <template v-else-if="column.key === 'actions'">
              <a-button size="small" type="text" danger @click="removeShareLink(record)"><template #icon><DeleteOutlined /></template></a-button>
            </template>
          </template>
        </a-table>
      </a-tab-pane>
    </a-tabs>

    <a-modal v-model:open="shareCreate.open" title="创建分享" :confirm-loading="shareCreate.loading" ok-text="创建并复制链接" cancel-text="取消" @ok="submitShareCreate">
      <a-form layout="vertical">
        <a-form-item label="分享内容">
          <div class="share-create-files">
            <a-tag v-for="item in shareCreate.record || []" :key="fileId(item)">{{ item.fileName }}</a-tag>
          </div>
        </a-form-item>
        <a-form-item label="有效期">
          <a-radio-group v-model:value="shareCreate.period">
            <a-radio-button :value="1">1 天</a-radio-button>
            <a-radio-button :value="2">7 天</a-radio-button>
            <a-radio-button :value="3">30 天</a-radio-button>
            <a-radio-button :value="0">永久</a-radio-button>
          </a-radio-group>
        </a-form-item>
        <a-form-item label="提取码"><a-input v-model:value="shareCreate.password" placeholder="留空则不设置提取码" /></a-form-item>
        <small class="form-hint">有效期：{{ shareExpireText[shareCreate.period] }}</small>
      </a-form>
    </a-modal>
  </div>
</template>
