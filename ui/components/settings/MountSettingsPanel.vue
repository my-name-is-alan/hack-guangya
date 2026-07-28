<script setup lang="ts">
import { computed, onMounted, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { CopyOutlined, FolderOpenOutlined, ReloadOutlined } from '@antdv-next/icons'
import { bridge, isTauri } from '../../bridge.js'
import { errorText, unwrapData } from '../../formatters.js'

interface MountInfo {
  enabled: boolean
  running: boolean
  configured?: boolean
  local_only?: boolean
  endpoint: string
  username: string
  password: string
  password_hint?: string
  error?: string | null
}

interface NativeMountInfo {
  supported: boolean
  enabled?: boolean
  available: boolean
  running: boolean
  engine: string
  platform: string
  rclone_available: boolean
  fuse_available: boolean
  version: string
  prerequisite: string
  target: string
  access_mode: 'read_only' | 'read_write'
  vfs_cache_mode: 'off' | 'minimal' | 'writes' | 'full'
  transfers: number
  read_streams: number
  cache_size_gb: number
  rclone_path: string
  started_at?: number | null
  error?: string | null
}

const loading = shallowRef(false)
const saving = shallowRef(false)
const nativeBusy = shallowRef(false)
const mountMode = shallowRef<'native' | 'webdav'>('native')
const activePlatform = shallowRef('windows')
const nativePassword = shallowRef('')
const info = reactive<MountInfo>({
  enabled: true,
  running: false,
  configured: false,
  local_only: true,
  endpoint: '',
  username: '',
  password: '',
  password_hint: '',
  error: null,
})
const credentials = reactive({
  username: '',
  password: '',
})
const native = reactive<NativeMountInfo>({
  supported: true,
  enabled: true,
  available: false,
  running: false,
  engine: 'rclone',
  platform: '',
  rclone_available: false,
  fuse_available: false,
  version: '',
  prerequisite: '',
  target: '',
  access_mode: 'read_write',
  vfs_cache_mode: 'full',
  transfers: 4,
  read_streams: 4,
  cache_size_gb: 20,
  rclone_path: '',
  started_at: null,
  error: null,
})

const commands = computed<Record<string, string>>(() => ({
  windows: `net use Z: "${info.endpoint}" /user:${credentials.username || info.username} * /persistent:yes`,
  macos: `mkdir -p "$HOME/Guangya"\nmount_webdav "${info.endpoint}" "$HOME/Guangya"`,
  linux: `sudo mkdir -p /mnt/guangya\nsudo mount -t davfs "${info.endpoint}" /mnt/guangya`,
  docker: `# 在宿主机运行；同一 Compose 网络可改用 http://guangya-sync:19090/dav/\nrclone config create guangya webdav \\\n  url "${info.endpoint}" vendor other user "${credentials.username || info.username}" \\\n  pass "$(rclone obscure '<挂载密码>')"\n\nrclone mount guangya: /mnt/guangya --vfs-cache-mode full`,
}))

const nativeStatus = computed(() => {
  if (native.running) return { type: 'success', title: `已挂载到 ${native.target}`, detail: `${native.engine} ${native.version}` }
  if (!native.available) return { type: 'warning', title: '原生挂载环境未就绪', detail: native.error || native.prerequisite }
  return { type: 'info', title: '原生挂载可以启动', detail: `${native.version} · ${native.prerequisite}` }
})

async function loadInfo() {
  loading.value = true
  try {
    const [loadedMount, loadedNative] = await Promise.all([
      bridge.invoke('get_mount_info'),
      bridge.invoke('get_native_mount_info'),
    ])
    const mountValue = unwrapData(loadedMount) as MountInfo
    Object.assign(info, mountValue)
    credentials.username = mountValue.username
    credentials.password = ''
    Object.assign(native, unwrapData(loadedNative) as NativeMountInfo)
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    loading.value = false
  }
}

async function copyText(value: string) {
  try {
    await navigator.clipboard.writeText(value)
    message.success('已复制')
  } catch {
    message.error('复制失败，请手动复制')
  }
}

async function saveCredentials() {
  const username = credentials.username.trim()
  if (username.length < 3 || username.length > 64 || username.includes(':')) {
    message.error('用户名需为 3 到 64 个字符，且不能包含冒号')
    return
  }
  if (credentials.password.length < 12 || credentials.password.length > 256) {
    message.error('密码需为 12 到 256 个字符')
    return
  }
  saving.value = true
  try {
    Object.assign(info, unwrapData(await bridge.invoke('update_mount_credentials', {
      username,
      password: credentials.password,
    })))
    credentials.username = info.username
    credentials.password = ''
    message.success('WebDAV 账号密码已保存，已有挂载需要重新连接')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    saving.value = false
  }
}

function nativeOptions() {
  return {
    rclone_path: native.rclone_path,
    target: native.target,
    access_mode: native.access_mode,
    vfs_cache_mode: native.vfs_cache_mode,
    transfers: native.transfers,
    read_streams: native.read_streams,
    cache_size_gb: native.cache_size_gb,
  }
}

async function saveNativeOptions(notify = true) {
  Object.assign(native, unwrapData(await bridge.invoke('update_native_mount_options', {
    options: nativeOptions(),
  })))
  if (notify) message.success('原生挂载参数已保存')
}

async function startNativeMount() {
  if (nativePassword.value.length < 12) {
    message.error('请输入当前 WebDAV 挂载密码')
    return
  }
  nativeBusy.value = true
  try {
    await saveNativeOptions(false)
    Object.assign(native, unwrapData(await bridge.invoke('start_native_mount', {
      password: nativePassword.value,
    })))
    nativePassword.value = ''
    message.success(`已原生挂载到 ${native.target}`)
  } catch (reason) {
    message.error(errorText(reason))
    await loadInfo()
  } finally {
    nativeBusy.value = false
  }
}

async function stopNativeMount() {
  nativeBusy.value = true
  try {
    Object.assign(native, unwrapData(await bridge.invoke('stop_native_mount')))
    message.success('原生挂载已卸载')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    nativeBusy.value = false
  }
}

async function selectNativeTarget() {
  const selected = await bridge.invoke('select_native_mount_target')
  if (selected) native.target = String(selected)
}

async function selectRcloneBinary() {
  const selected = await bridge.invoke('select_rclone_binary')
  if (selected) native.rclone_path = String(selected)
}

onMounted(loadInfo)
</script>

<template>
  <section class="setting-section">
    <div class="section-lead">
      <strong>本地目录挂载</strong>
      <span>原生挂载适合长期使用；WebDAV 兼容模式可直接交给系统或其他客户端连接。</span>
    </div>

    <a-segmented
      v-model:value="mountMode"
      :options="[
        { label: '原生挂载（rclone / FUSE）', value: 'native' },
        { label: 'WebDAV 兼容', value: 'webdav' },
      ]"
      class="mount-mode"
    />

    <template v-if="mountMode === 'native'">
      <a-alert
        :type="nativeStatus.type"
        show-icon
        :message="nativeStatus.title"
        :description="nativeStatus.detail"
        class="mount-alert"
      />

      <a-form class="mount-form native-form" layout="vertical">
        <div class="form-heading">
          <div>
            <strong>挂载位置与权限</strong>
            <span>Windows 可填写未占用盘符，例如 X:；macOS/Linux 使用绝对目录。</span>
          </div>
          <a-tag :color="native.running ? 'green' : 'default'">{{ native.running ? '运行中' : native.platform || '检测中' }}</a-tag>
        </div>

        <a-form-item label="盘符或挂载目录">
          <a-input v-model:value="native.target" :disabled="native.running" placeholder="X: 或 /mnt/guangya">
            <template v-if="isTauri" #suffix>
              <button
                type="button"
                class="input-icon-button"
                :disabled="native.running"
                aria-label="选择挂载目录"
                title="选择挂载目录"
                @click="selectNativeTarget"
              ><FolderOpenOutlined /></button>
            </template>
          </a-input>
        </a-form-item>

        <a-form-item label="访问权限">
          <a-radio-group v-model:value="native.access_mode" button-style="solid" :disabled="native.running">
            <a-radio-button value="read_only">只读</a-radio-button>
            <a-radio-button value="read_write">读写</a-radio-button>
          </a-radio-group>
          <div class="field-help">只读模式会向 rclone 传入 <code>--read-only</code>，所有写入、删除和重命名都会被拒绝。</div>
        </a-form-item>

        <div class="three-columns">
          <a-form-item label="VFS 缓存">
            <a-select v-model:value="native.vfs_cache_mode" :disabled="native.running">
              <a-select-option value="full">完整缓存（推荐）</a-select-option>
              <a-select-option value="writes">仅写入缓存</a-select-option>
              <a-select-option value="minimal">最小缓存</a-select-option>
              <a-select-option value="off">关闭缓存</a-select-option>
            </a-select>
          </a-form-item>
          <a-form-item label="上传并行">
            <a-input-number v-model:value="native.transfers" :min="1" :max="16" :disabled="native.running" />
          </a-form-item>
          <a-form-item label="读取并行">
            <a-input-number v-model:value="native.read_streams" :min="1" :max="16" :disabled="native.running" />
          </a-form-item>
        </div>

        <div class="two-columns">
          <a-form-item label="缓存上限（GB）">
            <a-input-number v-model:value="native.cache_size_gb" :min="1" :max="1024" :disabled="native.running" />
          </a-form-item>
          <a-form-item label="rclone 可执行文件">
            <a-input v-model:value="native.rclone_path" :disabled="native.running" :readonly="isTauri" placeholder="留空时优先使用软件内置版本">
              <template v-if="isTauri" #suffix>
                <button
                  type="button"
                  class="input-icon-button"
                  :disabled="native.running"
                  aria-label="选择 rclone 可执行文件"
                  title="选择 rclone 可执行文件"
                  @click="selectRcloneBinary"
                ><FolderOpenOutlined /></button>
              </template>
            </a-input>
            <div v-if="isTauri" class="field-help">自定义程序必须通过文件选择器批准，只在本次应用运行期间有效；重启后会恢复为内置版本。</div>
          </a-form-item>
        </div>

        <a-form-item label="当前 WebDAV 挂载密码">
          <a-input-password
            v-model:value="nativePassword"
            autocomplete="current-password"
            placeholder="只用于本次启动，不会保存"
            :disabled="native.running"
          />
          <div class="field-help">仅用于本次启动原生挂载，不会写入配置；重新启动应用后需要再次输入。</div>
        </a-form-item>

        <a-alert
          v-if="native.access_mode === 'read_write' && ['off', 'minimal'].includes(native.vfs_cache_mode)"
          type="warning"
          show-icon
          message="读写挂载建议使用“完整缓存”或“仅写入缓存”"
          description="关闭或最小缓存时，随机写入、覆盖现有文件和失败重试的兼容性会下降。"
          class="inline-alert"
        />

        <a-space wrap>
          <a-button :loading="loading" @click="loadInfo"><ReloadOutlined />刷新检测</a-button>
          <a-button :disabled="native.running" @click="saveNativeOptions()">保存参数</a-button>
          <a-popconfirm
            v-if="native.running"
            title="确认卸载当前盘符或目录？请先关闭正在使用其中内容的程序。"
            ok-text="卸载"
            cancel-text="取消"
            @confirm="stopNativeMount"
          >
            <a-button danger :loading="nativeBusy">卸载</a-button>
          </a-popconfirm>
          <a-button
            v-else
            type="primary"
            :loading="nativeBusy"
            :disabled="!native.available"
            @click="startNativeMount"
          >
            开始挂载
          </a-button>
        </a-space>
      </a-form>

      <div class="native-notes">
        <strong>运行说明</strong>
        <ul>
          <li>上传并行控制关闭文件后的并行回写；读取并行控制分块读取流数量。</li>
          <li>退出软件时会自动停止 rclone 并卸载，避免留下失联盘符。</li>
          <li v-if="!isTauri">Web/Docker 中的挂载目录属于服务器；Docker 需要显式开放 /dev/fuse，默认安全配置不会授予该权限。</li>
        </ul>
      </div>
    </template>

    <template v-else>
      <a-alert
        v-if="info.error"
        type="error"
        show-icon
        :message="info.error"
        description="请关闭占用端口的程序后重启光鸭，或设置 GUANGYA_WEBDAV_PORT。"
        class="mount-alert"
      />
      <a-alert
        v-else
        :type="info.running ? 'success' : 'warning'"
        show-icon
        :message="!info.configured ? '请先设置 WebDAV 账号密码' : (info.running ? '挂载服务运行中' : '挂载服务正在启动')"
        :description="isTauri ? '服务只监听本机 127.0.0.1，并使用独立账号密码。' : 'WebDAV 与管理页面使用不同端口和不同凭据；Compose 默认只发布到宿主机 127.0.0.1，不直接暴露公网。'"
        class="mount-alert"
      />

      <a-form class="mount-form" layout="vertical">
        <a-form-item label="WebDAV 地址">
          <a-input :value="info.endpoint" readonly>
            <template #suffix>
              <button type="button" class="input-icon-button" aria-label="复制 WebDAV 地址" title="复制 WebDAV 地址" @click="copyText(info.endpoint)"><CopyOutlined /></button>
            </template>
          </a-input>
        </a-form-item>
        <div class="two-columns">
          <a-form-item label="用户名">
            <a-input v-model:value="credentials.username" autocomplete="username" :maxlength="64">
              <template #suffix>
                <button type="button" class="input-icon-button" aria-label="复制 WebDAV 用户名" title="复制 WebDAV 用户名" @click="copyText(credentials.username)"><CopyOutlined /></button>
              </template>
            </a-input>
          </a-form-item>
          <a-form-item label="新密码">
            <a-input-password
              v-model:value="credentials.password"
              autocomplete="new-password"
              :placeholder="info.password_hint || '输入 12 位以上独立密码'"
              :maxlength="256"
            />
          </a-form-item>
        </div>
        <a-space>
          <a-button :loading="loading" @click="loadInfo"><ReloadOutlined />刷新状态</a-button>
          <a-popconfirm
            title="保存后，已经挂载的盘符或目录需要使用新账号密码重新连接。"
            ok-text="保存凭据"
            cancel-text="取消"
            @confirm="saveCredentials"
          >
            <a-button type="primary" :loading="saving">保存账号密码</a-button>
          </a-popconfirm>
        </a-space>
      </a-form>

      <div class="platform-guide">
        <div class="guide-title">
          <strong>WebDAV 挂载命令</strong>
          <span>执行前请确保已经登录光鸭云盘。</span>
        </div>
        <a-tabs v-model:active-key="activePlatform" size="small">
          <a-tab-pane key="windows" tab="Windows" />
          <a-tab-pane key="macos" tab="macOS" />
          <a-tab-pane key="linux" tab="Linux" />
          <a-tab-pane key="docker" tab="Docker / rclone" />
        </a-tabs>
        <div class="command-box">
          <pre>{{ commands[activePlatform] }}</pre>
          <a-button size="small" @click="copyText(commands[activePlatform])"><CopyOutlined />复制</a-button>
        </div>
        <p v-if="activePlatform === 'windows'" class="guide-note">Windows 使用系统 WebClient 映射盘符；需要更完整的文件语义时选择上方“原生挂载”。</p>
        <p v-else-if="activePlatform === 'macos'" class="guide-note">也可以在 Finder 中选择“前往 → 连接服务器”。</p>
        <p v-else-if="activePlatform === 'linux'" class="guide-note">需要先安装 davfs2；原生挂载模式则使用 rclone 与 FUSE。</p>
        <p v-else class="guide-note">Docker 原生挂载必须显式提供 /dev/fuse；只需让其他容器访问时，直接使用私有 WebDAV 地址更安全。</p>
      </div>
    </template>

    <a-alert
      type="info"
      show-icon
      message="文件操作范围"
      description="读取、列目录、创建、覆盖、重命名、移动、复制和删除都会映射到光鸭云盘；只读原生挂载会在本地文件系统层阻止所有修改。"
      class="support-alert"
    />
  </section>
</template>

<style scoped>
.setting-section { width: 100%; max-width: 860px; min-width: 0; overflow-x: hidden; padding: 8px 18px 36px 24px; }
.section-lead { margin-bottom: 18px; }
.section-lead strong, .section-lead span { display: block; }
.section-lead strong { font-size: 18px; }
.section-lead span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; }
.mount-mode { max-width: 100%; margin-bottom: 18px; }
.mount-alert { max-width: 720px; margin-bottom: 18px; }
.mount-form { width: 100%; max-width: 720px; min-width: 0; }
.native-form { padding: 18px; border: 1px solid var(--line, #e7e8eb); border-radius: 12px; background: var(--surface, #fff); }
.form-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; margin-bottom: 18px; }
.form-heading strong, .form-heading span { display: block; }
.form-heading span { margin-top: 4px; color: var(--text-3, #98a2b3); font-size: 12px; }
.two-columns { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.three-columns { display: grid; grid-template-columns: 1.35fr .8fr .8fr; gap: 16px; }
.three-columns :deep(.ant-input-number), .two-columns :deep(.ant-input-number) { width: 100%; }
.input-icon-button { display: inline-grid; width: 40px; height: 40px; place-items: center; padding: 0; border: 0; border-radius: 7px; color: var(--text-3, #98a2b3); background: transparent; cursor: pointer; }
.input-icon-button:hover:not(:disabled) { color: var(--text-1, #20242c); background: var(--surface-muted, #f3f4f6); }
.input-icon-button:focus-visible { outline: 2px solid color-mix(in srgb, var(--primary, #262626) 45%, transparent); outline-offset: 1px; color: var(--text-1, #20242c); background: var(--surface-muted, #f3f4f6); }
.input-icon-button:disabled { opacity: .45; cursor: not-allowed; }
.field-help { margin-top: 6px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.55; }
.field-help code { padding: 1px 4px; border-radius: 4px; background: var(--surface-muted, #f3f4f6); }
.inline-alert { margin: 0 0 18px; }
.native-notes { max-width: 720px; margin: 18px 0 24px; padding: 14px 16px; border-radius: 10px; background: var(--surface-muted, #f7f8fa); }
.native-notes strong { font-size: 13px; }
.native-notes ul { margin: 8px 0 0; padding-left: 18px; color: var(--text-2, #667085); font-size: 12px; line-height: 1.75; }
.platform-guide { max-width: 720px; margin: 30px 0 22px; padding-top: 24px; border-top: 1px solid var(--line, #e7e8eb); }
.guide-title { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; }
.guide-title span, .guide-note { color: var(--text-3, #98a2b3); font-size: 12px; }
.command-box { display: flex; align-items: flex-start; gap: 10px; padding: 12px; border: 1px solid var(--line, #e7e8eb); border-radius: 8px; background: var(--surface-muted, #f7f8fa); }
.command-box pre { min-width: 0; flex: 1; overflow: auto; margin: 0; color: var(--text-1, #20242c); font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 11px; line-height: 1.65; white-space: pre-wrap; word-break: break-all; }
.guide-note { margin: 9px 0 0; }
.support-alert { max-width: 720px; margin-top: 22px; }
@media (max-width: 760px) {
  .setting-section { padding: 6px 2px 28px; }
  .two-columns, .three-columns { grid-template-columns: 1fr; gap: 0; }
  .native-form { padding: 14px; }
  .form-heading, .guide-title { align-items: flex-start; flex-direction: column; gap: 6px; }
  .mount-mode { display: flex; width: 100%; }
  .mount-mode :deep(.ant-segmented-group) { width: 100%; }
  .mount-mode :deep(.ant-segmented-item) { min-width: 0; flex: 1; }
  .command-box { min-width: 0; flex-direction: column; }
  .command-box pre { width: 100%; overflow-x: hidden; }
}
</style>
