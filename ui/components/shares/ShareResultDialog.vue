<script setup lang="ts">
import { computed } from 'vue'
import { CheckCircleOutlined, CopyOutlined, ExportOutlined, StarOutlined } from '@antdv-next/icons'
import { message } from 'antdv-next'
import { copyText, receiptAlertType, receiptStatusLabel } from '../../formatters.js'

interface ShareResult {
  label: string
  url: string
  code: string
  reused: boolean
  hdhiveStatus: string
  hdhiveMessage: string
  hdhiveResourceUrl: string
}

interface Props {
  open: boolean
  result: ShareResult
  saving?: boolean
}

interface Emits {
  'update:open': [open: boolean]
  save: []
}

const props = withDefaults(defineProps<Props>(), { saving: false })
const emit = defineEmits<Emits>()

const resultLabel = computed(() => props.result.label.trim() || '分享链接')
const shareUrl = computed(() => props.result.url.trim())
const shareCode = computed(() => props.result.code.trim())
const shareText = computed(() => [shareUrl.value, shareCode.value ? `提取码：${shareCode.value}` : ''].filter(Boolean).join('\n'))
const hasHdhiveReceipt = computed(() => Boolean(
  props.result.hdhiveStatus || props.result.hdhiveMessage || props.result.hdhiveResourceUrl,
))
const hdhiveMessage = computed(() => props.result.hdhiveMessage.trim()
  || (props.result.hdhiveStatus ? receiptStatusLabel({ status: props.result.hdhiveStatus }) : ''))
const hdhiveAlertType = computed(() => receiptAlertType(props.result.hdhiveStatus))

function close() {
  emit('update:open', false)
}

function save() {
  emit('save')
}

async function copyShareInfo() {
  if (!shareUrl.value) {
    message.error('分享链接为空，无法复制')
    return
  }

  await copyText(shareText.value, {
    success: (content: string) => message.success(content),
    info: () => message.error('复制失败，请手动复制分享信息'),
  })
}
</script>

<template>
  <a-modal
    :open="props.open"
    :footer="null"
    :title="props.result.reused ? '已复用分享' : '分享已创建'"
    width="560px"
    @cancel="close"
  >
    <div class="share-result">
      <div class="share-result-heading">
        <CheckCircleOutlined class="share-result-icon" aria-hidden="true" />
        <strong class="share-result-label">{{ resultLabel }}</strong>
        <a-tag :color="props.result.reused ? 'blue' : 'green'">
          {{ props.result.reused ? '已复用' : '创建成功' }}
        </a-tag>
      </div>

      <div class="share-result-fields">
        <label class="share-result-field">
          <span class="share-result-field-label">分享链接</span>
          <a-input :value="shareUrl" readonly aria-label="分享链接" />
        </label>
        <label v-if="shareCode" class="share-result-field">
          <span class="share-result-field-label">提取码</span>
          <a-input :value="shareCode" readonly aria-label="提取码" />
        </label>
      </div>

      <a-button
        type="primary"
        block
        :disabled="!shareUrl"
        aria-label="复制分享信息"
        @click="copyShareInfo"
      >
        <template #icon><CopyOutlined /></template>
        复制分享信息
      </a-button>

      <section v-if="hasHdhiveReceipt" class="hdhive-receipt" aria-label="HDHive 回执">
        <a-alert
          v-if="hdhiveMessage"
          :type="hdhiveAlertType"
          :message="hdhiveMessage"
          show-icon
        />
        <a-button
          v-if="props.result.hdhiveResourceUrl"
          type="link"
          :href="props.result.hdhiveResourceUrl"
          target="_blank"
          rel="noopener noreferrer"
          aria-label="在新窗口打开 HDHive 资源"
        >
          <template #icon><ExportOutlined /></template>
          查看 HDHive 资源
        </a-button>
      </section>

      <div class="share-result-actions">
        <a-button @click="close">完成</a-button>
        <a-button type="primary" :loading="props.saving" :disabled="!shareUrl" @click="save">
          <template #icon><StarOutlined /></template>
          加入收藏
        </a-button>
      </div>
    </div>
  </a-modal>
</template>

<style scoped>
.share-result {
  display: grid;
  gap: 16px;
}

.share-result-heading {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}

.share-result-icon {
  flex: 0 0 auto;
  color: var(--ant-color-success, #52c41a);
  font-size: 20px;
}

.share-result-label {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.share-result-fields {
  display: grid;
  gap: 12px;
}

.share-result-field {
  display: grid;
  gap: 6px;
}

.share-result-field-label {
  color: var(--ant-color-text-secondary, #667085);
  font-size: 13px;
}

.hdhive-receipt {
  display: grid;
  gap: 6px;
}

.hdhive-receipt :deep(.ant-btn) {
  justify-self: start;
  padding-inline: 0;
}

.share-result-actions {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding-top: 4px;
}
</style>
