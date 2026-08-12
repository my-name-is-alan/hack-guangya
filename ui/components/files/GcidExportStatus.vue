<script setup>
import { computed } from 'vue';
import { CloseCircleOutlined, FileTextOutlined, LoadingOutlined, RightOutlined } from '@antdv-next/icons';
import { formatSize } from '../../formatters.js';

const props = defineProps({
  progress: { type: Object, default: null },
  running: { type: Boolean, default: false },
});
const open = defineModel('open', { default: false });

const percent = computed(() => Math.max(0, Math.min(100, Number(props.progress?.percent || 0))));
const failed = computed(() => props.progress?.status === 'failed');
const completed = computed(() => Math.max(0, Number(props.progress?.completed_files || 0)));
const total = computed(() => Math.max(0, Number(props.progress?.total_files || 0)));
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
    :class="{ 'is-failed': failed }"
    :aria-label="failed ? `秒传 JSON 生成失败：${props.progress?.error || '未知错误'}` : `查看秒传 JSON 生成详情，已完成 ${completed}/${total || '待扫描'}`"
    @click="open = true"
  >
    <span class="export-task-label">
      <LoadingOutlined v-if="props.running" spin />
      <CloseCircleOutlined v-else-if="failed" />
      {{ failed ? '生成失败' : '生成 JSON' }}
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
          <span>已完成 {{ completed }} / {{ total || '—' }} 个文件</span>
        </div>
        <a-tag :color="failed ? 'error' : 'processing'">{{ failed ? '失败' : '进行中' }}</a-tag>
      </div>
      <a-progress :percent="percent" :status="failed ? 'exception' : 'active'" />
      <div class="export-current-file">
        <span>当前文件</span>
        <strong :title="currentPath">{{ currentPath }}</strong>
      </div>
      <div class="export-metrics">
        <div><span>已读取 / 预计采样</span><strong>{{ byteLabel }}</strong></div>
        <div><span>源文件总大小</span><strong>{{ sourceBytes ? formatSize(sourceBytes) : '计算中' }}</strong></div>
      </div>
      <a-alert v-if="failed" type="error" show-icon message="生成失败" :description="props.progress?.error || '请稍后重试'" />
      <a-alert v-else type="info" show-icon message="最多并发处理 20 个文件；通常每个大文件只读取头、中、尾各 20 KB，单分段失败会独立重试（最多 3 次）；若云端不支持分段或缺少 GCID，才会回退完整校验，完整校验也支持重签重试（最多 3 次）。" />
    </div>
  </a-drawer>
</template>

<style scoped>
.export-task-trigger { display: grid; min-width: 206px; height: 32px; grid-template-columns: auto minmax(52px, 1fr) auto auto; align-items: center; gap: 8px; padding: 0 9px; border: 1px solid var(--primary-line, #d9d9d9); border-radius: 8px; color: var(--text-2, #525252); background: var(--surface, #fff); font: inherit; cursor: pointer; }
.export-task-trigger:hover { border-color: var(--primary, #262626); background: var(--primary-soft, #f5f5f5); }
.export-task-trigger.is-failed { border-color: #ffccc7; color: #cf1322; background: #fff2f0; }
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
.export-task-icon { display: grid; width: 36px; height: 36px; place-items: center; border-radius: 9px; color: var(--primary, #1677ff); background: var(--primary-soft, #f0f5ff); }
.export-current-file { min-width: 0; padding: 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 9px; background: var(--bg-toolbar, #fafafa); }
.export-current-file span, .export-current-file strong { display: block; }
.export-current-file span, .export-metrics span { color: var(--text-3, #737373); font-size: 11px; }
.export-current-file strong { margin-top: 5px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.export-metrics { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
.export-metrics > div { padding: 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 9px; }
.export-metrics strong { display: block; margin-top: 5px; font-size: 13px; font-variant-numeric: tabular-nums; }
@media (max-width: 760px) { .export-task-trigger { min-width: 170px; } .export-task-label { font-size: 0; } .export-task-label :deep(.anticon) { font-size: 13px; } .export-metrics { grid-template-columns: 1fr; } }
</style>
