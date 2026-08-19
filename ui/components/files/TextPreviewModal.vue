<script setup>
import { computed, ref, shallowRef, watch } from 'vue';
import { message } from 'antdv-next';
import { CopyOutlined, ExportOutlined } from '@antdv-next/icons';
import { bridge, isTauri } from '../../bridge.js';
import { copyText, errorText, fileId, formatSize, pick } from '../../formatters.js';
import { fileExtensionOf } from '../../fileOpen.js';
import { readCloudText, useFileOpener } from '../../composables/useFileOpener.js';

const MAX_PREVIEW_BYTES = 512 * 1024;

const { textPreview } = useFileOpener();
const loading = shallowRef(false);
const openingLocally = shallowRef(false);
const error = shallowRef('');
const content = shallowRef('');
const truncated = shallowRef(false);
const totalSize = shallowRef(0);
const pretty = ref(false);
let loadSequence = 0;

const record = computed(() => textPreview.record || null);
const fileName = computed(() => String(pick(record.value || {}, ['fileName', 'name'], '文本预览')));
const isJson = computed(() => fileExtensionOf(record.value) === 'json');
const prettyJson = computed(() => {
  if (!isJson.value || !content.value) return '';
  try {
    return JSON.stringify(JSON.parse(content.value), null, 2);
  } catch {
    return '';
  }
});
const displayText = computed(() => (pretty.value && prettyJson.value ? prettyJson.value : content.value));
const sizeLabel = computed(() => (totalSize.value ? formatSize(totalSize.value) : ''));

watch(() => textPreview.open, (open) => {
  if (open) void load();
});

async function load() {
  const target = record.value;
  if (!target) return;
  const sequence = ++loadSequence;
  loading.value = true;
  error.value = '';
  content.value = '';
  truncated.value = false;
  totalSize.value = 0;
  pretty.value = false;
  try {
    const result = await readCloudText(target, MAX_PREVIEW_BYTES);
    if (sequence !== loadSequence) return;
    content.value = result.text;
    truncated.value = result.truncated;
    totalSize.value = result.size;
    // JSON 默认直接展示格式化结果，原文一键切换。
    if (isJson.value && prettyJson.value) pretty.value = true;
  } catch (loadError) {
    if (sequence !== loadSequence) return;
    error.value = errorText(loadError);
  } finally {
    if (sequence === loadSequence) loading.value = false;
  }
}

async function copyContent() {
  if (!displayText.value) return;
  await copyText(displayText.value, message);
}

async function openWithSystem() {
  const target = record.value;
  if (!target || openingLocally.value) return;
  openingLocally.value = true;
  try {
    await bridge.invoke('open_cloud_file_with_system', {
      file_id: String(fileId(target)),
      file_name: String(pick(target, ['fileName', 'name'], '')),
    });
    message.success('已用系统默认程序打开');
  } catch (openError) {
    message.error(errorText(openError));
  } finally {
    openingLocally.value = false;
  }
}

function handleClose() {
  loadSequence += 1;
  textPreview.open = false;
  textPreview.record = null;
  content.value = '';
  error.value = '';
}
</script>

<template>
  <a-modal
    :open="textPreview.open"
    :title="fileName"
    :width="760"
    :footer="null"
    @cancel="handleClose"
  >
    <div class="text-preview">
      <a-alert
        v-if="truncated"
        type="warning"
        show-icon
        :message="`文件较大（${sizeLabel}），仅显示前 ${Math.round(MAX_PREVIEW_BYTES / 1024)} KB，完整内容请下载查看`"
      />
      <a-alert v-if="error" type="error" show-icon :message="error">
        <template #action><a-button size="small" @click="load">重试</a-button></template>
      </a-alert>

      <a-spin :spinning="loading">
        <pre v-if="!error" class="preview-body">{{ displayText || (loading ? '' : '（空文件）') }}</pre>
      </a-spin>

      <div class="preview-footer">
        <div class="footer-meta">
          <a-switch v-if="isJson && prettyJson" v-model:checked="pretty" size="small" />
          <span v-if="isJson && prettyJson">格式化 JSON</span>
          <span v-if="sizeLabel" class="size-label">{{ sizeLabel }}</span>
        </div>
        <a-flex gap="small">
          <a-button :disabled="!displayText" @click="copyContent"><template #icon><CopyOutlined /></template>复制内容</a-button>
          <a-button v-if="isTauri" :loading="openingLocally" @click="openWithSystem">
            <template #icon><ExportOutlined /></template>用系统程序打开
          </a-button>
        </a-flex>
      </div>
    </div>
  </a-modal>
</template>

<style scoped>
.text-preview { display: flex; flex-direction: column; gap: 10px; }
.preview-body { max-height: min(56vh, 560px); min-height: 160px; margin: 0; padding: 12px 14px; overflow: auto; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; background: var(--surface-muted, #fafafa); font-family: ui-monospace, SFMono-Regular, Consolas, 'Courier New', monospace; font-size: 12.5px; line-height: 1.6; white-space: pre-wrap; word-break: break-all; }
.preview-footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.footer-meta { display: flex; align-items: center; gap: 8px; color: var(--text-3, #737373); font-size: 12px; }
.size-label { font-variant-numeric: tabular-nums; }
</style>
