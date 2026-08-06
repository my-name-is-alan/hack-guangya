<script setup>
import { onMounted, reactive, ref } from 'vue'
import { message } from 'antdv-next'
import { GlobalOutlined } from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText } from '../../formatters.js'
import NetworkTestDialog from '../network/NetworkTestDialog.vue'

const loading = ref(true)
const saving = ref(false)
const testOpen = ref(false)
const form = reactive({ proxy_url: '' })

async function load() {
  loading.value = true
  try { Object.assign(form, await bridge.invoke('get_network_preferences')) }
  catch (error) { message.error(errorText(error)) }
  finally { loading.value = false }
}

async function save() {
  saving.value = true
  try {
    Object.assign(form, await bridge.invoke('update_network_preferences', { input: { ...form } }))
    message.success('网络代理设置已保存')
  } catch (error) { message.error(errorText(error)) }
  finally { saving.value = false }
}

onMounted(load)
</script>

<template>
  <section class="network-panel">
    <a-spin :spinning="loading">
      <div class="panel-lead"><div><strong>网络偏好</strong><span>所有外部网络服务共用一个代理：GitHub、TMDB、Telegram 和 HDHive。支持 HTTP/HTTPS 与 SOCKS5，留空表示直连。</span></div><GlobalOutlined class="lead-icon" /></div>
      <a-alert type="info" show-icon message="代理仅保存在当前光鸭实例的本地状态库；不会把用户名、密码或完整代理地址写入日志。保存后会同时用于 HDHive 请求。" class="network-alert" />
      <div class="proxy-card">
        <div class="proxy-card-icon"><GlobalOutlined /></div>
        <div class="proxy-card-copy"><strong>全局网络代理</strong><small>GitHub 更新、TMDB 刮削、Telegram 网络测试和 HDHive 自动投稿统一走此代理</small><a-input v-model:value="form.proxy_url" allow-clear placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080" /></div>
      </div>
      <div class="panel-actions"><a-button :disabled="loading" @click="testOpen = true"><GlobalOutlined />检测网络可用性</a-button><a-button type="primary" :disabled="loading" :loading="saving" @click="save">保存代理设置</a-button></div>
    </a-spin>
    <NetworkTestDialog v-model:open="testOpen" :proxy-url="form.proxy_url" />
  </section>
</template>

<style scoped>
.network-panel { max-width: 980px; padding: 8px 18px 36px 24px; }
.panel-lead { display: flex; align-items: flex-start; justify-content: space-between; gap: 18px; margin-bottom: 16px; }
.panel-lead strong, .panel-lead span { display: block; }
.panel-lead strong { font-size: 18px; }
.panel-lead span { max-width: 680px; margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.55; }
.lead-icon { color: var(--primary, #1677ff); font-size: 24px; }
.network-alert { margin-bottom: 16px; }
.proxy-card { display: grid; grid-template-columns: 30px minmax(0, 1fr); align-items: start; gap: 12px; padding: 16px; border: 1px solid var(--line, #e5e7eb); border-radius: 10px; background: var(--surface, #fff); }
.proxy-card-icon { display: grid; width: 28px; height: 28px; place-items: center; border-radius: 50%; color: var(--primary, #1677ff); background: var(--primary-soft, #f0f5ff); font-size: 18px; }
.proxy-card-copy { min-width: 0; }
.proxy-card strong, .proxy-card small { display: block; }
.proxy-card small { margin: 3px 0 10px; color: var(--text-3, #98a2b3); font-size: 11px; line-height: 1.5; }
.panel-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 16px; }
@media (max-width: 680px) { .network-panel { padding-inline: 14px; } .panel-actions { align-items: stretch; flex-direction: column; } .panel-actions .ant-btn { width: 100%; } }
</style>
