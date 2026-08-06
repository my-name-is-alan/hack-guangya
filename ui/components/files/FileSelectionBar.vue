<script setup lang="ts">
import {
  CheckOutlined,
  CloseOutlined,
  CopyOutlined,
  DeleteOutlined,
  DownloadOutlined,
  EditOutlined,
  ScissorOutlined,
  ShareAltOutlined,
  SwapOutlined,
  TagsOutlined,
} from '@antdv-next/icons'

defineProps<{
  selectedCount: number
  clipboardCount: number
  clipboardMode: 'copy' | 'move' | ''
}>()

defineEmits<{
  copy: []
  cut: []
  move: []
  rename: []
  download: []
  share: []
  scrape: []
  transferAccount: []
  delete: []
  paste: []
  clearSelection: []
  clearClipboard: []
}>()
</script>

<template>
  <div v-if="selectedCount" class="file-selection-bar" role="toolbar" aria-label="选中文件操作">
    <div class="selection-summary">
      <strong>已选 {{ selectedCount }} 项</strong>
      <a-button type="text" size="small" aria-label="取消选择" title="取消选择 (Esc)" @click="$emit('clearSelection')">
        <template #icon><CloseOutlined /></template>
      </a-button>
    </div>
    <div class="selection-actions">
      <a-button type="text" size="small" title="复制 (Ctrl+C)" @click="$emit('copy')"><template #icon><CopyOutlined /></template>复制 <kbd>Ctrl C</kbd></a-button>
      <a-button type="text" size="small" title="剪切 (Ctrl+X)" @click="$emit('cut')"><template #icon><ScissorOutlined /></template>剪切 <kbd>Ctrl X</kbd></a-button>
      <a-button type="text" size="small" title="重命名 (F2)" @click="$emit('rename')"><template #icon><EditOutlined /></template>重命名 <kbd>F2</kbd></a-button>
      <a-button type="text" size="small" @click="$emit('move')"><template #icon><SwapOutlined /></template>移动到</a-button>
      <a-button type="text" size="small" @click="$emit('download')"><template #icon><DownloadOutlined /></template>下载</a-button>
      <a-button type="text" size="small" @click="$emit('share')"><template #icon><ShareAltOutlined /></template>分享</a-button>
      <a-button type="text" size="small" @click="$emit('scrape')"><template #icon><TagsOutlined /></template>刮削到媒体库</a-button>
      <a-button type="text" size="small" @click="$emit('transferAccount')"><template #icon><SwapOutlined /></template>小号秒传</a-button>
      <a-button type="text" size="small" danger title="删除 (Delete)" @click="$emit('delete')"><template #icon><DeleteOutlined /></template>删除 <kbd>Del</kbd></a-button>
    </div>
    <slot name="status" />
  </div>

  <div v-else-if="clipboardCount" class="file-clipboard-bar" role="status">
    <span>
      <component :is="clipboardMode === 'move' ? ScissorOutlined : CopyOutlined" />
      已{{ clipboardMode === 'move' ? '剪切' : '复制' }} {{ clipboardCount }} 项
    </span>
    <div class="clipboard-actions">
      <a-button type="primary" size="small" title="粘贴 (Ctrl+V)" @click="$emit('paste')"><template #icon><CheckOutlined /></template>粘贴到当前目录 <kbd>Ctrl V</kbd></a-button>
      <a-button type="text" size="small" aria-label="清空文件剪贴板" @click="$emit('clearClipboard')"><template #icon><CloseOutlined /></template></a-button>
    </div>
    <slot name="status" />
  </div>
</template>

<style scoped>
.file-selection-bar,
.file-clipboard-bar {
  display: flex;
  width: 100%;
  min-width: 0;
  min-height: 32px;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  background: transparent;
}

.selection-summary,
.file-clipboard-bar > span,
.clipboard-actions {
  display: flex;
  flex: 0 0 auto;
  align-items: center;
  gap: 6px;
}

.selection-summary strong,
.file-clipboard-bar > span {
  font-size: 11px;
  white-space: nowrap;
}

.selection-actions {
  display: flex;
  flex: 1 1 auto;
  min-width: 0;
  align-items: center;
  justify-content: flex-end;
  overflow-x: auto;
  scrollbar-width: none;
}

.selection-actions::-webkit-scrollbar {
  display: none;
}

.selection-actions :deep(.ant-btn) {
  flex: 0 0 auto;
}

kbd {
  margin-left: 3px;
  padding: 1px 4px;
  border: 1px solid var(--line, #e5e5e5);
  border-radius: 4px;
  color: var(--text-3, #737373);
  background: var(--surface, #fff);
  font-size: 9px;
  line-height: 1.4;
}

@media (max-width: 820px) {
  .file-selection-bar { align-items: stretch; flex-direction: column; gap: 0; padding-block: 2px; }
  .selection-summary { justify-content: space-between; }
  .selection-actions { justify-content: flex-start; }
  kbd { display: none; }
}
</style>
