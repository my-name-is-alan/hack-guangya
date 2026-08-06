<script setup>
import { computed, reactive, ref, watch } from 'vue'
import { CheckCircleOutlined, CloseCircleOutlined, GlobalOutlined, LoadingOutlined, ReloadOutlined } from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText } from '../../formatters.js'

const open = defineModel('open', { type: Boolean, default: false })
const props = defineProps({ proxyUrl: { type: String, default: undefined } })

const targets = [
  { key: 'github', label: 'GitHub', description: '更新检查与代码托管服务' },
  { key: 'tmdb', label: 'TMDB', description: 'API、海报与背景图' },
  { key: 'tg', label: 'Telegram', description: 'Bot API 网络路径' },
  { key: 'hdhive', label: 'HDHive', description: '自动投稿与回执服务' },
]
const results = reactive(Object.fromEntries(targets.map(({ key }) => [key, null])))
const running = ref(false)
const resolvedProxyUrl = ref('')
const hasProxyOverride = computed(() => props.proxyUrl !== undefined)

const hasFailure = computed(() => targets.some(({ key }) => {
  const result = results[key]
  return result && !result.loading && result.configured !== false && !result.reachable
}))

function requestArgs(target) {
  const args = { target }
  const proxy = resolvedProxyUrl.value
  if (proxy) args.proxy_url = proxy
  return args
}

function failedResult(target, error) {
  return {
    target,
    success: false,
    reachable: false,
    status: 0,
    latency_ms: 0,
    proxy: resolvedProxyUrl.value ? '已配置代理' : '直连',
    message: `检测失败：${errorText(error)}`,
  }
}

async function resolveProxy() {
  if (hasProxyOverride.value) {
    resolvedProxyUrl.value = String(props.proxyUrl || '').trim()
    return
  }
  try {
    const preferences = await bridge.invoke('get_network_preferences')
    resolvedProxyUrl.value = String(preferences?.proxy_url || '').trim()
  } catch {
    resolvedProxyUrl.value = ''
  }
}

async function runOne(target) {
  results[target] = { target, loading: true }
  try {
    results[target] = await bridge.invoke('test_network', requestArgs(target))
  } catch (error) {
    results[target] = failedResult(target, error)
  }
}

async function runAll() {
  running.value = true
  await resolveProxy()
  targets.forEach(({ key }) => { results[key] = { target: key, loading: true } })
  await Promise.all(targets.map(({ key }) => runOne(key)))
  running.value = false
}

function retry(target) {
  void runOne(target)
}

function statusOf(result) {
  if (!result || result.loading) return 'testing'
  if (result.configured === false) return 'unconfigured'
  if (result.reachable && result.success) return 'success'
  if (result.reachable) return 'warning'
  return 'error'
}

watch(open, (visible) => {
  if (visible) void runAll()
})
watch(() => props.proxyUrl, (value) => {
  if (hasProxyOverride.value) resolvedProxyUrl.value = String(value || '').trim()
})
</script>

<template>
  <a-modal v-model:open="open" title="网络可用性检测" width="min(620px, 94vw)" :footer="null" :mask-closable="!running">
    <div class="network-test-dialog">
      <div class="dialog-lead">
        <div>
          <strong>正在检测所有网络服务</strong>
          <span>{{ resolvedProxyUrl ? '所有检测和 HDHive 请求都会使用同一个代理。' : '当前使用直连；保存代理后会用于所有检测和 HDHive 请求。' }}</span>
        </div>
        <GlobalOutlined class="dialog-icon" />
      </div>

      <div class="test-list">
        <article v-for="target in targets" :key="target.key" class="test-item">
          <div class="test-item-icon" :class="`status-${statusOf(results[target.key])}`">
            <LoadingOutlined v-if="results[target.key]?.loading" spin />
            <CheckCircleOutlined v-else-if="statusOf(results[target.key]) === 'success'" />
            <CloseCircleOutlined v-else-if="statusOf(results[target.key]) === 'error'" />
            <GlobalOutlined v-else />
          </div>
          <div class="test-item-copy">
            <div class="test-item-title">
              <strong>{{ target.label }}</strong>
              <a-tag
                v-if="results[target.key] && !results[target.key]?.loading"
                :color="statusOf(results[target.key]) === 'success' ? 'success' : statusOf(results[target.key]) === 'warning' ? 'warning' : statusOf(results[target.key]) === 'unconfigured' ? 'default' : 'error'"
              >{{ statusOf(results[target.key]) === 'success' ? '可用' : statusOf(results[target.key]) === 'warning' ? '可达' : statusOf(results[target.key]) === 'unconfigured' ? '未配置' : '失败' }}</a-tag>
            </div>
            <small>{{ target.description }}</small>
            <p v-if="results[target.key] && !results[target.key]?.loading">{{ results[target.key].message }}<em v-if="results[target.key].latency_ms"> · {{ results[target.key].latency_ms }} ms</em></p>
            <p v-else class="testing-copy">检测中…</p>
          </div>
          <a-button
            v-if="results[target.key] && !results[target.key]?.loading && results[target.key]?.configured !== false && !results[target.key]?.reachable"
            size="small"
            @click="retry(target)"
          ><ReloadOutlined />重试</a-button>
        </article>
      </div>

      <div class="dialog-footer">
        <span v-if="hasFailure" class="failure-hint">失败项目可以单独重试</span>
        <a-button :loading="running" @click="runAll"><ReloadOutlined />重新检测全部</a-button>
      </div>
    </div>
  </a-modal>
</template>

<style scoped>
.network-test-dialog { padding-top: 4px; }
.dialog-lead { display: flex; align-items: flex-start; justify-content: space-between; gap: 18px; margin-bottom: 18px; }
.dialog-lead strong, .dialog-lead span { display: block; }
.dialog-lead strong { font-size: 16px; }
.dialog-lead span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.55; }
.dialog-icon { color: var(--primary, #1677ff); font-size: 24px; }
.test-list { display: grid; gap: 8px; }
.test-item { display: grid; grid-template-columns: 30px minmax(0, 1fr) auto; align-items: center; gap: 11px; padding: 11px 12px; border: 1px solid var(--line, #e5e7eb); border-radius: 10px; background: var(--surface, #fff); }
.test-item-icon { display: grid; width: 28px; height: 28px; place-items: center; border-radius: 50%; color: var(--text-3, #98a2b3); background: var(--surface-muted, #f5f6f8); font-size: 16px; }
.test-item-icon.status-success { color: #16a34a; background: #ecfdf3; }
.test-item-icon.status-warning { color: #d97706; background: #fffbeb; }
.test-item-icon.status-error { color: #dc2626; background: #fef2f2; }
.test-item-icon.status-unconfigured { color: var(--text-3, #98a2b3); background: var(--surface-muted, #f5f6f8); }
.test-item-copy { min-width: 0; }
.test-item-title { display: flex; align-items: center; gap: 8px; }
.test-item-title strong { font-size: 13px; }
.test-item-copy small { display: block; margin-top: 2px; color: var(--text-3, #98a2b3); font-size: 11px; }
.test-item-copy p { margin: 4px 0 0; overflow: hidden; color: var(--text-2, #667085); font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }
.test-item-copy p em { color: var(--text-3, #98a2b3); font-style: normal; }
.testing-copy { color: var(--text-3, #98a2b3) !important; }
.dialog-footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-top: 16px; }
.failure-hint { color: #b45309; font-size: 12px; }
@media (max-width: 520px) { .test-item { grid-template-columns: 28px minmax(0, 1fr); } .test-item > .ant-btn { grid-column: 2; justify-self: start; } .dialog-footer { align-items: flex-start; flex-direction: column; } }
</style>
