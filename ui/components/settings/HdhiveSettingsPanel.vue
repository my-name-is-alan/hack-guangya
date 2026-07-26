<script setup lang="ts">
import { onMounted, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { bridge } from '../../bridge.js'
import { errorText, unwrapData } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'

const session = useSessionStore()
const saving = shallowRef(false)
const enabledSaving = shallowRef(false)
const form = reactive({ enabled: true, base_url: '', secret: '' })

async function saveEnabled(enabled: boolean) {
  enabledSaving.value = true
  try {
    const data = unwrapData(await bridge.invoke('update_hdhive_config', {
      enabled,
      base_url: session.state.hdhive?.base_url || '',
    }))
    Object.assign(session.state.hdhive, data)
    form.enabled = data.enabled !== false
    message.success(form.enabled ? 'HDHive 已开启' : 'HDHive 已关闭')
  } catch (reason) {
    form.enabled = session.state.hdhive?.enabled !== false
    message.error(errorText(reason))
  } finally {
    enabledSaving.value = false
  }
}

async function saveSettings() {
  saving.value = true
  try {
    const data = unwrapData(await bridge.invoke('update_hdhive_config', {
      enabled: form.enabled,
      base_url: form.base_url.trim(),
      secret: form.secret.trim() || undefined,
    }))
    Object.assign(session.state.hdhive, data)
    form.secret = ''
    message.success('HDHive 设置已保存')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    saving.value = false
  }
}

onMounted(() => {
  Object.assign(form, {
    enabled: session.state.hdhive?.enabled !== false,
    base_url: session.state.hdhive?.base_url || '',
    secret: '',
  })
})
</script>

<template>
  <section class="setting-section">
    <div class="setting-row">
      <div>
        <strong>HDHive 集成</strong>
        <span>关闭后停止自动投稿与回执轮询，已有备份任务配置会保留。</span>
      </div>
      <a-switch
        v-model:checked="form.enabled"
        :loading="enabledSaving"
        aria-label="启用或关闭 HDHive 集成"
        @change="saveEnabled"
      />
    </div>
    <a-form class="settings-form" layout="vertical">
      <a-form-item label="服务地址">
        <a-input v-model:value="form.base_url" placeholder="https://hdhive.example.com" :disabled="!form.enabled" />
      </a-form-item>
      <a-form-item label="接入密钥">
        <a-input-password v-model:value="form.secret" placeholder="留空表示不修改" :disabled="!form.enabled" />
      </a-form-item>
      <a-button type="primary" :loading="saving" :disabled="enabledSaving" @click="saveSettings">保存 HDHive 设置</a-button>
    </a-form>
  </section>
</template>

<style scoped>
.setting-section { max-width: 760px; padding: 8px 18px 36px 24px; }
.setting-row { display: flex; align-items: center; justify-content: space-between; gap: 24px; min-height: 54px; margin-bottom: 18px; }
.setting-row strong, .setting-row span { display: block; }
.setting-row span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; }
.settings-form { max-width: 520px; }
</style>
