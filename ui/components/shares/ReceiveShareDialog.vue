<script setup lang="ts">
import { computed, reactive } from 'vue'
import { ArrowLeftOutlined, CloudDownloadOutlined, FileOutlined, FolderOpenOutlined, FolderOutlined, InboxOutlined } from '@antdv-next/icons'
import { message } from 'antdv-next'
import { bridge, isTauri } from '../../bridge.js'
import { errorText, fileId, formatSize, formatTime, isFolder, unwrapData } from '../../formatters.js'
import { parseGuangyaShareLink } from '../../shareLink.js'
import { useTransfersStore } from '../../stores/transfers'
import CloudFolderPicker from '../cloud/CloudFolderPicker.vue'

const transfers = useTransfersStore()
const state = reactive({
  open: false,
  loading: false,
  restoring: false,
  downloading: false,
  url: '',
  password: '',
  info: null as Record<string, any> | null,
  files: [] as any[],
  selectedKeys: [] as string[],
  path: [] as Array<{ id: string, name: string }>,
  targetId: '',
  targetLabel: '全部文件',
  targetPickerOpen: false,
  error: '',
})

const columns = [
  { title: '名称', key: 'name', ellipsis: true },
  { title: '大小', key: 'size', width: 110 },
  { title: '修改时间', key: 'time', width: 170 },
]
const currentFolderId = computed(() => state.path.at(-1)?.id || '')
const selectedCount = computed(() => state.selectedKeys.length)
const folderCount = computed(() => state.files.filter(isFolder).length)
const fileCount = computed(() => state.files.length - folderCount.value)
const totalSize = computed(() => state.files.reduce((total, item) => total + Number(item.fileSize || 0), 0))
const breadcrumb = computed(() => [
  { key: 'root', label: '分享根目录' },
  ...state.path.map(folder => ({ key: folder.id, label: folder.name })),
])
const rowSelection = computed(() => ({
  selectedRowKeys: state.selectedKeys,
  onChange: (keys: string[]) => { state.selectedKeys = keys },
}))

function open() {
  Object.assign(state, {
    open: true,
    loading: false,
    url: '',
    password: '',
    info: null,
    files: [],
    selectedKeys: [],
    path: [],
    targetId: '',
    targetLabel: '全部文件',
    targetPickerOpen: false,
    error: '',
  })
}

function requestUrl() {
  const parsed = parseGuangyaShareLink(state.url.trim())
  const code = state.password.trim()
  if (!code || parsed.code) return parsed.url
  const url = new URL(parsed.url)
  url.searchParams.set('code', code)
  return url.toString()
}

async function loadFiles() {
  if (!state.info) return
  state.loading = true
  state.error = ''
  try {
    const data = unwrapData(await bridge.invoke('list_received_share_files', {
      access_token: state.info.access_token || '',
      parent_id: currentFolderId.value,
    }))
    state.files = Array.isArray(data.list) ? data.list : []
    state.selectedKeys = []
  }
  catch (reason) {
    state.error = errorText(reason)
  }
  finally {
    state.loading = false
  }
}

async function openLink() {
  if (!state.url.trim()) {
    message.warning('请输入分享链接')
    return
  }
  state.loading = true
  state.error = ''
  try {
    state.info = unwrapData(await bridge.invoke('open_received_share', { url: requestUrl() }))
    state.path = []
    await loadFiles()
  }
  catch (reason) {
    state.error = errorText(reason)
    state.info = null
  }
  finally {
    state.loading = false
  }
}

function selectedRecords() {
  const ids = new Set(state.selectedKeys)
  return state.files.filter(item => ids.has(fileId(item)))
}

function enterFolder(record: any) {
  if (!isFolder(record)) return
  state.path = [...state.path, { id: fileId(record), name: String(record.fileName || '文件夹') }]
  void loadFiles()
}

function rowProps(record: any) {
  return {
    tabindex: 0,
    onDblclick: () => enterFolder(record),
    onKeydown: (event: KeyboardEvent) => {
      if (event.key === 'Enter') enterFolder(record)
    },
  }
}

function jump(index: number) {
  state.path = index < 0 ? [] : state.path.slice(0, index + 1)
  void loadFiles()
}

async function restore() {
  if (!state.info || !selectedCount.value) return
  state.restoring = true
  try {
    await bridge.invoke('restore_received_share', {
      access_token: state.info.access_token || '',
      file_ids: selectedRecords().map(fileId),
      parent_id: state.targetId,
    })
    state.open = false
    message.success(`已将 ${selectedCount.value} 项转存到 ${state.targetLabel}`)
  }
  catch (reason) {
    message.error(errorText(reason))
  }
  finally {
    state.restoring = false
  }
}

function selectTarget(target: { id: string, label: string }) {
  state.targetId = target.id
  state.targetLabel = target.label
}

async function download() {
  if (!state.info || !selectedCount.value) return
  state.downloading = true
  try {
    const queued = await transfers.downloadReceivedShare(selectedRecords(), String(state.info.access_token || ''))
    if (isTauri && queued) message.success('已加入下载队列')
  }
  catch (reason) {
    message.error(errorText(reason))
  }
  finally {
    state.downloading = false
  }
}
</script>

<template>
  <a-button type="primary" @click="open"><template #icon><InboxOutlined /></template>接收分享</a-button>
  <a-modal v-model:open="state.open" title="接收分享" :footer="null" width="720px">
    <a-space direction="vertical" style="width:100%" :size="12">
      <a-flex gap="small" wrap="wrap">
        <a-input v-model:value="state.url" style="flex:1;min-width:240px" placeholder="https://www.guangyapan.com/s/…" @press-enter="openLink" />
        <a-input v-model:value="state.password" style="width:140px" placeholder="提取码" @press-enter="openLink" />
        <a-button type="primary" :loading="state.loading" @click="openLink">打开</a-button>
      </a-flex>
      <a-alert v-if="state.error" type="error" show-icon :message="state.error" />
      <template v-if="state.info">
        <div class="received-meta">
          <strong>{{ state.info.share_name || '分享内容' }}</strong>
          <span>{{ fileCount }} 个文件 · {{ folderCount }} 个文件夹 · {{ formatSize(totalSize) }}</span>
        </div>
        <div class="folder-toolbar">
          <a-button size="small" :disabled="!state.path.length" aria-label="返回上级分享目录" @click="jump(state.path.length - 2)"><template #icon><ArrowLeftOutlined /></template></a-button>
          <a-breadcrumb>
            <a-breadcrumb-item v-for="(item, index) in breadcrumb" :key="item.key">
              <button type="button" class="breadcrumb-button" @click="jump(index - 1)">{{ item.label }}</button>
            </a-breadcrumb-item>
          </a-breadcrumb>
        </div>
        <a-table :columns="columns" :data-source="state.files" :loading="state.loading" :row-key="fileId" :row-selection="rowSelection" :on-row="rowProps" :pagination="false" size="small" :scroll="{ y: 300 }">
          <template #emptyText><a-empty description="此目录为空" /></template>
          <template #bodyCell="{ column, record }">
            <template v-if="column.key === 'name'">
              <a-flex align="center" gap="small">
                <div class="file-icon" :class="isFolder(record) ? 'folder' : 'other'"><component :is="isFolder(record) ? FolderOutlined : FileOutlined" /></div>
                <button v-if="isFolder(record)" type="button" class="file-name-button clickable" @click="enterFolder(record)">{{ record.fileName }}</button>
                <span v-else class="file-name">{{ record.fileName }}</span>
              </a-flex>
            </template>
            <template v-else-if="column.key === 'size'">{{ isFolder(record) ? '—' : formatSize(record.fileSize) }}</template>
            <template v-else-if="column.key === 'time'">{{ formatTime(record.lastUpdateTime) }}</template>
          </template>
        </a-table>
        <a-flex justify="space-between" align="center" wrap="wrap" gap="small">
          <a-flex align="center" gap="small">
            <span class="modal-hint">已选 {{ selectedCount }} 项 · 转存到 {{ state.targetLabel }}</span>
            <a-button size="small" @click="state.targetPickerOpen = true"><template #icon><FolderOpenOutlined /></template>选择目录</a-button>
          </a-flex>
          <a-space>
            <a-button :loading="state.downloading" :disabled="!selectedCount" @click="download"><template #icon><CloudDownloadOutlined /></template>下载所选</a-button>
            <a-button type="primary" :loading="state.restoring" :disabled="!selectedCount" @click="restore"><template #icon><InboxOutlined /></template>转存所选</a-button>
          </a-space>
        </a-flex>
      </template>
    </a-space>
  </a-modal>
  <CloudFolderPicker v-model:open="state.targetPickerOpen" title="选择转存目录" @select="selectTarget" />
</template>

<style scoped>
.received-meta { display: flex; align-items: center; justify-content: space-between; gap: 12px; }
.received-meta span, .modal-hint { color: var(--text-3, #98a2b3); font-size: 12px; }
.folder-toolbar { display: flex; align-items: center; gap: 10px; }
.breadcrumb-button { padding: 0; border: 0; color: inherit; background: transparent; cursor: pointer; }
</style>
