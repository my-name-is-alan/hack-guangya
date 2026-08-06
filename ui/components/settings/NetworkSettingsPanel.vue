<script setup>
import { onMounted, reactive, ref } from 'vue'
import { message } from 'antdv-next'
import { GlobalOutlined, GithubOutlined, SendOutlined, CloudServerOutlined } from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText } from '../../formatters.js'

const loading = ref(true)
const saving = ref(false)
const testing = ref('')
const form = reactive({ github_proxy: '', tmdb_proxy: '', tg_proxy: '' })

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

async function test(target) {
  testing.value = target
  try {
    const result = await bridge.invoke('test_network', { target, proxy_url: proxyForTarget(target) })
    if (result?.reachable) message.success(`${target.toUpperCase()}：${result.message || '网络可达'}（${result.latency_ms || 0} ms）`)
    else message.error(`${target.toUpperCase()}：${result?.message || '网络不可达'}`)
  } catch (error) { message.error(errorText(error)) }
  finally { testing.value = '' }
}

function proxyForTarget(target) {
  const key = target === 'tg' ? 'tg_proxy' : `${target}_proxy`
  return form[key]
}

onMounted(load)
</script>

<template>
  <section class="network-panel">
    <a-spin :spinning="loading">
      <div class="panel-lead"><div><strong>网络偏好</strong><span>为 GitHub、TMDB 和 Telegram 分别设置代理。支持 HTTP/HTTPS 与 SOCKS5，留空表示直连。</span></div><GlobalOutlined class="lead-icon" /></div>
      <a-alert type="info" show-icon message="代理仅保存在当前光鸭实例的本地状态库；不会把用户名、密码或完整代理地址写入日志。" class="network-alert" />
      <div class="proxy-grid">
        <article class="proxy-card"><GithubOutlined /><div><strong>GitHub</strong><small>更新检查与 GitHub 网络测试</small><a-input v-model:value="form.github_proxy" allow-clear placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080" /></div><a-button size="small" :loading="testing === 'github'" @click="test('github')">测试</a-button></article>
        <article class="proxy-card"><CloudServerOutlined /><div><strong>TMDB</strong><small>TMDB API、海报与背景图下载</small><a-input v-model:value="form.tmdb_proxy" allow-clear placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080" /></div><a-button size="small" :loading="testing === 'tmdb'" @click="test('tmdb')">测试</a-button></article>
        <article class="proxy-card"><SendOutlined /><div><strong>Telegram</strong><small>Telegram Bot API 网络测试</small><a-input v-model:value="form.tg_proxy" allow-clear placeholder="http://127.0.0.1:7890 或 socks5://127.0.0.1:1080" /></div><a-button size="small" :loading="testing === 'tg'" @click="test('tg')">测试</a-button></article>
      </div>
      <div class="panel-actions"><a-button type="primary" :loading="saving" @click="save">保存代理设置</a-button></div>
    </a-spin>
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
.proxy-grid { display: grid; gap: 10px; }
.proxy-card { display: grid; grid-template-columns: 28px minmax(0, 1fr) auto; align-items: center; gap: 12px; padding: 14px; border: 1px solid var(--line, #e5e7eb); border-radius: 10px; background: var(--surface, #fff); }
.proxy-card > :deep(.anticon) { color: var(--primary, #1677ff); font-size: 20px; }
.proxy-card > div { min-width: 0; }
.proxy-card strong, .proxy-card small { display: block; }
.proxy-card small { margin: 3px 0 8px; color: var(--text-3, #98a2b3); font-size: 11px; }
.panel-actions { display: flex; justify-content: flex-end; margin-top: 16px; }
@media (max-width: 680px) { .network-panel { padding-inline: 14px; } .proxy-card { grid-template-columns: 24px minmax(0, 1fr); } .proxy-card > .ant-btn { grid-column: 2; justify-self: start; } }
</style>
