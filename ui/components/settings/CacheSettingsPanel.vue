<script setup lang="ts">
import { computed, onMounted, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { ReloadOutlined } from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText, formatSize, unwrapData } from '../../formatters.js'

interface CachePolicy {
  enabled: boolean
  max_entries: number
}

interface CacheStats {
  bytes: number
  entries: number
  file_fingerprints_bytes: number
  file_fingerprints_entries: number
  remote_cache_bytes: number
  remote_cache_entries: number
}

const loading = shallowRef(false)
const saving = shallowRef(false)
const savedEnabled = shallowRef(true)
const settings = reactive<CachePolicy>({ enabled: true, max_entries: 10_000 })
const stats = reactive<CacheStats>({
  bytes: 0,
  entries: 0,
  file_fingerprints_bytes: 0,
  file_fingerprints_entries: 0,
  remote_cache_bytes: 0,
  remote_cache_entries: 0,
})
const requiresDisableConfirmation = computed(() => savedEnabled.value && !settings.enabled)

function applyPolicy(value: unknown) {
  if (!value || typeof value !== 'object') return
  const policy = value as Partial<CachePolicy>
  if (typeof policy.enabled === 'boolean') settings.enabled = policy.enabled
  const maxEntries = Number(policy.max_entries)
  if (Number.isFinite(maxEntries)) settings.max_entries = Math.min(100_000, Math.max(100, Math.trunc(maxEntries)))
}

function applyStats(value: unknown) {
  if (!value || typeof value !== 'object') return
  const data = value as Partial<CacheStats>
  for (const key of Object.keys(stats) as Array<keyof CacheStats>) {
    const nextValue = Number(data[key])
    stats[key] = Number.isFinite(nextValue) ? nextValue : 0
  }
}

async function loadCache() {
  loading.value = true
  try {
    const [settingsData, statsData] = await Promise.all([
      bridge.invoke('get_cache_settings').then(unwrapData),
      bridge.invoke('get_metadata_cache_stats').then(unwrapData),
    ])
    applyStats(statsData)
    applyPolicy(statsData?.policy)
    applyPolicy(settingsData)
    savedEnabled.value = settings.enabled
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    loading.value = false
  }
}

async function saveSettings() {
  const maxEntries = Math.trunc(Number(settings.max_entries))
  if (!Number.isFinite(maxEntries) || maxEntries < 100 || maxEntries > 100_000) {
    message.warning('最大缓存条目数须在 100–100000 之间')
    return
  }

  saving.value = true
  try {
    const data = unwrapData(await bridge.invoke('update_cache_settings', {
      enabled: settings.enabled,
      max_entries: maxEntries,
    }))
    applyPolicy(data?.policy || data)
    settings.max_entries = maxEntries
    savedEnabled.value = settings.enabled

    const statsData = unwrapData(await bridge.invoke('get_metadata_cache_stats'))
    applyStats(statsData)
    applyPolicy(statsData?.policy)
    savedEnabled.value = settings.enabled
    message.success('缓存设置已保存')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    saving.value = false
  }
}

async function clearCache() {
  loading.value = true
  try {
    const data = unwrapData(await bridge.invoke('clear_metadata_cache'))
    applyStats(data)
    message.success('元数据缓存已清理')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    loading.value = false
  }
}

onMounted(loadCache)
</script>

<template>
  <section class="setting-section">
    <div class="section-lead">
      <strong>元数据缓存</strong>
      <span>仅包含 GCID 指纹与远端目录索引，不缓存文件内容。</span>
    </div>

    <div class="setting-row">
      <div>
        <strong>启用缓存</strong>
        <span>关闭时不再读写缓存，但会保留最大条目数配置。</span>
      </div>
      <a-switch v-model:checked="settings.enabled" aria-label="启用或关闭元数据缓存" />
    </div>

    <a-form class="settings-form" layout="vertical">
      <a-form-item label="最大缓存条目数">
        <a-input-number
          v-model:value="settings.max_entries"
          :min="100"
          :max="100000"
          :precision="0"
          class="entries-input"
        />
      </a-form-item>

      <a-popconfirm
        v-if="requiresDisableConfirmation"
        title="关闭缓存会清理可重建的元数据缓存，文件与上传记录不会受影响。确定保存？"
        ok-text="关闭并清理"
        cancel-text="取消"
        @confirm="saveSettings"
      >
        <a-button type="primary" :loading="saving">保存缓存设置</a-button>
      </a-popconfirm>
      <a-button v-else type="primary" :loading="saving" @click="saveSettings">保存缓存设置</a-button>
    </a-form>

    <div class="cache-stats">
      <div><span>总占用</span><strong>{{ formatSize(stats.bytes) }}</strong></div>
      <div><span>GCID 指纹</span><strong>{{ stats.file_fingerprints_entries }} 项</strong></div>
      <div><span>目录索引</span><strong>{{ stats.remote_cache_entries }} 项</strong></div>
    </div>
    <a-space>
      <a-button :loading="loading" @click="loadCache"><ReloadOutlined />刷新</a-button>
      <a-popconfirm
        title="只会清理元数据缓存，不会删除文件或上传记录。"
        ok-text="清理"
        cancel-text="取消"
        @confirm="clearCache"
      >
        <a-button danger :loading="loading">清理缓存</a-button>
      </a-popconfirm>
    </a-space>
  </section>
</template>

<style scoped>
.setting-section { max-width: 760px; padding: 8px 18px 36px 24px; }
.section-lead { margin-bottom: 28px; }
.section-lead strong, .section-lead span, .setting-row strong, .setting-row span { display: block; }
.section-lead strong { font-size: 18px; }
.section-lead span, .setting-row span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; }
.setting-row { display: flex; align-items: center; justify-content: space-between; gap: 24px; min-height: 54px; margin-bottom: 18px; }
.settings-form { max-width: 520px; }
.entries-input { width: 100%; }
.cache-stats { display: grid; grid-template-columns: repeat(3, 1fr); margin: 28px 0; border-block: 1px solid var(--line, #e7e8eb); }
.cache-stats > div { padding: 18px 16px; border-right: 1px solid var(--line, #e7e8eb); }
.cache-stats > div:last-child { border-right: 0; }
.cache-stats span, .cache-stats strong { display: block; }
.cache-stats span { color: var(--text-3, #98a2b3); font-size: 11px; }
.cache-stats strong { margin-top: 5px; font-size: 16px; }
</style>
