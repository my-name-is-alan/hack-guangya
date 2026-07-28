<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, shallowRef, useTemplateRef, watch } from 'vue'
import { message } from 'antdv-next'
import { MobileOutlined, QrcodeOutlined, ReloadOutlined } from '@antdv-next/icons'
import appLogo from '../../../src-tauri/icons/128x128.png'
import { bridge } from '../../bridge.js'
import { errorText, pick, unwrapData } from '../../formatters.js'
import { useSessionStore } from '../../stores/session'

const session = useSessionStore()
const mode = shallowRef<'qr' | 'sms'>('qr')
const captchaFrame = useTemplateRef<HTMLIFrameElement>('captchaFrame')
const qr = reactive({
  loading: false,
  value: '',
  userCode: '—',
  message: '正在获取二维码…',
  remaining: 0,
})
const sms = reactive({
  phone: '',
  code: '',
  requestId: '',
  sending: false,
  submitting: false,
  cooldown: 0,
  error: '',
})
const captcha = reactive({
  url: '',
  state: '',
  phone: '',
  origin: '',
  action: 'send' as 'send' | 'login',
  width: 420,
  height: 520,
})

let pollTimer: ReturnType<typeof setInterval> | null = null
let expiryTimer: ReturnType<typeof setInterval> | null = null
let smsTimer: ReturnType<typeof setInterval> | null = null

const phoneValid = computed(() => /^1\d{10}$/.test(sms.phone.trim()))

function isTrustedCaptchaHostname(hostname: string) {
  const normalizedHostname = hostname.toLowerCase()
  return normalizedHostname === 'guangyapan.com' || normalizedHostname.endsWith('.guangyapan.com')
}

function createCaptchaState() {
  const cryptoApi = window.crypto
  if (typeof cryptoApi?.randomUUID === 'function') {
    try {
      return cryptoApi.randomUUID()
    }
    catch {
      // 部分 WebView 暴露了 randomUUID，但可能因运行上下文限制而拒绝调用。
    }
  }
  if (typeof cryptoApi?.getRandomValues === 'function') {
    const bytes = cryptoApi.getRandomValues(new Uint8Array(16))
    return Array.from(bytes, byte => byte.toString(16).padStart(2, '0')).join('')
  }
  throw new Error('当前环境不支持安全随机数，已拒绝打开安全验证')
}

function clearQrTimers() {
  if (pollTimer) clearInterval(pollTimer)
  if (expiryTimer) clearInterval(expiryTimer)
  pollTimer = null
  expiryTimer = null
}

function captchaCallbackUrl() {
  const url = new URL('./captcha-callback.html', window.location.href)
  url.search = ''
  url.hash = ''
  return url
}

function closeCaptcha() {
  Object.assign(captcha, { url: '', state: '', phone: '', origin: '', width: 420, height: 520 })
}

function openCaptcha(data: Record<string, any>, action: 'send' | 'login') {
  const rawUrl = String(data.captcha_url || data.captchaUrl || data.url || '')
  if (!rawUrl) throw new Error('安全验证需要人机验证，但服务端没有返回验证页面')
  const challengeUrl = new URL(rawUrl)
  if (challengeUrl.protocol !== 'https:') throw new Error('安全验证地址不是 HTTPS，已拒绝打开')
  if (!isTrustedCaptchaHostname(challengeUrl.hostname)) {
    throw new Error('安全验证地址不属于 guangyapan.com，已拒绝打开')
  }
  const state = createCaptchaState()
  challengeUrl.searchParams.set('redirect_uri', captchaCallbackUrl().toString())
  challengeUrl.searchParams.set('state', state)
  Object.assign(captcha, {
    url: challengeUrl.toString(),
    state,
    phone: sms.phone.trim(),
    origin: challengeUrl.origin,
    action,
    width: 420,
    height: 520,
  })
}

function handleCaptchaMessage(event: MessageEvent) {
  if (!captcha.url || event.source !== captchaFrame.value?.contentWindow || event.origin !== captcha.origin) return
  const payload = event.data
  if (!payload || payload.action !== 'responsiveLayout' || typeof payload.data !== 'object') return
  const width = Number(payload.data.width)
  const height = Number(payload.data.height)
  if (Number.isFinite(width)) captcha.width = Math.min(720, Math.max(320, width))
  if (Number.isFinite(height)) captcha.height = Math.min(680, Math.max(360, height))
}

async function handleCaptchaLoad() {
  const frame = captchaFrame.value
  if (!frame?.contentWindow || !captcha.url) return
  let currentUrl: URL
  try {
    currentUrl = new URL(frame.contentWindow.location.href)
  }
  catch {
    // challenge 仍在跨域页面，等待回跳到本地 callback。
    return
  }
  const callbackUrl = captchaCallbackUrl()
  if (currentUrl.origin !== callbackUrl.origin || currentUrl.pathname !== callbackUrl.pathname) return
  const params = new URLSearchParams(currentUrl.search || currentUrl.hash.replace(/^#/, ''))
  if (!captcha.state || params.get('state') !== captcha.state) {
    sms.error = '安全验证状态校验失败，请重新获取验证码'
    closeCaptcha()
    return
  }
  const token = String(params.get('captcha_token') || '')
  const action = captcha.action
  const phone = captcha.phone
  closeCaptcha()
  if (!token) {
    sms.error = '安全验证未返回 token，请重试'
    return
  }
  if (phone !== sms.phone.trim()) {
    sms.error = '手机号已变更，请重新获取验证码'
    return
  }
  if (action === 'send') await sendSms(token)
  else await submitSms(token)
}

async function finishLogin() {
  clearQrTimers()
  await session.connect()
  if (session.state.logged_in) message.success('登录成功')
}

async function refreshQr() {
  clearQrTimers()
  Object.assign(qr, { loading: true, value: '', userCode: '—', message: '正在获取二维码…', remaining: 0 })
  try {
    const data = unwrapData(await bridge.login())
    const deviceCode = String(pick(data, ['device_code', 'deviceCode'], ''))
    const value = String(pick(data, [
      'verification_uri_complete', 'verificationUriComplete', 'verification_url',
      'verificationUrl', 'verification_uri', 'verificationUri',
    ], ''))
    if (!deviceCode || !value) throw new Error('官方没有返回完整扫码信息')
    Object.assign(qr, {
      loading: false,
      value,
      userCode: pick(data, ['user_code', 'userCode'], '—'),
      message: '使用光鸭 App 扫码确认',
      remaining: Number(data.expires_in || 120),
    })
    pollTimer = setInterval(async () => {
      try {
        const result = unwrapData(await bridge.invoke('poll_device_login', { device_code: deviceCode }))
        if (result.authenticated) await finishLogin()
        else if (result.message) qr.message = String(result.message)
      }
      catch (reason) {
        clearQrTimers()
        qr.message = errorText(reason)
      }
    }, Math.max(2, Number(data.interval || 3)) * 1000)
    expiryTimer = setInterval(() => {
      qr.remaining -= 1
      if (qr.remaining <= 0) void refreshQr()
    }, 1000)
  }
  catch (reason) {
    qr.loading = false
    qr.message = errorText(reason)
  }
}

async function sendSms(captchaToken = '') {
  if (!phoneValid.value || sms.cooldown > 0) return
  sms.sending = true
  sms.error = ''
  try {
    const data = unwrapData(await bridge.invoke('request_sms_code', {
      phone: sms.phone.trim(),
      captcha_token: captchaToken || undefined,
    }))
    if (data.captcha_required === true) {
      const nextToken = String(data.captcha_token || data.captchaToken || '')
      if (data.captcha_url || data.captchaUrl || data.url) {
        openCaptcha(data, 'send')
        return
      }
      if (nextToken && nextToken !== captchaToken) {
        await sendSms(nextToken)
        return
      }
      throw new Error('无法完成短信安全验证，请重试')
    }
    sms.requestId = String(data.request_id || data.requestId || data.biz_id || '')
    if (!sms.requestId) throw new Error('服务端未返回短信验证请求 ID')
    sms.cooldown = 60
    if (smsTimer) clearInterval(smsTimer)
    smsTimer = setInterval(() => {
      sms.cooldown -= 1
      if (sms.cooldown <= 0 && smsTimer) {
        clearInterval(smsTimer)
        smsTimer = null
      }
    }, 1000)
    message.success('验证码已发送')
  }
  catch (reason) {
    sms.error = errorText(reason)
  }
  finally {
    sms.sending = false
  }
}

async function submitSms(captchaToken = '') {
  if (!phoneValid.value || !/^\d{4,8}$/.test(sms.code.trim())) return
  sms.submitting = true
  sms.error = ''
  try {
    const result = unwrapData(await bridge.invoke('login_with_sms', {
      phone: sms.phone.trim(),
      code: sms.code.trim(),
      request_id: sms.requestId,
      captcha_token: captchaToken || undefined,
    }))
    if (result.captcha_required === true) {
      const nextToken = String(result.captcha_token || result.captchaToken || '')
      if (result.captcha_url || result.captchaUrl || result.url) {
        openCaptcha(result, 'login')
        return
      }
      if (nextToken && nextToken !== captchaToken) {
        await submitSms(nextToken)
        return
      }
      throw new Error('无法完成登录安全验证，请重试')
    }
    if (result.authenticated === false) throw new Error(result.message || '验证码登录失败')
    await finishLogin()
  }
  catch (reason) {
    sms.error = errorText(reason)
  }
  finally {
    sms.submitting = false
  }
}

watch(mode, value => {
  if (value === 'qr') closeCaptcha()
  if (value === 'qr' && !qr.value && !qr.loading) void refreshQr()
})

onMounted(() => {
  window.addEventListener('message', handleCaptchaMessage)
  void refreshQr()
})
onBeforeUnmount(() => {
  clearQrTimers()
  closeCaptcha()
  window.removeEventListener('message', handleCaptchaMessage)
  if (smsTimer) clearInterval(smsTimer)
})
</script>

<template>
  <main class="auth-gate">
    <section class="auth-brand">
      <img :src="appLogo" alt="" />
      <div>
        <strong>光鸭云盘</strong>
        <p>文件、备份与分享，一处完成。</p>
      </div>
    </section>

    <section class="auth-panel" aria-labelledby="auth-title">
      <header>
        <span>登录</span>
        <h1 id="auth-title">连接你的云盘</h1>
      </header>

      <a-tabs v-model:active-key="mode" centered>
        <a-tab-pane key="qr">
          <template #tab><QrcodeOutlined /> 扫码登录</template>
          <div class="qr-login">
            <div class="qr-frame">
              <a-skeleton v-if="qr.loading" active :paragraph="{ rows: 3 }" />
              <a-qrcode v-else-if="qr.value" :value="qr.value" :size="216" error-level="M" />
              <a-result v-else status="warning" title="二维码获取失败" :sub-title="qr.message">
                <template #extra><a-button @click="refreshQr"><ReloadOutlined /> 重试</a-button></template>
              </a-result>
            </div>
            <strong>{{ qr.message }}</strong>
            <span v-if="qr.remaining">{{ qr.remaining }} 秒后自动刷新</span>
            <a-button type="text" size="small" @click="refreshQr"><ReloadOutlined /> 刷新二维码</a-button>
          </div>
        </a-tab-pane>

        <a-tab-pane key="sms">
          <template #tab><MobileOutlined /> 手机验证码</template>
          <a-form class="sms-form" layout="vertical" @submit.prevent="submitSms()">
            <a-form-item label="手机号">
              <a-input v-model:value="sms.phone" size="large" inputmode="numeric" maxlength="11" autocomplete="tel" placeholder="请输入手机号" />
            </a-form-item>
            <a-form-item label="短信验证码" :validate-status="sms.error ? 'error' : undefined" :help="sms.error || undefined">
              <div class="sms-code-row">
                <a-input v-model:value="sms.code" size="large" inputmode="numeric" maxlength="8" autocomplete="one-time-code" placeholder="验证码" @press-enter="submitSms()" />
                <a-button size="large" :disabled="!phoneValid || sms.cooldown > 0" :loading="sms.sending" @click="sendSms()">
                  {{ sms.cooldown > 0 ? `${sms.cooldown}s` : '获取验证码' }}
                </a-button>
              </div>
            </a-form-item>
            <a-button type="primary" size="large" block html-type="submit" :loading="sms.submitting" :disabled="!phoneValid || !/^\d{4,8}$/.test(sms.code.trim())">登录</a-button>
          </a-form>
        </a-tab-pane>
      </a-tabs>

      <a-modal
        :open="Boolean(captcha.url)"
        title="完成安全验证"
        :footer="null"
        :width="Math.min(captcha.width + 48, 760)"
        :mask-closable="false"
        destroy-on-close
        @cancel="closeCaptcha"
      >
        <iframe
          v-if="captcha.url"
          ref="captchaFrame"
          class="captcha-frame"
          :src="captcha.url"
          :style="{ height: `${captcha.height}px` }"
          title="短信登录安全验证"
          sandbox="allow-scripts allow-forms allow-same-origin"
          referrerpolicy="no-referrer"
          @load="handleCaptchaLoad"
        />
      </a-modal>

      <footer>登录信息仅保存在当前设备</footer>
    </section>
  </main>
</template>

<style scoped>
.auth-gate { display: grid; width: 100%; max-width: 100vw; height: 100vh; min-width: 0; min-height: 0; overflow-x: hidden; overflow-y: auto; box-sizing: border-box; grid-template-columns:minmax(320px, .9fr) minmax(520px, 1.1fr); background: var(--app-bg, #f7f7f8); }
.auth-brand { display: flex; align-items: center; justify-content: center; gap: 18px; padding: 48px; border-right: 1px solid var(--line, #e7e8eb); background: var(--sidebar-bg, #fff0f6); }
.auth-brand img { width: 72px; height: 72px; object-fit: contain; }
.auth-brand strong { display: block; font-size: 28px; letter-spacing: -.03em; }
.auth-brand p { margin: 8px 0 0; color: var(--text-2, #667085); font-size: 15px; }
.auth-panel { width: min(440px, calc(100% - 64px)); align-self: center; justify-self: center; padding: 32px 0; }
.auth-panel header > span { color: var(--primary, #52c41a); font-weight: 700; }
.auth-panel h1 { margin: 8px 0 26px; font-size: 30px; letter-spacing: -.03em; }
.qr-login { display: flex; min-height: 340px; align-items: center; flex-direction: column; justify-content: center; gap: 10px; }
.qr-frame { display: grid; width: 248px; min-height: 248px; place-items: center; margin-bottom: 8px; padding: 16px; background: #fff; }
.qr-login > span { color: var(--text-3, #98a2b3); font-size: 12px; }
.sms-form { min-height: 340px; padding-top: 30px; }
.sms-code-row { display: grid; grid-template-columns:1fr auto; gap: 10px; }
.captcha-frame { display: block; width: 100%; border: 0; border-radius: 8px; background: #fff; }
.auth-panel footer { margin-top: 20px; color: var(--text-3, #98a2b3); font-size: 12px; text-align: center; }
@media (max-height: 680px) {
  .auth-panel { align-self: start; }
}
@media (max-width: 860px) {
  .auth-gate { grid-template-columns:1fr; }
  .auth-brand { justify-content: flex-start; padding: 20px 28px; border-right: 0; border-bottom: 1px solid var(--line, #e7e8eb); }
  .auth-brand img { width: 42px; height: 42px; }
  .auth-brand strong { font-size: 19px; }
  .auth-brand p { display: none; }
  .auth-panel { width: min(440px, calc(100% - 40px)); }
}
</style>
