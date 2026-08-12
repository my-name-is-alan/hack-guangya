<script setup>
import { computed } from 'vue';
import { CloseCircleOutlined, FileTextOutlined, LoadingOutlined, RightOutlined } from '@antdv-next/icons';
import { formatSize } from '../../formatters.js';

const props = defineProps({
  progress: { type: Object, default: null },
  running: { type: Boolean, default: false },
});
defineEmits(['exportLog']);
const open = defineModel('open', { default: false });

const percent = computed(() => Math.max(0, Math.min(100, Number(props.progress?.percent || 0))));
const failed = computed(() => props.progress?.status === 'failed');
const warning = computed(() => props.progress?.status === 'warning');
const completed = computed(() => Math.max(0, Number(props.progress?.completed_files || 0)));
const total = computed(() => Math.max(0, Number(props.progress?.total_files || 0)));
const scanning = computed(() => props.progress?.phase === 'scan');
const scannedPages = computed(() => Math.max(0, Number(props.progress?.scanned_pages || 0)));
const totalPages = computed(() => Math.max(0, Number(props.progress?.total_pages || 0)));
const scannedEntries = computed(() => Math.max(0, Number(props.progress?.scanned_entries || 0)));
const currentPath = computed(() => props.progress?.current_path || '正在扫描所选文件');
const readBytes = computed(() => Number(props.progress?.sampled_bytes ?? props.progress?.downloaded_bytes ?? 0));
const plannedBytes = computed(() => Number(props.progress?.planned_sample_bytes ?? props.progress?.total_bytes ?? 0));
const sourceBytes = computed(() => Number(props.progress?.source_total_bytes ?? 0));
const byteLabel = computed(() => {
  const read = readBytes.value ? formatSize(readBytes.value) : '0 B';
  const planned = plannedBytes.value ? formatSize(plannedBytes.value) : '计算中';
  return `${read} / 预计 ${planned}`;
});
</script>

<template>
  <button
    type="button"
    class="export-task-trigger"
    :class="{ 'is-failed': failed, 'is-warning': warning }"
    :aria-label="failed ? `秒传 JSON 生成失败：${props.progress?.error || '未知错误'}` : `查看秒传 JSON 生成详情，已完成 ${completed}/${total || '待扫描'}`"
    @click="open = true"
  >
    <span class="export-task-label">
      <LoadingOutlined v-if="props.running" spin />
      <CloseCircleOutlined v-else-if="failed" />
      {{ failed ? '生成失败' : warning ? '部分跳过' : '生成 JSON' }}
    </span>
    <span class="export-task-progress" aria-hidden="true">
      <a-progress :percent="percent" :show-info="false" :status="failed ? 'exception' : 'active'" :stroke-width="4" />
    </span>
    <strong>{{ percent }}%</strong>
    <RightOutlined aria-hidden="true" />
  </button>

  <a-drawer v-model:open="open" title="秒传 JSON 生成进度" :width="440">
    <div class="export-task-detail" aria-live="polite">
      <div class="export-task-heading">
        <span class="export-task-icon"><FileTextOutlined /></span>
        <div>
          <strong>{{ props.progress?.stage || '正在生成秒传 JSON' }}</strong>
          <span v-if="scanning">已加载 {{ scannedPages }} / {{ totalPages || '—' }} 页，{{ scannedEntries }} 条索引</span>
          <span v-else>已完成 {{ completed }} / {{ total || '—' }} 个文件</span>
        </div>
        <a-tag :color="failed ? 'error' : warning ? 'warning' : 'processing'">{{ failed ? '失败' : warning ? '部分跳过' : '进行中' }}</a-tag>
      </div>
      <a-progress :percent="percent" :status="failed ? 'exception' : 'active'" />
      <div class="export-current-file">
        <span>{{ scanning ? '扫描进度' : '当前文件' }}</span>
        <strong :title="currentPath">{{ currentPath }}</strong>
      </div>
      <div class="export-metrics">
        <div><span>已读取 / 预计采样</span><strong>{{ byteLabel }}</strong></div>
        <div><span>源文件总大小</span><strong>{{ sourceBytes ? formatSize(sourceBytes) : '计算中' }}</strong></div>
      </div>
      <a-alert v-if="failed" type="error" show-icon message="生成失败" :description="props.progress?.error || '请稍后重试'" />
      <a-alert v-else-if="warning" type="warning" show-icon message="部分文件读取失败" description="已生成成功文件的 JSON；请导出诊断日志后发给开发者排查失败请求。" />
      <a-alert v-else type="info" show-icon message="大库扫描使用每页 1000 条的全库索引并发加载，再按完整父目录链筛选；小目录仍直接扫描。相同选择 10 分钟内可复用完整快照，重新扫描也会复用未变化文件的指纹。全局最多并发 24 个 CDN Range 请求；通常每个大文件只读取头、中、尾各 20 KB，单分段失败会独立错峰重试（最多 3 次），不会回退整文件下载。" />
      <div class="export-log-actions">
        <a-button :disabled="props.running" @click="$emit('exportLog')">导出诊断日志</a-button>
        <span>{{ props.running ? '任务结束后即可导出' : '日志包含请求阶段、重试、HTTP 状态和耗时，凭据及签名地址已脱敏' }}</span>
      </div>
    </div>
  </a-drawer>
</template>

<style scoped>
.export-task-trigger { display: grid; min-width: 206px; height: var(--h-md, 28px); grid-template-columns: auto minmax(52px, 1fr) auto auto; align-items: center; gap: 8px; padding: 0 9px; border: 1px solid var(--primary-line, #e5e5e5); border-radius: 8px; color: var(--text-2, #525252); background: var(--surface, #fff); font: inherit; cursor: pointer; }
.export-task-trigger:hover { border-color: var(--primary, #262626); background: var(--primary-soft, #f5f5f5); }
.export-task-trigger.is-failed { border-color: #fca5a5; color: #b91c1c; background: var(--danger-soft, #fef2f2); }
.export-task-trigger.is-warning { border-color: #fdba74; color: #c2410c; background: var(--warning-soft, #fff7ed); }
.export-task-trigger:focus-visible { outline: 2px solid var(--primary, #262626); outline-offset: 2px; }
.export-task-label { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }
.export-task-progress { min-width: 52px; }
.export-task-progress :deep(.ant-progress) { display: block; margin: 0; line-height: 1; }
.export-task-trigger strong { color: var(--text-1, #262626); font-size: 11px; font-variant-numeric: tabular-nums; }
.export-task-detail { display: grid; gap: 15px; }
.export-task-heading { display: grid; grid-template-columns: 36px minmax(0, 1fr) auto; align-items: center; gap: 10px; }
.export-task-heading > div { min-width: 0; }
.export-task-heading strong, .export-task-heading span { display: block; }
.export-task-heading span { margin-top: 3px; color: var(--text-3, #737373); font-size: 12px; }
.export-task-icon { display: grid; width: 36px; height: 36px; place-items: center; border-radius: 9px; color: var(--primary, #262626); background: var(--primary-soft, #f5f5f5); }
.export-current-file { min-width: 0; padding: 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 9px; background: var(--bg-toolbar, #fafafa); }
.export-current-file span, .export-current-file strong { display: block; }
.export-current-file span, .export-metrics span { color: var(--text-3, #737373); font-size: 11px; }
.export-current-file strong { margin-top: 5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.export-metrics { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.export-metrics > div { padding: 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 9px; }
.export-metrics strong { display: block; margin-top: 5px; font-size: 13px; font-variant-numeric: tabular-nums; }
.export-log-actions { display: flex; align-items: center; gap: 10px; }
.export-log-actions span { color: var(--text-3, #737373); font-size: 11px; line-height: 1.5; }
@media (max-width: 760px) { .export-task-trigger { min-width: 170px; } .export-task-label { font-size: 0; } .export-task-label :deep(.anticon) { font-size: 13px; } .export-metrics { grid-template-columns: 1fr; } }
</style>
