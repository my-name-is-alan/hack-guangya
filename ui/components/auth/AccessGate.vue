<script setup lang="ts">
import { reactive, shallowRef } from 'vue'
import { LockOutlined, SafetyCertificateOutlined } from '@antdv-next/icons'
import { useSessionStore } from '../../stores/session'
import { errorText } from '../../formatters.js'

const session = useSessionStore()
const form = reactive({ code: '' })
const loading = shallowRef(false)
const error = shallowRef('')

async function submit() {
  if (!form.code.trim()) {
    error.value = '请输入访问码'
    return
  }
  loading.value = true
  error.value = ''
  try {
    await session.unlockAccess(form.code)
    form.code = ''
  }
  catch (reason) {
    error.value = errorText(reason)
  }
  finally {
    loading.value = false
  }
}
</script>

<template>
  <main class="access-gate">
    <section class="access-panel" aria-labelledby="access-title">
      <div class="access-mark"><SafetyCertificateOutlined /></div>
      <h1 id="access-title">访问光鸭</h1>
      <p>此 Web 控制台受访问码保护</p>
      <a-form layout="vertical" @submit.prevent="submit()">
        <a-form-item label="访问码" :validate-status="error ? 'error' : undefined" :help="error || undefined">
          <a-input-password v-model:value="form.code" size="large" autocomplete="current-password" placeholder="输入部署时生成的访问码" @press-enter="submit()">
            <template #prefix><LockOutlined /></template>
          </a-input-password>
        </a-form-item>
        <a-button type="primary" size="large" block html-type="submit" :loading="loading">进入系统</a-button>
      </a-form>
      <small>访问码与云盘账号相互独立</small>
    </section>
  </main>
</template>

<style scoped>
.access-gate { display: grid; min-height: 100vh; place-items: center; padding: 32px; background: var(--app-bg, #fafafa); }
.access-panel { width: min(400px, 100%); }
.access-mark { display: grid; width: 48px; height: 48px; place-items: center; margin-bottom: 28px; border: 2px solid currentColor; border-radius: 16px; color: var(--primary, #262626); font-size: 22px; }
.access-panel h1 { margin: 0; font-size: 28px; letter-spacing: -.02em; }
.access-panel > p { margin: 8px 0 28px; color: var(--text-2, #525252); }
.access-panel small { display: block; margin-top: 20px; color: var(--text-3, #737373); text-align: center; }
</style>
