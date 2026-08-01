<script setup>
import { computed, ref, watch } from 'vue';
import { message, Modal } from 'antdv-next';
import { CopyOutlined, DisconnectOutlined, InfoCircleOutlined, LinkOutlined, ReloadOutlined } from '@antdv-next/icons';
import { bridge } from '../../bridge.js';
import { copyText, errorText, fileId, formatSize, formatTime, isFolder, pick, unwrapData } from '../../formatters.js';

const props = defineProps({
  open: { type: Boolean, default: false },
  record: { type: Object, default: null },
});
const emit = defineEmits(['update:open', 'closed']);

const loading = ref(false);
const loadError = ref('');
const response = ref(null);
const directLink = ref('');
const directLinkLoading = ref(false);
const directLinkError = ref('');
let requestSerial = 0;

const details = computed(() => {
  const payload = unwrapData(response.value);
  const fileInfo = payload.fileInfo || payload.file_info || payload.info || payload;
  return { ...(props.record || {}), ...(fileInfo && typeof fileInfo === 'object' ? fileInfo : {}) };
});
const location = computed(() => {
  const payload = unwrapData(response.value);
  const raw = payload.location || payload.path || details.value.location || details.value.parentName || '';
  if (Array.isArray(raw)) {
    return raw.map((item) => typeof item === 'string' ? item : pick(item, ['fileName', 'name', 'title'], '')).filter(Boolean).join(' / ');
  }
  if (raw && typeof raw === 'object') return pick(raw, ['path', 'fileName', 'name'], '—');
  return String(raw || '—');
});
const name = computed(() => pick(details.value, ['fileName', 'name', 'title'], '未命名文件'));
const extension = computed(() => String(pick(details.value, ['ext', 'fileSuffix', 'extension'], '') || '').replace(/^\./, '').toUpperCase());
const canToggleDirectLink = computed(() => (
  Boolean(fileId(details.value))
  && isFolder(details.value)
  && Number(pick(details.value, ['depth'], 0)) === 1
));
const canGetDirectLink = computed(() => Boolean(fileId(details.value)) && !isFolder(details.value));
const canShowDirectLink = computed(() => canToggleDirectLink.value || canGetDirectLink.value);

async function loadDetails() {
  const id = fileId(props.record);
  if (!props.open || !id) {
    response.value = null;
    loadError.value = id ? '' : '当前项目缺少 fileId，无法读取详情';
    return;
  }
  const serial = ++requestSerial;
  loading.value = true;
  loadError.value = '';
  try {
    const payload = await bridge.invoke('get_file_detail', { file_id: id });
    if (serial === requestSerial) response.value = payload;
  } catch (error) {
    if (serial !== requestSerial) return;
    loadError.value = errorText(error);
    message.error(loadError.value);
  } finally {
    if (serial === requestSerial) loading.value = false;
  }
}

function close() {
  emit('update:open', false);
}

function handleOpenChange(open) {
  if (!open) emit('closed');
}

async function getDirectLink(shortLink = false) {
  const id = fileId(details.value);
  if (!id || !canGetDirectLink.value || directLinkLoading.value) return;
  directLinkLoading.value = true;
  directLinkError.value = '';
  try {
    const data = unwrapData(await bridge.invoke('get_direct_link', { file_id: id, short_link: shortLink }));
    const url = String(pick(data, ['directLink', 'direct_link', 'url'], '') || '').trim();
    if (!url) throw new Error('光鸭没有返回直链，请先开启直链后重试');
    directLink.value = url;
  } catch (error) {
    directLinkError.value = errorText(error);
    message.error(directLinkError.value);
  } finally {
    directLinkLoading.value = false;
  }
}

async function enableDirectLink() {
  const id = fileId(details.value);
  if (!id || !canToggleDirectLink.value || directLinkLoading.value) return;
  directLinkLoading.value = true;
  directLinkError.value = '';
  try {
    await bridge.invoke('set_direct_link', { file_id: id });
    message.success('直链文件夹已开启');
  } catch (error) {
    directLinkError.value = errorText(error);
    message.error(directLinkError.value);
  } finally {
    directLinkLoading.value = false;
  }
}

function disableDirectLink() {
  const id = fileId(details.value);
  if (!id || !canToggleDirectLink.value || directLinkLoading.value) return;
  Modal.confirm({
    title: '关闭直链文件夹',
    content: '此文件夹内已有直链将立即失效，是否继续？',
    okText: '关闭',
    okButtonProps: { danger: true },
    cancelText: '取消',
    async onOk() {
      directLinkLoading.value = true;
      try {
        await bridge.invoke('unset_direct_link', { file_id: id });
        directLink.value = '';
        directLinkError.value = '';
        message.success('直链文件夹已关闭');
      } catch (error) {
        directLinkError.value = errorText(error);
        message.error(directLinkError.value);
        throw error;
      } finally {
        directLinkLoading.value = false;
      }
    },
  });
}

async function copyDirectLink() {
  if (directLink.value) await copyText(directLink.value, message);
}

watch(() => [props.open, fileId(props.record)], ([open]) => {
  directLink.value = '';
  directLinkError.value = '';
  if (open) void loadDetails();
  else requestSerial += 1;
}, { immediate: true });
</script>

<template>
  <a-drawer
    :open="open"
    title="文件详情"
    :width="420"
    placement="right"
    @update:open="close"
    @after-open-change="handleOpenChange"
  >
    <template #extra>
      <a-button type="text" :loading="loading" aria-label="刷新文件详情" @click="loadDetails">
        <template #icon><ReloadOutlined /></template>
      </a-button>
    </template>

    <a-skeleton v-if="loading && !response" active :paragraph="{ rows: 8 }" />
    <template v-else>
      <a-alert
        v-if="loadError"
        type="error"
        show-icon
        :message="loadError"
      ><template #action><a-button size="small" @click="loadDetails">重试</a-button></template></a-alert>
      <div class="details-heading">
        <a-avatar :size="46" class="details-icon"><InfoCircleOutlined /></a-avatar>
        <div>
          <strong :title="name">{{ name }}</strong>
          <span>{{ isFolder(details) ? '文件夹' : (extension || '文件') }}</span>
        </div>
      </div>
      <a-descriptions :column="1" size="small" bordered class="details-grid">
        <a-descriptions-item label="文件 ID">{{ fileId(details) || '—' }}</a-descriptions-item>
        <a-descriptions-item label="位置">{{ location }}</a-descriptions-item>
        <a-descriptions-item label="大小">{{ isFolder(details) ? '—' : formatSize(pick(details, ['fileSize', 'size'], 0)) }}</a-descriptions-item>
        <a-descriptions-item label="修改时间">{{ formatTime(pick(details, ['utime', 'lastUpdateTime', 'updateTime', 'modifiedAt'], 0)) }}</a-descriptions-item>
        <a-descriptions-item label="创建时间">{{ formatTime(pick(details, ['ctime', 'createTime', 'createdAt'], 0)) }}</a-descriptions-item>
        <a-descriptions-item v-if="pick(details, ['gcid'], '')" label="GCID"><span class="mono-value">{{ pick(details, ['gcid'], '') }}</span></a-descriptions-item>
        <a-descriptions-item v-if="pick(details, ['md5'], '')" label="MD5"><span class="mono-value">{{ pick(details, ['md5'], '') }}</span></a-descriptions-item>
      </a-descriptions>

      <template v-if="canShowDirectLink">
        <a-divider />
        <section class="direct-link-section" :aria-label="canToggleDirectLink ? '直链文件夹管理' : '文件直链获取'">
          <div class="direct-link-heading">
            <div v-if="canToggleDirectLink">
              <strong>直链文件夹</strong>
              <span>开启后，可在此文件夹内的文件详情获取长链或短链；仅根目录一级文件夹支持</span>
            </div>
            <div v-else>
              <strong>文件直链</strong>
              <span>请先在所属根级文件夹详情中开启；获取会消耗账号直链流量</span>
            </div>
            <a-space wrap>
              <template v-if="canToggleDirectLink">
                <a-button size="small" :loading="directLinkLoading" @click="enableDirectLink"><template #icon><LinkOutlined /></template>开启</a-button>
                <a-button size="small" danger :disabled="directLinkLoading" @click="disableDirectLink"><template #icon><DisconnectOutlined /></template>关闭</a-button>
              </template>
              <template v-else>
                <a-button size="small" :loading="directLinkLoading" @click="getDirectLink(false)">获取长链</a-button>
                <a-button size="small" :loading="directLinkLoading" @click="getDirectLink(true)">获取短链</a-button>
              </template>
            </a-space>
          </div>
          <a-alert v-if="directLinkError" type="warning" show-icon :message="directLinkError" />
          <a-input v-if="directLink" :value="directLink" readonly aria-label="文件直链" class="direct-link-value">
            <template #suffix><a-button type="text" size="small" aria-label="复制文件直链" @click="copyDirectLink"><CopyOutlined /></a-button></template>
          </a-input>
        </section>
      </template>
    </template>
  </a-drawer>
</template>

<style scoped>
.details-heading { display: flex; align-items: center; gap: 12px; margin-bottom: 18px; }
.details-heading > div:last-child { min-width: 0; }
.details-heading strong, .details-heading span { display: block; }
.details-heading strong { max-width: 300px; overflow: hidden; font-size: 15px; text-overflow: ellipsis; white-space: nowrap; }
.details-heading span { margin-top: 3px; color: var(--text-3, #98a2b3); font-size: 12px; }
.details-icon { color: var(--primary, #262626); background: var(--primary-soft, #f5f5f5); }
.details-grid { overflow-wrap: anywhere; }
.mono-value { font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 11px; }
.direct-link-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 12px; }
.direct-link-heading strong, .direct-link-heading span { display: block; }
.direct-link-heading span { margin-top: 3px; color: var(--text-3, #98a2b3); font-size: 12px; }
.direct-link-value { margin-top: 12px; }
</style>
