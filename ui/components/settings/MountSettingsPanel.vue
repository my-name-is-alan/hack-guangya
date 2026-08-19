<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { CopyOutlined, DeleteOutlined, FolderOpenOutlined, PlusOutlined, ReloadOutlined, SyncOutlined } from '@antdv-next/icons'
import { bridge, isTauri } from '../../bridge.js'
import { errorText, unwrapData } from '../../formatters.js'
import CloudFolderPicker from '../cloud/CloudFolderPicker.vue'

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

interface VirtualLibraryMapping {
  id: string
  name: string
  source_dir_id: string
  source_path: string
  local_path: string
  emby_path?: string
  include_metadata: boolean
  enabled: boolean
}

interface VirtualLibraryStatus {
  running: boolean
  last_sync_at?: number | null
  strm_files: number
  metadata_files: number
  skipped_files: number
  error?: string | null
}

interface VirtualLibraryInfo {
  strm_endpoint?: string
  strm_base_url: string
  strm_port?: number
  strm_running?: boolean
  strm_error?: string | null
  strm_configured?: boolean
  emby_upstream: string
  emby_api_key_configured?: boolean
  gateway_endpoint?: string
  gateway_running?: boolean
  gateway_error?: string | null
  refresh_minutes: number
  virtual_root?: string
  mappings: VirtualLibraryMapping[]
  statuses: Record<string, VirtualLibraryStatus>
}

const loading = shallowRef(false)
const saving = shallowRef(false)
const nativeBusy = shallowRef(false)
const mountMode = shallowRef<'virtual' | 'native' | 'webdav'>('virtual')
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
const virtual = reactive<VirtualLibraryInfo>({
  strm_endpoint: '',
  strm_base_url: '',
  strm_port: 18096,
  strm_running: false,
  strm_error: null,
  strm_configured: false,
  emby_upstream: 'http://127.0.0.1:8096',
  emby_api_key_configured: false,
  gateway_endpoint: '',
  gateway_running: false,
  gateway_error: null,
  refresh_minutes: 15,
  virtual_root: '',
  mappings: [],
  statuses: {},
})
const embyApiKeyInput = shallowRef('')
const virtualBusy = reactive<Record<string, boolean>>({})
const virtualForm = reactive({
  open: false,
  id: '',
  name: '',
  source_dir_id: '',
  source_path: '',
  source_label: '',
  local_path: '',
  emby_path: '',
  include_metadata: false,
  enabled: true,
  cloudPickerOpen: false,
})
let unsubscribe: null | (() => void) = null

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

const strmEndpoint = computed(() => {
  if (virtual.strm_endpoint) return virtual.strm_endpoint
  return virtual.strm_base_url ? `${virtual.strm_base_url}/strm/` : ''
})

const strmPlaceholder = computed(() => (isTauri
  ? `留空使用本机 http://127.0.0.1:${virtual.strm_port || 18096}`
  : `${typeof window === 'undefined' ? 'http://192.168.1.10:8080' : window.location.origin}`))

const gatewayEndpoint = computed(() => virtual.gateway_endpoint || (virtual.strm_base_url ? `${virtual.strm_base_url}/` : ''))

const strmStatus = computed(() => {
  if (isTauri) {
    if (virtual.strm_error) return { type: 'error' as const, title: 'STRM 直链与 Emby 网关未运行', detail: virtual.strm_error }
    if (virtual.strm_running) return { type: 'success' as const, title: 'STRM 直链与 Emby 网关运行中', detail: '' }
    return { type: 'warning' as const, title: 'STRM 直链服务正在启动', detail: '' }
  }
  if (virtual.gateway_error) return { type: 'error' as const, title: 'Emby 网关未运行', detail: virtual.gateway_error }
  if (virtual.strm_configured) return { type: 'success' as const, title: 'STRM 直链已配置，Emby 网关运行中', detail: '' }
  return {
    type: 'warning' as const,
    title: '请先填写 STRM 直链地址',
    detail: '填写 Emby 服务器和播放客户端都能访问到本服务的地址（例如 http://192.168.1.10:8080），保存后再同步虚拟库。',
  }
})

/** 端点信息（紧凑展示 + 复制），替代原先塞进状态条的大段文字。 */
const virtualEndpoints = computed(() => [
  { label: 'Emby 网关', value: gatewayEndpoint.value, hint: '客户端把它当 Emby 服务器，播放全程 302 直链' },
  { label: 'STRM 直链', value: strmEndpoint.value ? `${strmEndpoint.value}<fileId>?sign=…` : '', copy: strmEndpoint.value, hint: 'STRM 文件内容指向这里' },
  { label: 'Emby 上游', value: virtual.emby_upstream, hint: '浏览 / 搜索 / 元数据请求转发目标' },
].filter((item) => item.value))

async function loadInfo() {
  loading.value = true
  try {
    const [loadedMount, loadedNative, loadedVirtual] = await Promise.all([
      bridge.invoke('get_mount_info'),
      bridge.invoke('get_native_mount_info'),
      bridge.invoke('get_virtual_library_info'),
    ])
    const mountValue = unwrapData(loadedMount) as MountInfo
    Object.assign(info, mountValue)
    credentials.username = mountValue.username
    credentials.password = ''
    Object.assign(native, unwrapData(loadedNative) as NativeMountInfo)
    Object.assign(virtual, unwrapData(loadedVirtual) as VirtualLibraryInfo)
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
  if (!isTauri && nativePassword.value.length < 12) {
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

function resetVirtualForm() {
  Object.assign(virtualForm, {
    open: true,
    id: '',
    name: '',
    source_dir_id: '',
    source_path: '',
    source_label: '',
    local_path: '',
    emby_path: '',
    include_metadata: false,
    enabled: true,
    cloudPickerOpen: false,
  })
}

function editVirtualMapping(mapping: VirtualLibraryMapping) {
  Object.assign(virtualForm, {
    open: true,
    id: mapping.id,
    name: mapping.name,
    source_dir_id: mapping.source_dir_id,
    source_path: mapping.source_path,
    source_label: mapping.source_path || mapping.name,
    local_path: mapping.local_path,
    emby_path: mapping.emby_path || '',
    include_metadata: mapping.include_metadata,
    enabled: mapping.enabled,
    cloudPickerOpen: false,
  })
}

function selectVirtualSource(value: { id: string, label: string, path: string }) {
  if (!value.id) {
    message.warning('请选择根目录下的具体云端目录，不能直接选择整个云盘')
    return
  }
  virtualForm.source_dir_id = value.id
  virtualForm.source_path = value.path || value.label
  virtualForm.source_label = value.label
  if (!virtualForm.name) virtualForm.name = value.path.split('/').filter(Boolean).at(-1) || '虚拟库'
}

async function selectVirtualTarget() {
  const selected = await bridge.invoke('select_virtual_library_target')
  if (selected) virtualForm.local_path = String(selected)
}

async function saveVirtualMapping() {
  if (!virtualForm.source_dir_id) { message.error('请选择云端目录'); return }
  if (!virtualForm.local_path.trim()) { message.error('请填写本地虚拟库目录'); return }
  const key = virtualForm.id || 'new'
  virtualBusy[key] = true
  try {
    Object.assign(virtual, unwrapData(await bridge.invoke('upsert_virtual_library_mapping', {
      mapping: {
        id: virtualForm.id,
        name: virtualForm.name,
        source_dir_id: virtualForm.source_dir_id,
        source_path: virtualForm.source_path,
        local_path: virtualForm.local_path,
        emby_path: virtualForm.emby_path,
        include_metadata: virtualForm.include_metadata,
        enabled: virtualForm.enabled,
      },
    })) as VirtualLibraryInfo)
    virtualForm.open = false
    message.success('虚拟库配置已保存')
  }
  catch (reason) { message.error(errorText(reason)) }
  finally { virtualBusy[key] = false }
}

async function saveVirtualSettings() {
  virtualBusy.settings = true
  try {
    Object.assign(virtual, unwrapData(await bridge.invoke('update_virtual_library_settings', {
      refresh_minutes: virtual.refresh_minutes,
      strm_base_url: virtual.strm_base_url,
      emby_upstream: virtual.emby_upstream,
      emby_api_key: embyApiKeyInput.value,
    })) as VirtualLibraryInfo)
    embyApiKeyInput.value = ''
    message.success('虚拟库设置已保存，下次同步会按新直链地址重写 STRM')
  }
  catch (reason) { message.error(errorText(reason)) }
  finally { virtualBusy.settings = false }
}

async function syncVirtualMapping(mapping: VirtualLibraryMapping) {
  virtualBusy[mapping.id] = true
  try {
    Object.assign(virtual, unwrapData(await bridge.invoke('sync_virtual_library', { id: mapping.id })) as VirtualLibraryInfo)
    message.success('虚拟库同步已在后台开始')
  }
  catch (reason) { message.error(errorText(reason)) }
  finally { virtualBusy[mapping.id] = false }
}

async function removeVirtualMapping(mapping: VirtualLibraryMapping) {
  virtualBusy[mapping.id] = true
  try {
    Object.assign(virtual, unwrapData(await bridge.invoke('remove_virtual_library_mapping', { id: mapping.id })) as VirtualLibraryInfo)
    message.success('虚拟库配置已移除，本地已生成文件保持不变')
  }
  catch (reason) { message.error(errorText(reason)) }
  finally { virtualBusy[mapping.id] = false }
}

function virtualStatus(mapping: VirtualLibraryMapping) {
  return virtual.statuses?.[mapping.id] || { running: false, strm_files: 0, metadata_files: 0, skipped_files: 0 }
}

function syncTime(value?: number | null) {
  return value ? new Date(value * 1000).toLocaleString() : '尚未同步'
}

onMounted(async () => {
  await loadInfo()
  unsubscribe = await bridge.subscribe((event: any) => {
    if (event?.type === 'virtual-library' && event.data) Object.assign(virtual, event.data)
  })
})
onBeforeUnmount(() => unsubscribe?.())
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
        { label: 'Emby 虚拟库（STRM）', value: 'virtual' },
        { label: '原生挂载（rclone / FUSE）', value: 'native' },
        { label: 'WebDAV 兼容', value: 'webdav' },
      ]"
      class="mount-mode"
    />

    <template v-if="mountMode === 'virtual'">
      <a-alert
        :type="strmStatus.type"
        show-icon
        :message="strmStatus.title"
        :description="strmStatus.detail || undefined"
        class="mount-alert"
      />

      <div v-if="virtualEndpoints.length" class="endpoint-list">
        <div v-for="endpoint in virtualEndpoints" :key="endpoint.label" class="endpoint-row">
          <span class="endpoint-label">{{ endpoint.label }}</span>
          <code class="endpoint-value" :title="endpoint.hint">{{ endpoint.value }}</code>
          <CopyOutlined class="copy-icon" @click="copyText(endpoint.copy || endpoint.value)" />
          <small>{{ endpoint.hint }}</small>
        </div>
      </div>

      <div class="virtual-toolbar">
        <div>
          <strong>云端目录 → 本地 STRM 虚拟库</strong>
          <span>视频/音频生成同名 <code>.strm</code> 直链文件，Emby 只需加入这一个目录。</span>
        </div>
        <a-button type="primary" @click="resetVirtualForm"><PlusOutlined />添加虚拟库</a-button>
      </div>

      <a-form class="virtual-settings" layout="vertical">
        <div class="virtual-settings-grid">
          <a-form-item label="STRM 直链地址">
            <a-input v-model:value="virtual.strm_base_url" :placeholder="strmPlaceholder" />
          </a-form-item>
          <a-form-item label="Emby 原始地址">
            <a-input v-model:value="virtual.emby_upstream" placeholder="http://127.0.0.1:8096" />
          </a-form-item>
          <a-form-item label="Emby API 密钥">
            <a-input-password
              v-model:value="embyApiKeyInput"
              autocomplete="off"
              :placeholder="virtual.emby_api_key_configured ? '已配置；留空保持不变，输入 off 清除' : 'Emby 后台 → API 密钥 中生成'"
            />
          </a-form-item>
          <a-form-item label="自动刷新间隔">
            <a-input-number v-model:value="virtual.refresh_minutes" :min="1" :max="1440" addon-after="分钟" class="refresh-input" />
          </a-form-item>
        </div>
        <div class="virtual-settings-actions">
          <span>配置了 API 密钥后，同步有变更会自动通知 Emby 增量扫描。</span>
          <a-button type="primary" :loading="virtualBusy.settings" @click="saveVirtualSettings">保存设置</a-button>
        </div>
      </a-form>

      <a-empty v-if="!virtual.mappings.length" description="尚未配置虚拟库" class="virtual-empty" />
      <div v-else class="virtual-list">
        <article v-for="mapping in virtual.mappings" :key="mapping.id" class="virtual-card">
          <header>
            <div>
              <strong>{{ mapping.name }}</strong>
              <span>{{ mapping.source_path }} → {{ mapping.local_path }}</span>
            </div>
            <a-tag :color="virtualStatus(mapping).running ? 'processing' : virtualStatus(mapping).error ? 'error' : mapping.enabled ? 'green' : 'default'">
              {{ virtualStatus(mapping).running ? '同步中' : virtualStatus(mapping).error ? '同步失败' : mapping.enabled ? '已启用' : '已停用' }}
            </a-tag>
          </header>
          <div class="virtual-stats">
            <span><small>STRM</small><strong>{{ virtualStatus(mapping).strm_files }}</strong></span>
            <span><small>元数据</small><strong>{{ virtualStatus(mapping).metadata_files }}</strong></span>
            <span><small>已排除</small><strong>{{ virtualStatus(mapping).skipped_files }}</strong></span>
            <span><small>上次同步</small><strong>{{ syncTime(virtualStatus(mapping).last_sync_at) }}</strong></span>
          </div>
          <a-alert v-if="virtualStatus(mapping).error" type="error" :message="virtualStatus(mapping).error" />
          <footer>
            <span>{{ mapping.include_metadata ? '保留 NFO、图片、字幕等元数据' : '排除所有元数据，只生成 STRM' }}</span>
            <a-space>
              <a-button size="small" :disabled="virtualStatus(mapping).running" @click="editVirtualMapping(mapping)">编辑</a-button>
              <a-button size="small" type="primary" :loading="virtualStatus(mapping).running || virtualBusy[mapping.id]" :disabled="!mapping.enabled" @click="syncVirtualMapping(mapping)"><SyncOutlined />立即同步</a-button>
              <a-popconfirm title="只移除配置，本地已生成的 STRM/元数据不会删除。" ok-text="移除配置" cancel-text="取消" @confirm="removeVirtualMapping(mapping)">
                <a-button size="small" danger :disabled="virtualStatus(mapping).running"><DeleteOutlined /></a-button>
              </a-popconfirm>
            </a-space>
          </footer>
        </article>
      </div>

      <a-collapse ghost class="usage-collapse">
        <a-collapse-panel key="emby-usage" header="Emby 使用说明">
          <ul class="usage-list">
            <li v-if="isTauri">把本地虚拟库目录作为媒体库加入 Emby——只需这一个目录，不需要映射挂载盘。</li>
            <li v-else>把 {{ virtual.virtual_root || '/virtual-library' }} 映射给 Emby 容器并作为媒体库加入——只需这一个目录。</li>
            <li><strong>推荐播放方式</strong>：客户端把上方“Emby 网关”地址当作 Emby 服务器，播放全程 302 直链，数据不经过 Emby 和本机。</li>
            <li>直连 Emby 原生地址（如 8096）也能播放，部分客户端的播放数据会经 Emby 服务器中转。</li>
            <li v-if="isTauri">Emby 在 Docker 或其他机器上时，把 STRM 直链地址改成本机局域网地址（如 http://192.168.x.x:18096），保存后自动监听所有网卡。</li>
            <li v-else>STRM 直链地址必须是 Emby 和播放设备都能访问到的本服务地址。</li>
            <li>修改直链地址后，下次同步会自动重写全部 STRM；同步不会删除目录里非光鸭生成的文件。</li>
          </ul>
        </a-collapse-panel>
      </a-collapse>
    </template>

    <template v-else-if="mountMode === 'native'">
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
              <FolderOpenOutlined class="copy-icon" @click="!native.running && selectNativeTarget()" />
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
            <a-input v-model:value="native.rclone_path" :disabled="native.running" placeholder="留空时优先使用软件内置版本">
              <template v-if="isTauri" #suffix>
                <FolderOpenOutlined class="copy-icon" @click="!native.running && selectRcloneBinary()" />
              </template>
            </a-input>
          </a-form-item>
        </div>

        <a-form-item v-if="!isTauri" label="当前 WebDAV 挂载密码">
          <a-input-password
            v-model:value="nativePassword"
            autocomplete="current-password"
            placeholder="只用于本次启动，不会保存"
            :disabled="native.running"
          />
          <div class="field-help">服务端只保存密码哈希，因此启动原生挂载时需要再次输入；该值不会写入配置。</div>
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
            <template #suffix><CopyOutlined class="copy-icon" @click="copyText(info.endpoint)" /></template>
          </a-input>
        </a-form-item>
        <div class="two-columns">
          <a-form-item label="用户名">
            <a-input v-model:value="credentials.username" autocomplete="username" :maxlength="64">
              <template #suffix><CopyOutlined class="copy-icon" @click="copyText(credentials.username)" /></template>
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

    <p v-if="mountMode !== 'virtual'" class="section-footnote">
      读取、列目录、创建、覆盖、重命名、移动、复制和删除都会映射到光鸭云盘；只读原生挂载会在本地文件系统层阻止所有修改。
    </p>

    <a-modal v-model:open="virtualForm.open" :title="virtualForm.id ? '编辑虚拟库' : '添加虚拟库'" width="min(680px, 94vw)" ok-text="保存" cancel-text="取消" :confirm-loading="virtualBusy[virtualForm.id || 'new']" @ok="saveVirtualMapping">
      <a-form layout="vertical">
        <a-form-item label="名称"><a-input v-model:value="virtualForm.name" placeholder="例如：电影虚拟库" :maxlength="80" /></a-form-item>
        <a-form-item label="云端源目录" required>
          <a-input :value="virtualForm.source_label" readonly placeholder="选择光鸭云盘中的媒体目录" @click="virtualForm.cloudPickerOpen = true">
            <template #suffix><FolderOpenOutlined /></template>
          </a-input>
        </a-form-item>
        <a-form-item label="本地虚拟库目录" required>
          <a-input v-model:value="virtualForm.local_path" :placeholder="isTauri ? '选择 Emby 扫描的本地目录' : `${virtual.virtual_root || '/virtual-library'}/movies`">
            <template v-if="isTauri" #suffix><FolderOpenOutlined class="copy-icon" @click="selectVirtualTarget" /></template>
          </a-input>
          <div class="field-help">同步只清理光鸭生成且云端已删除的文件，不影响目录里的其他文件。</div>
        </a-form-item>
        <a-form-item label="Emby 内路径（刷新通知）">
          <a-input v-model:value="virtualForm.emby_path" :placeholder="isTauri ? '与本地目录相同，或 Emby 容器内路径，如 /visual_media' : '/visual_media'" />
          <div class="field-help">该目录在 Emby 看到的路径，用于变更后通知增量扫描（需 API 密钥）；留空则等 Emby 定时扫描。</div>
        </a-form-item>
        <div class="virtual-option-row">
          <span><strong>保留元数据</strong><small>开启后下载 NFO、海报、字幕等小文件；关闭后只生成视频/音频 STRM。</small></span>
          <a-switch v-model:checked="virtualForm.include_metadata" />
        </div>
        <div class="virtual-option-row">
          <span><strong>启用自动刷新</strong><small>按上方刷新间隔更新目录；也可以随时手动同步。</small></span>
          <a-switch v-model:checked="virtualForm.enabled" />
        </div>
      </a-form>
    </a-modal>
    <CloudFolderPicker v-model:open="virtualForm.cloudPickerOpen" title="选择虚拟库云端源目录" @select="selectVirtualSource" />
  </section>
</template>

<style scoped>
/* 骨架样式已提升为全局类；本面板内容较宽，仅覆盖最大宽度。 */
.setting-section { max-width: 860px; }
.mount-mode { margin-bottom: 18px; }
.mount-alert { max-width: 720px; margin-bottom: 18px; }
.mount-form { max-width: 720px; }
.native-form { padding: 18px; border: 1px solid var(--line, #e5e5e5); border-radius: 12px; background: var(--surface, #fff); }
.form-heading { display: flex; align-items: flex-start; justify-content: space-between; gap: 16px; margin-bottom: 18px; }
.form-heading strong, .form-heading span { display: block; }
.form-heading span { margin-top: 4px; color: var(--text-3, #737373); font-size: 12px; }
.two-columns { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.three-columns { display: grid; grid-template-columns: 1.35fr .8fr .8fr; gap: 16px; }
.three-columns :deep(.ant-input-number), .two-columns :deep(.ant-input-number) { width: 100%; }
.copy-icon { color: var(--text-3, #737373); cursor: pointer; }
.field-help { margin-top: 6px; color: var(--text-3, #737373); font-size: 12px; line-height: 1.55; }
.field-help code { padding: 1px 4px; border-radius: 4px; background: var(--surface-muted, #fafafa); }
.inline-alert { margin: 0 0 18px; }
.native-notes { max-width: 720px; margin: 18px 0 24px; padding: 14px 16px; border-radius: 10px; background: var(--surface-muted, #fafafa); }
.native-notes strong { font-size: 13px; }
.native-notes ul { margin: 8px 0 0; padding-left: 18px; color: var(--text-2, #525252); font-size: 12px; line-height: 1.75; }
.platform-guide { max-width: 720px; margin: 30px 0 22px; padding-top: 24px; border-top: 1px solid var(--line, #e5e5e5); }
.guide-title { display: flex; align-items: baseline; justify-content: space-between; gap: 16px; }
.guide-title span, .guide-note { color: var(--text-3, #737373); font-size: 12px; }
.command-box { display: flex; align-items: flex-start; gap: 10px; padding: 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 8px; background: var(--surface-muted, #fafafa); }
.command-box pre { min-width: 0; flex: 1; overflow: auto; margin: 0; color: var(--text-1, #262626); font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 11px; line-height: 1.65; white-space: pre-wrap; word-break: break-all; }
.guide-note { margin: 9px 0 0; }
.support-alert { max-width: 720px; margin-top: 22px; }
.virtual-toolbar { display: flex; max-width: 780px; align-items: flex-start; justify-content: space-between; gap: 20px; margin-bottom: 14px; }
.virtual-toolbar > div { display: grid; gap: 4px; }
.virtual-toolbar span { color: var(--text-3, #737373); font-size: 12px; }
.endpoint-list { display: grid; max-width: 780px; gap: 6px; margin-bottom: 18px; padding: 12px 14px; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; background: var(--surface, #fff); }
.endpoint-row { display: flex; min-width: 0; align-items: center; gap: 10px; }
.endpoint-label { flex: 0 0 76px; color: var(--text-3, #737373); font-size: 12px; }
.endpoint-value { min-width: 0; overflow: hidden; padding: 2px 6px; border-radius: 4px; background: var(--surface-muted, #fafafa); font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.endpoint-row small { overflow: hidden; flex: 1; color: var(--text-3, #737373); font-size: 11px; text-align: right; text-overflow: ellipsis; white-space: nowrap; }
.virtual-settings { max-width: 780px; margin-bottom: 16px; padding: 14px 16px 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; background: var(--surface-muted, #fafafa); }
.virtual-settings-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 0 16px; }
.virtual-settings-grid :deep(.ant-form-item) { margin-bottom: 12px; }
.refresh-input { width: 100%; }
.virtual-settings-actions { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.virtual-settings-actions span { color: var(--text-3, #737373); font-size: 12px; }
.usage-collapse { max-width: 780px; margin-top: 18px; }
.usage-collapse :deep(.ant-collapse-header) { padding-inline-start: 0 !important; color: var(--text-2, #525252); font-size: 13px; }
.usage-list { margin: 0; padding-left: 18px; color: var(--text-2, #525252); font-size: 12px; line-height: 1.9; }
.section-footnote { max-width: 720px; margin: 18px 0 0; color: var(--text-3, #737373); font-size: 12px; line-height: 1.6; }
.virtual-empty { max-width: 780px; padding: 32px 0; }
.virtual-list { display: grid; max-width: 780px; gap: 12px; }
.virtual-card { display: grid; gap: 12px; padding: 15px; border: 1px solid var(--line, #e5e5e5); border-radius: 12px; background: var(--surface, #fff); }
.virtual-card header, .virtual-card footer, .virtual-option-row { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
.virtual-card header > div, .virtual-option-row > span { display: grid; min-width: 0; gap: 3px; }
.virtual-card header span, .virtual-card footer > span, .virtual-option-row small { overflow-wrap: anywhere; color: var(--text-3, #737373); font-size: 11px; }
.virtual-stats { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
.virtual-stats > span { display: grid; gap: 3px; padding: 9px 10px; border-radius: 8px; background: var(--surface-muted, #fafafa); }
.virtual-stats small { color: var(--text-3, #737373); font-size: 10px; }
.virtual-stats strong { overflow: hidden; font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }
.virtual-option-row { min-height: 62px; margin-top: 10px; padding: 10px 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 9px; }
@media (max-width: 760px) {
  .two-columns, .three-columns { grid-template-columns: 1fr; gap: 0; }
  .native-form { padding: 14px; }
  .virtual-toolbar, .virtual-card footer { align-items: stretch; flex-direction: column; }
  .virtual-stats { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
</style>
