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
  target_name: string
  file_names: string[]
  total_count: number
  success_count: number
  skipped_count: number
  status: string
  message?: string | null
  created_at: number
}

const loading = shallowRef(false)
const saving = shallowRef(false)
const testing = shallowRef(false)
const modeSaving = shallowRef(false)
const targetSaving = shallowRef(false)
const jobsLoading = shallowRef(false)
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

const canTest = computed(() => settings.configured && !saving.value)
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

function applySettings(value: any) {
  Object.assign(settings, {
    configured: value?.configured === true,
    enabled: value?.enabled === true,
    requested_enabled: value?.requested_enabled === true,
    client_id: String(value?.client_id || ''),
    client_secret_set: value?.client_secret_set === true,
    account_id: String(value?.account_id || ''),
    current_account_id: String(value?.current_account_id || ''),
    account_verified: value?.account_verified === true,
    account_matches_current: value?.account_matches_current === true,
    verified_at: Number(value?.verified_at || 0),
    managed_by_environment: value?.managed_by_environment === true,
    client_id_managed_by_environment: value?.client_id_managed_by_environment === true,
    client_secret_managed_by_environment: value?.client_secret_managed_by_environment === true,
    targets: Array.isArray(value?.targets) ? value.targets : [],
  })
  credentials.client_id = settings.client_id
  credentials.client_secret = ''
}

async function loadSettings() {
  loading.value = true
  try {
    applySettings(unwrapData(await bridge.invoke('get_developer_settings')))
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    loading.value = false
  }
}

async function loadJobs() {
  jobsLoading.value = true
  try {
    const data = unwrapData(await bridge.invoke('list_developer_transfers', { limit: 20 }))
    jobs.value = Array.isArray(data.list) ? data.list : []
  } catch (reason) {
    message.error(errorText(reason))
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
    const result = unwrapData(await bridge.invoke('test_developer_credentials', {
      probe_file_id: probeFileId.value || undefined,
    }))
    applySettings(result.settings || await bridge.invoke('get_developer_settings'))
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
    applySettings(unwrapData(await bridge.invoke('update_developer_mode', { enabled })))
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
  targetSaving.value = true
  try {
    await bridge.invoke('upsert_developer_target', {
      id: targetModal.id || undefined,
      name: targetModal.name.trim(),
      token_id: targetModal.token_id.trim() || undefined,
    })
    targetModal.open = false
    await loadSettings()
    message.success(targetModal.id ? '小号配置已更新' : '小号 TOKEN 已添加')
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

function handleTransferEvent(payload: any) {
  if (payload?.type !== 'developer-transfer' || !payload.job?.id) return
  const index = jobs.value.findIndex((item) => item.id === payload.job.id)
  if (index >= 0) jobs.value.splice(index, 1, payload.job)
  else jobs.value.unshift(payload.job)
  jobs.value = jobs.value.slice(0, 20)
}

onMounted(async () => {
  await Promise.all([loadSettings(), loadJobs()])
  unsubscribe = await bridge.subscribe(handleTransferEvent)
})
onBeforeUnmount(() => unsubscribe?.())
</script>

<template>
  <section class="setting-section" :aria-busy="loading">
    <div class="section-lead">
      <div>
        <strong>开发者模式</strong>
        <span>主文件接口读取失败时使用开发者接口兜底，同时为当前账号提供小号 TOKEN 秒传。</span>
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
      </a-space>
    </a-form>

    <div class="subsection-head">
      <div>
        <strong>小号接收 TOKEN</strong>
        <span>TOKEN 由小号创建并授权目标目录；一个 TOKEN 只对应“当前账号 → 小号”这一发送方向。</span>
      </div>
      <a-button type="primary" ghost @click="openTarget()">
        <template #icon><PlusOutlined /></template>
        添加小号
      </a-button>
    </div>

    <a-list class="target-list" :data-source="settings.targets" :locale="{ emptyText: '还没有小号 TOKEN' }">
      <template #renderItem="{ item }">
        <a-list-item>
          <template #actions>
            <a-button type="text" size="small" :aria-label="`编辑 ${item.name}`" @click="openTarget(item)"><EditOutlined /></a-button>
            <a-button type="text" size="small" danger :aria-label="`删除 ${item.name}`" @click="removeTarget(item)"><DeleteOutlined /></a-button>
          </template>
          <a-list-item-meta>
            <template #avatar><span class="token-icon"><KeyOutlined /></span></template>
            <template #title>{{ item.name }}</template>
            <template #description><code>{{ item.token_masked }}</code> · 更新于 {{ formatTime(item.updated_at) }}</template>
          </a-list-item-meta>
        </a-list-item>
      </template>
    </a-list>

    <div class="subsection-head jobs-head">
      <div>
        <strong>最近互传任务</strong>
        <span>预审任务会在后台续跑，应用重启后也会恢复跟进。</span>
      </div>
      <a-button type="text" :loading="jobsLoading" @click="loadJobs"><template #icon><ReloadOutlined /></template>刷新</a-button>
    </div>

    <a-list class="jobs-list" :loading="jobsLoading" :data-source="jobs" :locale="{ emptyText: '暂无互传任务' }">
      <template #renderItem="{ item }">
        <a-list-item>
          <a-list-item-meta>
            <template #title>
              <span class="job-title">{{ jobTitle(item) }}</span>
              <a-tag :color="(statusMeta[item.status] || statusMeta.queued).color">{{ (statusMeta[item.status] || statusMeta.queued).label }}</a-tag>
            </template>
            <template #description>
              <span>发送到 {{ item.target_name }} · {{ formatTime(item.created_at) }}</span>
              <span v-if="item.message">{{ item.message }}</span>
              <span v-if="item.status === 'success'">成功 {{ item.success_count || item.total_count }} 项<span v-if="item.skipped_count">，跳过 {{ item.skipped_count }} 项</span></span>
            </template>
          </a-list-item-meta>
        </a-list-item>
      </template>
    </a-list>

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
.setting-section { max-width: 820px; }
.section-lead, .subsection-head { display: flex; align-items: flex-start; justify-content: space-between; gap: 20px; }
.section-lead { margin-bottom: 16px; }
.section-lead strong, .section-lead span, .subsection-head strong, .subsection-head span { display: block; }
.section-lead strong { font-size: 16px; }
.section-lead span, .subsection-head span { margin-top: 5px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.55; }
.mode-control { display: flex; align-items: center; gap: 10px; }
.boundary-alert { margin-bottom: 24px; }
.boundary-alert a { margin-left: 6px; }
.binding-line { display: flex; align-items: center; flex-wrap: wrap; gap: 6px; margin: -10px 0 18px; color: var(--text-3, #98a2b3); font-size: 12px; }
.credentials-form { padding-bottom: 28px; border-bottom: 1px solid var(--line, #e5e7eb); }
.credentials-grid { display: grid; grid-template-columns: 1fr 1fr; gap: 16px; }
.field-help { margin: -8px 0 12px; color: var(--text-3, #98a2b3); font-size: 12px; line-height: 1.5; }
.subsection-head { margin: 26px 0 10px; }
.subsection-head strong { font-size: 15px; }
.target-list, .jobs-list { overflow: hidden; border: 1px solid var(--line, #e5e7eb); border-radius: 10px; background: var(--surface, #fff); }
.target-list :deep(.ant-list-item), .jobs-list :deep(.ant-list-item) { padding-inline: 14px; }
.token-icon { display: grid; width: 34px; height: 34px; place-items: center; border-radius: 9px; color: var(--primary, #1677ff); background: color-mix(in srgb, var(--primary, #1677ff) 10%, transparent); }
code { color: var(--text-2, #475467); font-size: 12px; }
.jobs-head { margin-top: 30px; }
.job-title { margin-right: 8px; }
.jobs-list :deep(.ant-list-item-meta-description) { display: grid; gap: 3px; }
@media (max-width: 760px) {
  .credentials-grid { grid-template-columns: 1fr; gap: 0; }
  .section-lead, .subsection-head { align-items: stretch; flex-direction: column; gap: 10px; }
  .mode-control { justify-content: space-between; }
  .subsection-head .ant-btn { align-self: flex-start; }
}
</style>
