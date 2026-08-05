<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, shallowRef } from 'vue'
import { message, Modal } from 'antdv-next'
import {
  CheckCircleOutlined,
  DeleteOutlined,
  EditOutlined,
  KeyOutlined,
  PlusOutlined,
  ReloadOutlined,
} from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText, formatTime, unwrapData } from '../../formatters.js'
import { useFilesStore } from '../../stores/files'

type DeveloperTarget = {
  id: string
  name: string
  token_masked: string
  created_at: number
  updated_at: number
}

type TransferJob = {
  id: string
  target_id: string
  target_name: string
  file_ids: string[]
  file_names: string[]
  total_count: number
  passed_count: number
  rejected_count: number
  pending_count: number
  success_count: number
  skipped_count: number
  status: string
  phase: string
  message?: string | null
  error_code?: number | null
  created_at: number
  updated_at: number
}

type PanelTab = 'tokens' | 'jobs'

const activeTab = shallowRef<PanelTab>('tokens')
const loading = shallowRef(false)
const saving = shallowRef(false)
const testing = shallowRef(false)
const modeSaving = shallowRef(false)
const targetSaving = shallowRef(false)
const jobsLoading = shallowRef(false)
const settingsError = ref('')
const jobsError = ref('')
const settings = reactive({
  configured: false,
  enabled: false,
  requested_enabled: false,
  client_id: '',
  client_secret_set: false,
  account_id: '',
  current_account_id: '',
  account_verified: false,
  account_matches_current: false,
  verified_at: 0,
  managed_by_environment: false,
  client_id_managed_by_environment: false,
  client_secret_managed_by_environment: false,
  targets: [] as DeveloperTarget[],
})
const credentials = reactive({ client_id: '', client_secret: '' })
const filesStore = useFilesStore()
const targetModal = reactive({ open: false, id: '', name: '', token_id: '' })
const jobs = ref<TransferJob[]>([])
let unsubscribe: undefined | (() => void)

const canTest = computed(() => settings.configured && !saving.value && !testing.value)
const probeFileId = computed(() => {
  const record = filesStore.files.find((item) => item?.fileId || item?.id)
  return String(record?.fileId || record?.id || '')
})
const modeStatus = computed(() => {
  if (settings.enabled) return { label: '开发者模式已开启', color: 'success' }
  if (settings.account_verified && !settings.current_account_id) return { label: '无法确认当前账号', color: 'warning' }
  if (settings.account_verified && !settings.account_matches_current) return { label: '绑定账号不一致', color: 'error' }
  if (settings.account_verified) return { label: '当前账号已验证', color: 'blue' }
  if (settings.configured) return { label: '等待账号验证', color: 'warning' }
  return { label: '等待配置', color: 'default' }
})

const statusMeta: Record<string, { label: string; color: string }> = {
  queued: { label: '排队中', color: 'default' },
  direct: { label: '尝试直传', color: 'processing' },
  auditing: { label: '预审中', color: 'gold' },
  copying: { label: '提交秒传', color: 'processing' },
  running: { label: '接收中', color: 'processing' },
  success: { label: '已完成', color: 'success' },
  failed: { label: '失败', color: 'error' },
}

function objectValue(value: unknown): Record<string, any> {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, any> : {}
}

function unwrapSettingsPayload(payload: unknown): Record<string, any> {
  let value: any = unwrapData(payload)
  if (value?.settings && typeof value.settings === 'object') value = value.settings
  return objectValue(value)
}

function normalizeTarget(value: any): DeveloperTarget | null {
  const id = String(value?.id ?? value?.target_id ?? value?.targetId ?? '').trim()
  if (!id) return null
  return {
    id,
    name: String(value?.name ?? value?.target_name ?? value?.targetName ?? '未命名小号').trim() || '未命名小号',
    token_masked: String(value?.token_masked ?? value?.tokenMasked ?? value?.masked_token ?? value?.token ?? '已配置'),
    created_at: Number(value?.created_at ?? value?.createdAt ?? 0) || 0,
    updated_at: Number(value?.updated_at ?? value?.updatedAt ?? value?.created_at ?? value?.createdAt ?? 0) || 0,
  }
}

function targetListFromPayload(value: Record<string, any>): DeveloperTarget[] {
  const candidates = [value.targets, value.data?.targets, value.list, value.items]
  const source = candidates.find((item) => Array.isArray(item)) || []
  return source.map(normalizeTarget).filter(Boolean) as DeveloperTarget[]
}

function normalizeJob(value: any): TransferJob | null {
  const id = String(value?.id ?? value?.job_id ?? value?.jobId ?? '').trim()
  if (!id) return null
  const names = Array.isArray(value?.file_names)
    ? value.file_names
    : Array.isArray(value?.fileNames)
      ? value.fileNames
      : []
  const ids = Array.isArray(value?.file_ids)
    ? value.file_ids
    : Array.isArray(value?.fileIds)
      ? value.fileIds
      : []
  return {
    id,
    target_id: String(value?.target_id ?? value?.targetId ?? '').trim(),
    target_name: String(value?.target_name ?? value?.targetName ?? '未命名小号').trim() || '未命名小号',
    file_ids: ids.map((item: unknown) => String(item)),
    file_names: names.map((item: unknown) => String(item)).filter(Boolean),
    total_count: Number(value?.total_count ?? value?.totalCount ?? 0) || 0,
    passed_count: Number(value?.passed_count ?? value?.passedCount ?? 0) || 0,
    rejected_count: Number(value?.rejected_count ?? value?.rejectedCount ?? 0) || 0,
    pending_count: Number(value?.pending_count ?? value?.pendingCount ?? 0) || 0,
    success_count: Number(value?.success_count ?? value?.successCount ?? 0) || 0,
    skipped_count: Number(value?.skipped_count ?? value?.skippedCount ?? 0) || 0,
    status: String(value?.status ?? 'queued').toLowerCase(),
    phase: String(value?.phase ?? '').toLowerCase(),
    message: value?.message == null ? null : String(value.message),
    error_code: value?.error_code == null && value?.errorCode == null
      ? null
      : Number(value?.error_code ?? value?.errorCode),
    created_at: Number(value?.created_at ?? value?.createdAt ?? 0) || 0,
    updated_at: Number(value?.updated_at ?? value?.updatedAt ?? value?.created_at ?? value?.createdAt ?? 0) || 0,
  }
}

function jobListFromPayload(payload: unknown): TransferJob[] {
  const value: any = unwrapData(payload)
  const source = Array.isArray(value)
    ? value
    : [value?.list, value?.items, value?.tasks, value?.data?.list, value?.data?.items].find(Array.isArray) || []
  return source.map(normalizeJob).filter(Boolean) as TransferJob[]
}

function applySettings(value: unknown) {
  const source = unwrapSettingsPayload(value)
  Object.assign(settings, {
    configured: source.configured === true,
    enabled: source.enabled === true,
    requested_enabled: source.requested_enabled === true || source.requestedEnabled === true,
    client_id: String(source.client_id ?? source.clientId ?? ''),
    client_secret_set: source.client_secret_set === true || source.clientSecretSet === true,
    account_id: String(source.account_id ?? source.accountId ?? ''),
    current_account_id: String(source.current_account_id ?? source.currentAccountId ?? ''),
    account_verified: source.account_verified === true || source.accountVerified === true,
    account_matches_current: source.account_matches_current === true || source.accountMatchesCurrent === true,
    verified_at: Number(source.verified_at ?? source.verifiedAt ?? 0) || 0,
    managed_by_environment: source.managed_by_environment === true || source.managedByEnvironment === true,
    client_id_managed_by_environment: source.client_id_managed_by_environment === true || source.clientIdManagedByEnvironment === true,
    client_secret_managed_by_environment: source.client_secret_managed_by_environment === true || source.clientSecretManagedByEnvironment === true,
    targets: targetListFromPayload(source),
  })
  credentials.client_id = settings.client_id
  credentials.client_secret = ''
}

async function loadSettings() {
  loading.value = true
  settingsError.value = ''
  try {
    applySettings(await bridge.invoke('get_developer_settings'))
  } catch (reason) {
    settingsError.value = errorText(reason)
  } finally {
    loading.value = false
  }
}

async function loadJobs() {
  jobsLoading.value = true
  jobsError.value = ''
  try {
    jobs.value = jobListFromPayload(await bridge.invoke('list_developer_transfers', { limit: 50 }))
  } catch (reason) {
    jobsError.value = errorText(reason)
  } finally {
    jobsLoading.value = false
  }
}

async function saveCredentials() {
  saving.value = true
  try {
    await bridge.invoke('update_developer_credentials', {
      client_id: credentials.client_id.trim(),
      client_secret: credentials.client_secret.trim() || undefined,
    })
    await loadSettings()
    message.success('开发者凭据已保存；凭据变化后需要重新验证当前账号')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    saving.value = false
  }
}

async function testCredentials() {
  testing.value = true
  try {
    const result: any = unwrapData(await bridge.invoke('test_developer_credentials', {
      probe_file_id: probeFileId.value || undefined,
    }))
    applySettings(result?.settings || await bridge.invoke('get_developer_settings'))
    message.success('验证通过：client_id 可读取当前账号的同一个文件')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    testing.value = false
  }
}

async function setDeveloperMode(enabled: boolean) {
  modeSaving.value = true
  try {
    applySettings(await bridge.invoke('update_developer_mode', { enabled }))
    message.success(enabled ? '开发者模式已开启' : '开发者模式已关闭')
  } catch (reason) {
    message.error(errorText(reason))
    await loadSettings()
  } finally {
    modeSaving.value = false
  }
}

function openTarget(target?: DeveloperTarget) {
  Object.assign(targetModal, {
    open: true,
    id: target?.id || '',
    name: target?.name || '',
    token_id: '',
  })
}

async function saveTarget() {
  if (!targetModal.name.trim()) {
    message.warning('请填写小号名称')
    return
  }
  if (!targetModal.id && !targetModal.token_id.trim()) {
    message.warning('首次添加小号必须填写接收 TOKEN')
    return
  }
  const editing = Boolean(targetModal.id)
  targetSaving.value = true
  try {
    await bridge.invoke('upsert_developer_target', {
      id: targetModal.id || undefined,
      name: targetModal.name.trim(),
      token_id: targetModal.token_id.trim() || undefined,
    })
    targetModal.open = false
    await loadSettings()
    activeTab.value = 'tokens'
    message.success(editing ? '小号配置已更新' : '小号 TOKEN 已添加')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    targetSaving.value = false
  }
}

function removeTarget(target: DeveloperTarget) {
  Modal.confirm({
    title: `删除「${target.name}」？`,
    content: '只会删除本机保存的接收 TOKEN，不会删除小号云盘中的文件。',
    okText: '删除',
    okType: 'danger',
    cancelText: '取消',
    async onOk() {
      try {
        await bridge.invoke('delete_developer_target', { id: target.id })
        await loadSettings()
        message.success('小号配置已删除')
      } catch (reason) {
        message.error(errorText(reason))
        throw reason
      }
    },
  })
}

function jobTitle(job: TransferJob) {
  const names = Array.isArray(job.file_names) ? job.file_names.filter(Boolean) : []
  if (names.length === 1) return names[0]
  if (names.length > 1) return `${names[0]} 等 ${job.total_count || names.length} 项`
  return `${job.total_count || 0} 项文件`
}

function jobCountLabel(job: TransferJob) {
  const total = job.total_count || job.file_names.length || job.file_ids.length
  if (job.status === 'success') return `完成 ${job.success_count || total} 项${job.skipped_count ? `，跳过 ${job.skipped_count} 项` : ''}`
  if (job.status === 'failed') return `失败${job.error_code ? `（${job.error_code}）` : ''}`
  if (job.pending_count) return `待处理 ${job.pending_count} / ${total} 项`
  return `${total} 项 · ${statusMeta[job.status]?.label || '处理中'}`
}

function handleTransferEvent(payload: any) {
  if (payload?.type !== 'developer-transfer' || !payload.job?.id) return
  const next = normalizeJob(payload.job)
  if (!next) return
  const index = jobs.value.findIndex((item) => item.id === next.id)
  if (index >= 0) jobs.value.splice(index, 1, next)
  else jobs.value.unshift(next)
  jobs.value.sort((left, right) => right.updated_at - left.updated_at || right.created_at - left.created_at)
  jobs.value = jobs.value.slice(0, 50)
}

onMounted(async () => {
  await Promise.all([loadSettings(), loadJobs()])
  try {
    unsubscribe = await bridge.subscribe(handleTransferEvent)
  } catch (reason) {
    jobsError.value ||= errorText(reason)
  }
})
onBeforeUnmount(() => unsubscribe?.())
</script>

<template>
  <section class="developer-panel" :aria-busy="loading || jobsLoading">
    <div class="panel-lead">
      <div>
        <strong>多号秒传</strong>
        <span>使用当前账号的开发者凭据，把已拥有的文件直接发送到已授权的小号，无需下载再上传。</span>
      </div>
      <div class="mode-control">
        <a-tag :color="modeStatus.color">{{ modeStatus.label }}</a-tag>
        <a-switch
          :checked="settings.requested_enabled"
          :loading="modeSaving"
          :disabled="!settings.requested_enabled && (!settings.account_verified || !settings.account_matches_current)"
          aria-label="开发者模式"
          @change="setDeveloperMode"
        />
      </div>
    </div>

    <a-tabs v-model:active-key="activeTab" class="developer-tabs">
      <a-tab-pane key="tokens">
        <template #tab><span class="inner-tab"><KeyOutlined />Token 配置 <em>{{ settings.targets.length }}</em></span></template>

        <a-alert v-if="settingsError" type="warning" show-icon class="load-alert" :message="`Token 配置读取失败：${settingsError}`">
          <template #action><a-button size="small" @click="loadSettings">重试</a-button></template>
        </a-alert>

        <a-alert class="boundary-alert" :type="settings.account_verified && settings.current_account_id && !settings.account_matches_current ? 'error' : 'info'" show-icon>
          <template #message>client_id 必须属于当前登录账号</template>
          <template #description>
            应用会先用当前登录态读取一个文件，再用开发者凭据读取同一个 <code>fileId</code>；只有两次读取都成功才允许开启。
            登录账号变化后模式会立即失效，避免文件列表、详情和转存混入其他账号。
            <a href="https://wcn6ijfe07e0.feishu.cn/wiki/R6Z2weFwKiwnuBktcoacoDAHnZg" target="_blank" rel="noopener noreferrer">查看官方 TOKEN 上传文档</a>
          </template>
        </a-alert>

        <div v-if="settings.account_verified" class="binding-line">
          <span>已验证账号</span>
          <code>{{ settings.account_id }}</code>
          <span>· {{ formatTime(settings.verified_at) }}</span>
        </div>

        <a-form class="credentials-form" layout="vertical" @submit.prevent="saveCredentials">
          <div class="credentials-grid">
            <a-form-item label="开发者 client_id" required>
              <a-input
                v-model:value="credentials.client_id"
                autocomplete="off"
                :disabled="settings.client_id_managed_by_environment"
                placeholder="填写开发者后台生成的 client_id"
              />
            </a-form-item>
            <a-form-item label="开发者 client_secret" required>
              <a-input-password
                v-model:value="credentials.client_secret"
                autocomplete="new-password"
                :disabled="settings.client_secret_managed_by_environment"
                :placeholder="settings.client_secret_set ? '已保存；留空表示不修改' : '填写 client_secret'"
              />
            </a-form-item>
          </div>
          <div v-if="settings.managed_by_environment" class="field-help">
            带锁字段由 GUANGYA_DEVELOPER_CLIENT_ID / GUANGYA_DEVELOPER_CLIENT_SECRET 环境变量托管。
          </div>
          <a-space wrap>
            <a-button type="primary" html-type="submit" :loading="saving">保存开发者凭据</a-button>
            <a-button :disabled="!canTest" :loading="testing" @click="testCredentials">
              <template #icon><CheckCircleOutlined /></template>
              验证当前账号
            </a-button>
            <a-button :loading="loading" @click="loadSettings"><template #icon><ReloadOutlined /></template>刷新配置</a-button>
          </a-space>
        </a-form>

        <div class="subsection-head">
          <div>
            <strong>接收 TOKEN</strong>
            <span>小号创建 TOKEN 并授权目标目录后，在这里保存多个接收方向；一个 TOKEN 只对应当前账号到一个小号。</span>
          </div>
          <a-space>
            <a-button size="small" :loading="loading" @click="loadSettings"><template #icon><ReloadOutlined /></template>刷新</a-button>
            <a-button type="primary" ghost @click="openTarget()"><template #icon><PlusOutlined /></template>添加小号</a-button>
          </a-space>
        </div>

        <a-spin :spinning="loading">
          <div v-if="settings.targets.length" class="target-grid" aria-live="polite">
            <article v-for="item in settings.targets" :key="item.id" class="target-card">
              <div class="target-card-head">
                <span class="token-icon"><KeyOutlined /></span>
                <div class="target-identity">
                  <strong :title="item.name">{{ item.name }}</strong>
                  <code>{{ item.token_masked }}</code>
                </div>
                <a-space size="small">
                  <a-button type="text" size="small" :aria-label="`编辑 ${item.name}`" @click="openTarget(item)"><EditOutlined /></a-button>
                  <a-button type="text" size="small" danger :aria-label="`删除 ${item.name}`" @click="removeTarget(item)"><DeleteOutlined /></a-button>
                </a-space>
              </div>
              <div class="target-card-foot">更新于 {{ formatTime(item.updated_at) }}</div>
            </article>
          </div>
          <a-empty v-else description="还没有小号 TOKEN" />
        </a-spin>
      </a-tab-pane>

      <a-tab-pane key="jobs">
        <template #tab><span class="inner-tab"><ReloadOutlined />任务记录 <em>{{ jobs.length }}</em></span></template>

        <a-alert v-if="jobsError" type="warning" show-icon class="load-alert" :message="`任务记录读取失败：${jobsError}`">
          <template #action><a-button size="small" @click="loadJobs">重试</a-button></template>
        </a-alert>

        <div class="subsection-head jobs-head">
          <div>
            <strong>最近任务</strong>
            <span>预审任务会在后台续跑，应用重启后也会恢复跟进。</span>
          </div>
          <a-button size="small" :loading="jobsLoading" @click="loadJobs"><template #icon><ReloadOutlined /></template>刷新记录</a-button>
        </div>

        <a-spin :spinning="jobsLoading">
          <div v-if="jobs.length" class="job-list" aria-live="polite">
            <article v-for="item in jobs" :key="item.id" class="job-card">
              <div class="job-card-head">
                <div class="job-title-wrap">
                  <strong class="job-title" :title="jobTitle(item)">{{ jobTitle(item) }}</strong>
                  <a-tag :color="(statusMeta[item.status] || statusMeta.queued).color">{{ (statusMeta[item.status] || statusMeta.queued).label }}</a-tag>
                </div>
                <span class="job-time">{{ formatTime(item.created_at) }}</span>
              </div>
              <div class="job-meta">
                <span>发送到 {{ item.target_name }}</span>
                <span>{{ jobCountLabel(item) }}</span>
              </div>
              <div v-if="item.message" class="job-message" :class="{ error: item.status === 'failed' }">{{ item.message }}</div>
            </article>
          </div>
          <a-empty v-else description="暂无互传任务" />
        </a-spin>
      </a-tab-pane>
    </a-tabs>

    <a-modal
      v-model:open="targetModal.open"
      :title="targetModal.id ? '编辑小号 TOKEN' : '添加小号 TOKEN'"
      ok-text="保存"
      cancel-text="取消"
      :confirm-loading="targetSaving"
      @ok="saveTarget"
    >
      <a-form layout="vertical" @submit.prevent="saveTarget">
        <a-form-item label="小号名称" required>
          <a-input v-model:value="targetModal.name" :maxlength="64" placeholder="例如：小号 A / 家庭盘" />
        </a-form-item>
        <a-form-item label="接收 TOKEN" :required="!targetModal.id">
          <a-input-password
            v-model:value="targetModal.token_id"
            autocomplete="new-password"
            :placeholder="targetModal.id ? '已保存；留空表示不修改' : '粘贴小号生成的接收 TOKEN'"
          />
          <div class="field-help">TOKEN 仅写入本机状态库，界面和接口不会回显完整值。</div>
        </a-form-item>
      </a-form>
    </a-modal>
  </section>
</template>

<style scoped>
.developer-panel { max-width: 980px; padding: 8px 18px 36px 24px; }
.panel-lead { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; margin-bottom: 18px; }
.panel-lead strong, .panel-lead span { display: block; }
.panel-lead strong { font-size: 18px; }
.panel-lead span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.55; }
.mode-control { display: flex; align-items: center; gap: 10px; }
.inner-tab { display: inline-flex; align-items: center; gap: 7px; }
.inner-tab em { min-width: 18px; padding: 0 5px; border-radius: 10px; color: var(--text-2, #475467); background: var(--bg-toolbar, #f2f4f7); font-size: 11px; font-style: normal; line-height: 18px; text-align: center; }
.load-alert { margin-bottom: 16px; }
.boundary-alert { margin-bottom: 20px; }
.boundary-alert a { margin-left: 6px; }
.binding-line { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; margin: -5px 0 18px; color: var(--text-3, #98a2b3); font-size: 12px; }
.credentials-form { padding-bottom: 24px; border-bottom: 1px solid var(--line, #e5e7eb); }
.credentials-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.field-help { margin: -8px 0 12px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.5; }
.subsection-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; margin: 24px 0 12px; }
.subsection-head strong, .subsection-head span { display: block; }
.subsection-head strong { font-size: 15px; }
.subsection-head span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.5; }
.target-grid, .job-list { display: grid; gap: 10px; }
.target-card, .job-card { padding: 14px 16px; border: 1px solid var(--line, #e5e7eb); border-radius: 10px; background: var(--surface, #fff); }
.target-card-head, .job-card-head { display: flex; min-width: 0; align-items: center; gap: 10px; }
.token-icon { display: grid; width: 34px; height: 34px; flex: 0 0 34px; place-items: center; border-radius: 9px; color: var(--primary, #1677ff); background: color-mix(in srgb, var(--primary, #1677ff) 10%, transparent); }
.target-identity { min-width: 0; flex: 1; }
.target-identity strong, .target-identity code { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.target-identity code { margin-top: 3px; color: var(--text-2, #475467); font-size: 12px; }
.target-card-foot { margin-top: 10px; padding-left: 44px; color: var(--text-3, #98a2b3); font-size: 12px; }
.jobs-head { margin-top: 4px; }
.job-title-wrap { display: flex; min-width: 0; flex: 1; align-items: center; gap: 8px; }
.job-title { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.job-time { flex: 0 0 auto; color: var(--text-3, #98a2b3); font-size: 12px; }
.job-meta { display: flex; flex-wrap: wrap; gap: 5px 18px; margin-top: 9px; color: var(--text-2, #475467); font-size: 12px; }
.job-message { margin-top: 7px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.5; }
.job-message.error { color: var(--danger, #ff4d4f); }
code { color: var(--text-2, #475467); font-size: 12px; }
@media (max-width: 760px) {
  .developer-panel { padding-inline: 14px; }
  .panel-lead, .subsection-head { align-items: stretch; flex-direction: column; gap: 10px; }
  .credentials-grid { grid-template-columns: 1fr; gap: 0; }
  .target-card-head, .job-card-head { align-items: flex-start; }
  .job-card-head { flex-direction: column; }
  .job-time { padding-left: 0; }
}
</style>
