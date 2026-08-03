<script setup lang="ts">
import { computed, onMounted, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { CopyOutlined } from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText, unwrapData } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'

const session = useSessionStore()
const saving = shallowRef(false)
const enabledSaving = shallowRef(false)
const form = reactive({ enabled: true, base_url: '', secret: '' })
const instanceId = computed(() => String(session.state.hdhive?.instance_id || ''))

async function copyInstanceId() {
  if (!instanceId.value) {
    message.warning('实例 ID 尚未生成，请刷新状态后重试')
    return
  }
  try {
    await navigator.clipboard.writeText(instanceId.value)
    message.success('实例 ID 已复制')
  } catch {
    message.warning('无法自动复制，请选中实例 ID 手动复制')
  }
}

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
      <a-form-item label="同步实例 ID">
        <div class="instance-field">
          <a-input :value="instanceId" readonly placeholder="正在读取实例 ID…" />
          <a-button :disabled="!instanceId" @click="copyInstanceId">
            <template #icon><CopyOutlined /></template>
            复制
          </a-button>
        </div>
        <div class="field-help">
          此 ID 由当前同步端首次启动时生成并保存在本地状态库，不要与其他同步实例共用。
        </div>
      </a-form-item>
      <a-form-item label="服务地址">
        <a-input v-model:value="form.base_url" placeholder="https://hdhive.example.com" :disabled="!form.enabled" />
      </a-form-item>
      <a-form-item label="接入密钥">
        <a-input-password v-model:value="form.secret" placeholder="留空表示不修改" :disabled="!form.enabled" />
      </a-form-item>
      <a-button type="primary" :loading="saving" :disabled="enabledSaving" @click="saveSettings">保存 HDHive 设置</a-button>
    </a-form>
    <a-alert class="setup-guide" type="info" show-icon>
      <template #message>在 HDHive 中这样配置</template>
      <template #description>
        <ol>
          <li>进入 HDHive 管理后台 → 光鸭同步 → 添加账号。</li>
          <li>把上面的实例 ID 填入“同步实例 ID”，再填写已绑定 HDHive 账号的 Telegram 数字 ID。</li>
          <li>创建后复制后台只显示一次的 HMAC 密钥，回到这里填入“接入密钥”并保存。</li>
        </ol>
      </template>
    </a-alert>
  </section>
</template>

<style scoped>
.setting-section { max-width: 760px; padding: 8px 18px 36px 24px; }
.setting-row { display: flex; align-items: center; justify-content: space-between; gap: 24px; min-height: 54px; margin-bottom: 18px; }
.setting-row strong, .setting-row span { display: block; }
.setting-row span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; }
.settings-form { max-width: 520px; }
.instance-field { display: flex; gap: 8px; }
.instance-field :deep(.ant-input) { font-family: ui-monospace, SFMono-Regular, Consolas, monospace; }
.field-help { margin-top: 6px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.5; }
.setup-guide { max-width: 620px; margin-top: 22px; }
.setup-guide ol { margin: 8px 0 0; padding-left: 20px; line-height: 1.8; }
</style>
