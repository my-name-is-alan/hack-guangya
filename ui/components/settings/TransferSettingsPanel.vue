<script setup lang="ts">
import { onMounted, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { bridge } from '../../bridge.js'
import { errorText, unwrapData } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'

const session = useSessionStore()
const saving = shallowRef(false)
const form = reactive({
  upload_concurrency: 2,
  download_concurrency: 2,
  multipart_part_size: 'auto',
})
const multipartOptions = [
  { value: 'auto', label: '自动（推荐）' },
  { value: '4m', label: '4 MiB' },
  { value: '8m', label: '8 MiB' },
  { value: '16m', label: '16 MiB' },
]

async function loadSettings() {
  try {
    const data = unwrapData(await bridge.invoke('get_transfer_settings'))
    const settings = data.transfer || data
    Object.assign(form, {
      upload_concurrency: Number(settings.upload_concurrency || session.state.upload_concurrency || 2),
      download_concurrency: Number(settings.download_concurrency || session.state.download_concurrency || 2),
      multipart_part_size: String(settings.multipart_part_size || settings.multipart || session.state.multipart_part_size || 'auto'),
    })
  } catch {
    // 旧后端不提供独立设置接口时，保留当前会话中的默认值。
  }
}

async function saveSettings() {
  saving.value = true
  try {
    const data = unwrapData(await bridge.invoke('update_transfer_settings', { ...form }))
    session.applyState(data)
    message.success('传输设置已保存')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    saving.value = false
  }
}

onMounted(loadSettings)
</script>

<template>
  <section class="setting-section">
    <div class="section-lead">
      <strong>并发与分片</strong>
      <span>分片默认自动；手动档位仅提供 OSS 安全值。</span>
    </div>
    <a-form class="settings-form" layout="vertical">
      <div class="two-columns">
        <a-form-item label="上传并发">
          <a-input-number v-model:value="form.upload_concurrency" :min="1" :max="8" />
        </a-form-item>
        <a-form-item label="下载并发">
          <a-input-number v-model:value="form.download_concurrency" :min="1" :max="8" />
        </a-form-item>
      </div>
      <a-form-item label="OSS 分片大小">
        <a-select v-model:value="form.multipart_part_size" :options="multipartOptions" />
      </a-form-item>
      <a-button type="primary" :loading="saving" @click="saveSettings">保存传输设置</a-button>
    </a-form>
  </section>
</template>

<style scoped>
.setting-section { max-width: 760px; padding: 8px 18px 36px 24px; }
.section-lead { margin-bottom: 28px; }
.section-lead strong, .section-lead span { display: block; }
.section-lead strong { font-size: 18px; }
.section-lead span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; }
.settings-form { max-width: 520px; }
.two-columns { display: grid; grid-template-columns: 1fr 1fr; gap: 18px; }
</style>
