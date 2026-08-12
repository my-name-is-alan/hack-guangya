<script setup lang="ts">
import { onMounted, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { bridge } from '../../bridge.js'
import { errorText, unwrapData } from '../../formatters.js'

const loading = shallowRef(false)
const saving = shallowRef(false)
const settings = reactive({
  filename_obfuscation_enabled: false,
  pending_restores: 0,
})

function applySettings(value: unknown) {
  if (!value || typeof value !== 'object') return
  const data = value as Partial<typeof settings>
  settings.filename_obfuscation_enabled = data.filename_obfuscation_enabled === true
  settings.pending_restores = Math.max(0, Number(data.pending_restores || 0))
}

async function loadSettings() {
  loading.value = true
  try {
    applySettings(unwrapData(await bridge.invoke('get_offline_settings')))
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    loading.value = false
  }
}

async function saveSettings() {
  saving.value = true
  try {
    applySettings(unwrapData(await bridge.invoke('update_offline_settings', {
      filename_obfuscation_enabled: settings.filename_obfuscation_enabled,
    })))
    message.success('离线下载设置已保存')
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
      <strong>离线下载保护</strong>
      <span>针对磁力和 ED2K 的文件名审核场景；HTTP/HTTPS 离线任务不受影响。</span>
    </div>

    <div class="setting-row">
      <div>
        <strong>文件名混淆</strong>
        <span>提交时使用随机安全名称，离线成功并取得云端文件 ID 后自动恢复原名称。</span>
      </div>
      <a-switch
        v-model:checked="settings.filename_obfuscation_enabled"
        aria-label="启用或关闭离线文件名混淆"
        :loading="loading"
      />
    </div>

    <a-alert
      type="info"
      show-icon
      message="恢复任务会持久化"
      :description="settings.pending_restores
        ? `当前有 ${settings.pending_restores} 个任务等待恢复名称；应用重启后会继续。`
        : '当前没有等待恢复名称的任务。'"
    />

    <a-button class="save-button" type="primary" :loading="saving" @click="saveSettings">保存离线设置</a-button>
  </section>
</template>

<style scoped>
/* 骨架样式（setting-section / section-lead / setting-row）已提升为全局类。 */
.save-button { margin-top: 22px; }
</style>
