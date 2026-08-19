<script setup lang="ts">
import { computed, onMounted, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { CopyOutlined, ReloadOutlined, SendOutlined } from '@antdv-next/icons'
import { bridge, isTauri } from '../../bridge.js'
import { errorText, unwrapData } from '../../formatters.js'

type TelegramSettings = {
  enabled: boolean
  mode: string
  chat_id: string
  api_base_url: string
  api_id: string
  bot_token_configured: boolean
  api_hash_configured: boolean
  configured: boolean
  notify: Record<string, boolean>
  connected: boolean
  bot_username: string
  last_error: string | null
  webhook: { secret: string, path: string, gateway_path: string }
  enabled_managed_by_environment?: boolean
  mode_managed_by_environment?: boolean
  bot_token_managed_by_environment?: boolean
  api_base_url_managed_by_environment?: boolean
  api_id_managed_by_environment?: boolean
  api_hash_managed_by_environment?: boolean
  chat_id_managed_by_environment?: boolean
}

const loading = shallowRef(true)
const saving = shallowRef(false)
const testing = shallowRef(false)
const regenerating = shallowRef(false)
const settings = shallowRef<TelegramSettings | null>(null)
const form = reactive({
  enabled: false,
  mode: 'bot_api',
  bot_token: '',
  api_base_url: '',
  api_id: '',
  api_hash: '',
  chat_id: '',
  notify: {
    organize: true,
    review: true,
    auth: true,
    emby_new: true,
    emby_play: true,
    emby_login: true,
  } as Record<string, boolean>,
})

const NOTIFY_LABELS: Array<{ key: string, label: string, help: string }> = [
  { key: 'organize', label: '入库完成', help: '云盘整理完成（转移 + 刮削）' },
  { key: 'review', label: '识别失败', help: '识别待处理 / 整理失败，附重新整理按钮' },
  { key: 'auth', label: '登录失效', help: '光鸭登录态失效时提醒并支持扫码重登' },
  { key: 'emby_new', label: 'Emby 入库', help: '来自 Emby webhook 的 library.new' },
  { key: 'emby_play', label: 'Emby 播放', help: '播放开始 / 停止 / 暂停' },
  { key: 'emby_login', label: 'Emby 登录', help: '用户登录成功 / 失败' },
]

const webhookWebUrl = computed(() => {
  const path = settings.value?.webhook?.path || ''
  if (!path) return ''
  return isTauri ? '' : `${window.location.origin}${path}`
})
const webhookGatewayUrl = computed(() => {
  const path = settings.value?.webhook?.gateway_path || ''
  if (!path) return ''
  return `http://<本机或服务器IP>:18096${path}`
})

function applySettings(data: TelegramSettings) {
  settings.value = data
  form.enabled = data.enabled === true
  form.mode = data.mode === 'mtproto' ? 'mtproto' : 'bot_api'
  form.api_base_url = data.api_base_url || ''
  form.api_id = String(data.api_id || '')
  form.chat_id = data.chat_id || ''
  form.bot_token = ''
  form.api_hash = ''
  for (const key of Object.keys(form.notify)) {
    form.notify[key] = data.notify?.[key] !== false
  }
}

async function reload() {
  loading.value = true
  try {
    applySettings(unwrapData(await bridge.invoke('get_telegram_settings')) as TelegramSettings)
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    loading.value = false
  }
}

function buildPayload(extra: Record<string, unknown> = {}) {
  const payload: Record<string, unknown> = {
    enabled: form.enabled,
    mode: form.mode,
    api_base_url: form.api_base_url.trim(),
    api_id: form.api_id.trim(),
    chat_id: form.chat_id.trim(),
    notify: { ...form.notify },
    ...extra,
  }
  if (form.bot_token.trim()) payload.bot_token = form.bot_token.trim()
  if (form.api_hash.trim()) payload.api_hash = form.api_hash.trim()
  return payload
}

async function saveSettings(extra: Record<string, unknown> = {}) {
  saving.value = true
  try {
    const data = unwrapData(await bridge.invoke('update_telegram_settings', { input: buildPayload(extra) })) as TelegramSettings
    applySettings(data)
    message.success('Telegram 设置已保存')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    saving.value = false
  }
}

async function sendTest() {
  testing.value = true
  try {
    const data = unwrapData(await bridge.invoke('test_telegram_message')) as { bot_username?: string }
    message.success(`测试消息已发送${data.bot_username ? `（@${data.bot_username}）` : ''}，请在 Telegram 中确认`)
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    testing.value = false
  }
}

async function regenerateSecret() {
  regenerating.value = true
  try {
    const data = unwrapData(await bridge.invoke('update_telegram_settings', { input: { regenerate_webhook_secret: true } })) as TelegramSettings
    applySettings(data)
    message.success('Webhook 密钥已重新生成，请同步更新 Emby 里的地址')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    regenerating.value = false
  }
}

async function copyText(value: string, label: string) {
  if (!value) return
  try {
    await navigator.clipboard.writeText(value)
    message.success(`${label}已复制`)
  } catch {
    message.warning('无法自动复制，请手动选中复制')
  }
}

onMounted(reload)
</script>

<template>
  <section class="setting-section">
    <div class="setting-row">
      <div>
        <strong>Telegram Bot 通知与交互</strong>
        <span>入库 / 识别失败 / 登录失效 / Emby 事件推送，并支持 /status /logs /login 与 re 命令交互。</span>
      </div>
      <a-switch
        v-model:checked="form.enabled"
        :disabled="loading || settings?.enabled_managed_by_environment"
        aria-label="启用或关闭 Telegram Bot"
      />
    </div>

    <a-alert
      v-if="settings"
      :type="settings.connected ? 'success' : settings.last_error ? 'warning' : 'info'"
      show-icon
      class="status-alert"
    >
      <template #message>
        <template v-if="settings.connected">已连接{{ settings.bot_username ? `：@${settings.bot_username}` : '' }}</template>
        <template v-else-if="settings.last_error">未连接：{{ settings.last_error }}</template>
        <template v-else>尚未连接（保存并启用后自动连接）</template>
      </template>
    </a-alert>

    <a-form class="settings-form" layout="vertical">
      <a-form-item label="接入模式">
        <a-radio-group v-model:value="form.mode" :disabled="settings?.mode_managed_by_environment">
          <a-radio-button value="bot_api">Bot API（直接 Bot Token）</a-radio-button>
          <a-radio-button value="mtproto">MTProto（TG API）</a-radio-button>
        </a-radio-group>
        <div class="field-help">
          Bot API 通过 HTTPS 调用官方或自建反代地址，支持 HTTP/SOCKS5 全局代理；
          MTProto 使用 api_id / api_hash 以 bot 身份直连 Telegram 数据中心，仅支持 SOCKS5 代理。
        </div>
      </a-form-item>

      <a-form-item label="Bot Token">
        <a-input-password
          v-model:value="form.bot_token"
          :placeholder="settings?.bot_token_configured ? '已配置；留空表示不修改，输入 off 清除' : '例如 123456789:AAF…（@BotFather 创建）'"
          :disabled="settings?.bot_token_managed_by_environment"
        />
      </a-form-item>

      <a-form-item v-if="form.mode === 'bot_api'" label="Bot API 地址（可选反代）">
        <a-input
          v-model:value="form.api_base_url"
          placeholder="留空使用 https://api.telegram.org"
          :disabled="settings?.api_base_url_managed_by_environment"
        />
      </a-form-item>

      <template v-if="form.mode === 'mtproto'">
        <a-form-item label="API ID">
          <a-input
            v-model:value="form.api_id"
            placeholder="在 my.telegram.org → API development tools 获取"
            :disabled="settings?.api_id_managed_by_environment"
          />
        </a-form-item>
        <a-form-item label="API Hash">
          <a-input-password
            v-model:value="form.api_hash"
            :placeholder="settings?.api_hash_configured ? '已配置；留空表示不修改，输入 off 清除' : '32 位十六进制'"
            :disabled="settings?.api_hash_managed_by_environment"
          />
        </a-form-item>
      </template>

      <a-form-item label="Chat ID（通知目标与命令白名单）">
        <a-input
          v-model:value="form.chat_id"
          placeholder="给机器人发送 /start 可获取；多个用逗号分隔，第一个接收通知"
          :disabled="settings?.chat_id_managed_by_environment"
        />
      </a-form-item>

      <a-form-item label="通知类型">
        <div class="notify-grid">
          <label v-for="item in NOTIFY_LABELS" :key="item.key" class="notify-item">
            <a-checkbox v-model:checked="form.notify[item.key]" />
            <span class="notify-text">
              <strong>{{ item.label }}</strong>
              <span>{{ item.help }}</span>
            </span>
          </label>
        </div>
      </a-form-item>

      <div class="actions">
        <a-button type="primary" :loading="saving" @click="saveSettings()">保存 Telegram 设置</a-button>
        <a-button :loading="testing" :disabled="saving" @click="sendTest">
          <template #icon><SendOutlined /></template>
          发送测试消息
        </a-button>
      </div>
    </a-form>

    <a-divider />

    <div class="setting-row">
      <div>
        <strong>Emby Webhook 通知</strong>
        <span>在 Emby 后台把 Webhook 地址指向本服务，即可推送 Emby 入库 / 播放 / 登录事件。</span>
      </div>
      <a-button size="small" :loading="regenerating" @click="regenerateSecret">
        <template #icon><ReloadOutlined /></template>
        重新生成密钥
      </a-button>
    </div>
    <a-form class="settings-form" layout="vertical">
      <a-form-item v-if="webhookWebUrl" label="Webhook 地址（管理端口）">
        <div class="copy-field">
          <a-input :value="webhookWebUrl" readonly />
          <a-button @click="copyText(webhookWebUrl, 'Webhook 地址')">
            <template #icon><CopyOutlined /></template>
            复制
          </a-button>
        </div>
      </a-form-item>
      <a-form-item label="Webhook 地址（Emby 网关端口）">
        <div class="copy-field">
          <a-input :value="webhookGatewayUrl" readonly />
          <a-button @click="copyText(webhookGatewayUrl, 'Webhook 地址')">
            <template #icon><CopyOutlined /></template>
            复制
          </a-button>
        </div>
        <div class="field-help">把 &lt;本机或服务器IP&gt; 替换为 Emby 能访问到本服务的地址；桌面端请使用网关端口地址。</div>
      </a-form-item>
    </a-form>
    <a-alert class="setup-guide" type="info" show-icon>
      <template #message>在 Emby 中这样配置</template>
      <template #description>
        <ol>
          <li>进入 Emby 控制台 → 通知（Notifications）→ 添加 Webhook / Webhooks 通知。</li>
          <li>URL 填上面的 Webhook 地址（含 token 参数），请求内容类型保持默认即可（JSON 与表单都支持）。</li>
          <li>勾选需要的事件：新媒体入库、播放开始/停止、用户登录等，保存后可用“发送测试”验证。</li>
        </ol>
      </template>
    </a-alert>
  </section>
</template>

<style scoped>
/* 骨架样式（setting-section / setting-row / settings-form）已提升为全局类。 */
.status-alert { max-width: 620px; margin-bottom: 16px; }
.field-help { margin-top: 6px; color: var(--text-3, #737373); font-size: 12px; line-height: 1.5; }
.notify-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 10px 18px; }
.notify-item { display: flex; align-items: flex-start; gap: 8px; cursor: pointer; }
.notify-text { display: flex; flex-direction: column; line-height: 1.4; }
.notify-text > span { color: var(--text-3, #737373); font-size: 12px; }
.actions { display: flex; gap: 10px; }
.copy-field { display: flex; gap: 8px; }
.copy-field :deep(.ant-input) { font-family: ui-monospace, SFMono-Regular, Consolas, monospace; }
.setup-guide { max-width: 620px; margin-top: 22px; }
.setup-guide ol { margin: 8px 0 0; padding-left: 20px; line-height: 1.8; }
</style>
