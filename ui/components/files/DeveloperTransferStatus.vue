<script setup>
import { computed } from 'vue';
import { LoadingOutlined, RightOutlined, SwapOutlined } from '@antdv-next/icons';
import {
  developerTransferIsActive,
  developerTransferPercent,
  developerTransferProgressLabel,
  developerTransferStageLabel,
} from '../../developerTransfer.js';

const props = defineProps({ jobs: { type: Array, default: () => [] } });
const open = defineModel('open', { default: false });
const activeJobs = computed(() => props.jobs.filter(developerTransferIsActive)
  .sort((left, right) => Number(right.updated_at || 0) - Number(left.updated_at || 0)));
const lead = computed(() => activeJobs.value[0] || null);
const leadPercent = computed(() => developerTransferPercent(lead.value));
</script>

<template>
  <button
    v-if="activeJobs.length"
    type="button"
    class="transfer-task-trigger"
    :aria-label="`查看小号秒传进度，${activeJobs.length} 个任务进行中`"
    @click="open = true"
  >
    <span class="transfer-task-label"><LoadingOutlined spin />小号秒传</span>
    <span class="transfer-task-progress" aria-hidden="true">
      <a-progress :percent="leadPercent" :show-info="false" status="active" :stroke-width="4" />
    </span>
    <strong>{{ activeJobs.length > 1 ? `${activeJobs.length} 项` : `${leadPercent}%` }}</strong>
    <RightOutlined aria-hidden="true" />
  </button>

  <a-drawer v-model:open="open" title="小号秒传进度" :width="460">
    <div class="transfer-job-list" aria-live="polite">
      <article v-for="job in activeJobs" :key="job.id" class="transfer-job-card">
        <div class="transfer-job-heading">
          <span class="transfer-job-icon"><SwapOutlined /></span>
          <div>
            <strong :title="job.file_names?.[0]">{{ job.file_names?.[0] || `${job.total_count || 0} 项文件` }}</strong>
            <span>发送到 {{ job.target_name }} · {{ developerTransferStageLabel(job) }}</span>
          </div>
          <a-tag color="processing">{{ developerTransferProgressLabel(job) }}</a-tag>
        </div>
        <a-progress :percent="developerTransferPercent(job)" status="active" />
        <p v-if="job.current_path" class="transfer-current" :title="job.current_path">当前：{{ job.current_path }}</p>
        <p class="transfer-message">{{ job.message || '任务正在后台处理' }}</p>
      </article>
      <a-empty v-if="!activeJobs.length" description="当前没有进行中的小号秒传" />
    </div>
  </a-drawer>
</template>

<style scoped>
.transfer-task-trigger { display: grid; min-width: 202px; height: 32px; grid-template-columns: auto minmax(50px, 1fr) auto auto; align-items: center; gap: 8px; padding: 0 9px; border: 1px solid var(--primary-line, #d9d9d9); border-radius: 8px; color: var(--text-2, #525252); background: var(--surface, #fff); font: inherit; cursor: pointer; }
.transfer-task-trigger:hover { border-color: var(--primary, #262626); background: var(--primary-soft, #f5f5f5); }
.transfer-task-trigger:focus-visible { outline: 2px solid var(--primary, #262626); outline-offset: 2px; }
.transfer-task-label { display: inline-flex; align-items: center; gap: 5px; white-space: nowrap; }
.transfer-task-progress { min-width: 50px; }
.transfer-task-progress :deep(.ant-progress) { display: block; margin: 0; line-height: 1; }
.transfer-task-trigger strong { color: var(--text-1, #262626); font-size: 11px; font-variant-numeric: tabular-nums; white-space: nowrap; }
.transfer-job-list { display: grid; gap: 10px; }
.transfer-job-card { display: grid; gap: 11px; padding: 14px; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; background: var(--surface, #fff); }
.transfer-job-heading { display: grid; grid-template-columns: 36px minmax(0, 1fr) auto; align-items: center; gap: 10px; }
.transfer-job-heading > div { min-width: 0; }
.transfer-job-heading strong, .transfer-job-heading span { display: block; }
.transfer-job-heading strong { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.transfer-job-heading span { margin-top: 3px; color: var(--text-3, #737373); font-size: 12px; }
.transfer-job-icon { display: grid; width: 36px; height: 36px; place-items: center; border-radius: 9px; color: var(--primary, #1677ff); background: var(--primary-soft, #f0f5ff); }
.transfer-current, .transfer-message { margin: -3px 0 0; color: var(--text-3, #737373); font-size: 12px; line-height: 1.5; }
.transfer-current { overflow: hidden; color: var(--text-2, #525252); text-overflow: ellipsis; white-space: nowrap; }
@media (max-width: 760px) { .transfer-task-trigger { min-width: 168px; } .transfer-task-label { font-size: 0; } .transfer-task-label :deep(.anticon) { font-size: 13px; } }
</style>
