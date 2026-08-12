<script setup lang="ts">
import { computed } from 'vue'
import { LoadingOutlined, RightOutlined } from '@antdv-next/icons'

interface GcidImportStatus {
  status?: string
  destination_name?: string
  current_path?: string
  error?: string
  total_files?: number
  counts?: Record<string, number>
}

const props = defineProps<{
  status: GcidImportStatus | null
  percent: number
}>()

const open = defineModel<boolean>('open', { default: false })

const stageLabel = computed(() => (
  props.status?.status === 'preparing' ? '准备中' : `导入中 ${props.percent}%`
))

const detailText = computed(() => (
  props.status?.current_path || props.status?.error || '正在准备导入任务'
))

const counts = computed(() => ([
  ['总数', props.status?.total_files || 0],
  ['秒传', props.status?.counts?.imported || 0],
  ['已存在', props.status?.counts?.existing || 0],
  ['未命中', props.status?.counts?.missed || 0],
  ['冲突', props.status?.counts?.conflict || 0],
  ['失败', props.status?.counts?.failed || 0],
]))
</script>

<template>
  <button
    type="button"
    class="gcid-task-trigger"
    :aria-label="`查看 GCID 导入详情，${stageLabel}`"
    @click="open = true"
  >
    <span class="gcid-task-label"><LoadingOutlined spin />GCID 导入</span>
    <span class="gcid-task-progress" aria-hidden="true">
      <a-progress :percent="props.percent" :show-info="false" status="active" :stroke-width="4" />
    </span>
    <strong>{{ props.status?.status === 'preparing' ? '准备中' : `${props.percent}%` }}</strong>
    <RightOutlined aria-hidden="true" />
  </button>

  <a-drawer v-model:open="open" title="GCID 导入详情" :width="420">
    <div class="gcid-task-detail" aria-live="polite">
      <div class="gcid-task-heading">
        <div>
          <strong>{{ props.status?.destination_name || 'GCID 导入' }}</strong>
          <span>{{ stageLabel }}</span>
        </div>
        <a-tag color="processing">进行中</a-tag>
      </div>

      <a-progress :percent="props.percent" status="active" />
      <p class="gcid-current-path" :title="detailText">{{ detailText }}</p>

      <div class="gcid-task-counts">
        <div v-for="([label, value]) in counts" :key="label">
          <span>{{ label }}</span>
          <strong>{{ value }}</strong>
        </div>
      </div>
    </div>
  </a-drawer>
</template>

<style scoped>
.gcid-task-trigger {
  display: grid;
  min-width: 218px;
  height: var(--h-md, 28px);
  grid-template-columns: auto minmax(56px, 1fr) auto auto;
  align-items: center;
  gap: 8px;
  padding: 0 9px;
  border: 1px solid var(--primary-line, #e5e5e5);
  border-radius: 8px;
  color: var(--text-2, #525252);
  background: var(--surface, #fff);
  font: inherit;
  cursor: pointer;
}

.gcid-task-trigger:hover {
  border-color: var(--primary, #262626);
  background: var(--primary-soft, #f5f5f5);
}

.gcid-task-trigger:focus-visible {
  outline: 2px solid var(--primary, #262626);
  outline-offset: 2px;
}

.gcid-task-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  white-space: nowrap;
}

.gcid-task-progress {
  min-width: 56px;
}

.gcid-task-progress :deep(.ant-progress) {
  display: block;
  margin: 0;
  line-height: 1;
}

.gcid-task-trigger strong {
  color: var(--text-1, #262626);
  font-size: 11px;
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

.gcid-task-detail {
  display: grid;
  gap: 14px;
}

.gcid-task-heading {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}

.gcid-task-heading > div {
  min-width: 0;
}

.gcid-task-heading strong,
.gcid-task-heading span {
  display: block;
}

.gcid-task-heading span {
  margin-top: 3px;
  color: var(--text-3, #737373);
  font-size: 12px;
}

.gcid-current-path {
  margin: -6px 0 0;
  overflow: hidden;
  color: var(--text-3, #737373);
  font-size: 12px;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.gcid-task-counts {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
}

.gcid-task-counts > div {
  padding: 12px;
  border: 1px solid var(--line, #e5e5e5);
  border-radius: 9px;
  background: var(--bg-toolbar, #fafafa);
}

.gcid-task-counts span,
.gcid-task-counts strong {
  display: block;
}

.gcid-task-counts span {
  color: var(--text-3, #737373);
  font-size: 11px;
}

.gcid-task-counts strong {
  margin-top: 5px;
  color: var(--text-1, #262626);
  font-size: 18px;
  font-variant-numeric: tabular-nums;
}

@media (max-width: 760px) {
  .gcid-task-trigger { min-width: 176px; }
  .gcid-task-label { font-size: 0; }
  .gcid-task-label :deep(.anticon) { font-size: 13px; }
}
</style>
