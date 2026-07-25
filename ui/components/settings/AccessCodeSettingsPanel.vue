<script setup lang="ts">
import { reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { bridge } from '../../bridge.js'
import { errorText } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'

const session = useSessionStore()
const saving = shallowRef(false)
const form = reactive({ current_code: '', new_code: '', confirm_code: '' })

async function updateAccessCode() {
  if (form.new_code.length < 8 || form.new_code.length > 256) {
    message.warning('新访问码须为 8–256 个字符')
    return
  }
  if (form.new_code !== form.confirm_code) {
    message.warning('两次输入的新访问码不一致')
    return
  }

  saving.value = true
  try {
    await bridge.invoke('update_access_code', {
      current_code: form.current_code,
      new_code: form.new_code,
    })
    Object.assign(form, { current_code: '', new_code: '', confirm_code: '' })
    message.success('访问码已更新')
    await session.checkAccess()
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    saving.value = false
  }
}
</script>

<template>
  <section class="setting-section">
    <div class="section-lead">
      <strong>Web 控制台访问码</strong>
      <span>公网部署时必须先通过此门禁，访问码不会作为云盘凭据使用。</span>
    </div>
    <a-form class="settings-form" layout="vertical" @submit.prevent="updateAccessCode">
      <a-form-item label="当前访问码">
        <a-input-password v-model:value="form.current_code" autocomplete="current-password" :maxlength="256" />
      </a-form-item>
      <a-form-item label="新访问码">
        <a-input-password v-model:value="form.new_code" autocomplete="new-password" :maxlength="256" />
      </a-form-item>
      <a-form-item label="确认新访问码">
        <a-input-password v-model:value="form.confirm_code" autocomplete="new-password" :maxlength="256" />
      </a-form-item>
      <a-button type="primary" html-type="submit" :loading="saving">更新访问码</a-button>
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
</style>
