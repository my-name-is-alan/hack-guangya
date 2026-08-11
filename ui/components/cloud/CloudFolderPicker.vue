<script setup lang="ts">
import { reactive, watch } from 'vue'
import { ArrowLeftOutlined, FolderOutlined } from '@antdv-next/icons'
import { message } from 'antdv-next'
import { bridge } from '../../bridge.js'
import { errorText, fileId, isFolder, unwrapData } from '../../formatters.js'

const props = withDefaults(defineProps<{
  open: boolean
  title?: string
}>(), {
  title: '选择云端目录',
})

const emit = defineEmits<{
  'update:open': [value: boolean]
  select: [value: { id: string, label: string, path: string }]
}>()

const picker = reactive({
  loading: false,
  items: [] as any[],
  path: [{ id: '', name: '全部文件' }],
  page: 0,
  total: 0,
})
let latestFoldersRequest = 0

const columns = [
  { title: '文件夹', key: 'name', ellipsis: true },
  { title: '操作', key: 'actions', width: 84 },
]

async function loadFolders(page = 0) {
  const requestId = ++latestFoldersRequest
  const parentId = picker.path.at(-1)?.id || ''
  picker.loading = true
  try {
    const data = unwrapData(await bridge.invoke('list_files', {
      parent_id: parentId,
      page,
      folders_only: true,
    }))
    if (requestId !== latestFoldersRequest || parentId !== (picker.path.at(-1)?.id || '')) return
    picker.items = (Array.isArray(data.list) ? data.list : []).filter(isFolder)
    picker.page = page
    picker.total = Math.max(Number(data.total || 0), picker.items.length)
  }
  catch (reason) {
    message.error(errorText(reason))
  }
  finally {
    if (requestId === latestFoldersRequest) picker.loading = false
  }
}

async function enterFolder(record: any) {
  picker.path.push({
    id: String(fileId(record)),
    name: String(record.fileName || record.name || '未命名文件夹'),
  })
  await loadFolders(0)
}

async function leaveFolder() {
  if (picker.path.length <= 1) return
  picker.path.pop()
  await loadFolders(0)
}

async function jumpTo(index: number) {
  if (index < 0 || index >= picker.path.length - 1) return
  picker.path = picker.path.slice(0, index + 1)
  await loadFolders(0)
}

function selectCurrent() {
  const current = picker.path.at(-1)
  const names = picker.path.slice(1).map(item => item.name)
  emit('select', {
    id: current?.id || '',
    label: names.length ? `全部文件 / ${names.join(' / ')}` : '全部文件',
    path: names.length ? `/${names.join('/')}` : '',
  })
  emit('update:open', false)
}

function handleTableChange(pagination: any) {
  const page = Math.max(0, Number(pagination?.current || 1) - 1)
  if (page !== picker.page) void loadFolders(page)
}

watch(() => props.open, (open) => {
  if (!open) {
    latestFoldersRequest += 1
    picker.loading = false
    return
  }
  picker.path = [{ id: '', name: '全部文件' }]
  void loadFolders(0)
})
</script>

<template>
  <a-modal
    :open="open"
    :title="title"
    width="620px"
    ok-text="选择当前目录"
    cancel-text="取消"
    @ok="selectCurrent"
    @cancel="emit('update:open', false)"
  >
    <a-flex class="cloud-folder-toolbar" align="center" gap="small">
      <a-button type="text" :disabled="picker.path.length <= 1" aria-label="返回上级目录" @click="leaveFolder">
        <template #icon><ArrowLeftOutlined /></template>
      </a-button>
      <a-breadcrumb>
        <a-breadcrumb-item v-for="(segment, index) in picker.path" :key="segment.id || 'root'">
          <a v-if="index < picker.path.length - 1" href="#" @click.prevent="jumpTo(index)">{{ segment.name }}</a>
          <span v-else>{{ segment.name }}</span>
        </a-breadcrumb-item>
      </a-breadcrumb>
    </a-flex>
    <a-table
      :columns="columns"
      :data-source="picker.items"
      :loading="picker.loading"
      :row-key="fileId"
      :pagination="{ current: picker.page + 1, pageSize: 100, total: picker.total, showSizeChanger: false }"
      size="small"
      @change="handleTableChange"
    >
      <template #emptyText><a-empty description="当前目录下没有文件夹" /></template>
      <template #bodyCell="{ column, record }">
        <template v-if="column.key === 'name'">
          <a-space><FolderOutlined /><span>{{ record.fileName || record.name }}</span></a-space>
        </template>
        <template v-else-if="column.key === 'actions'">
          <a-button type="link" size="small" @click="enterFolder(record)">进入</a-button>
        </template>
      </template>
    </a-table>
  </a-modal>
</template>

<style scoped>
.cloud-folder-toolbar { min-height: 40px; margin-bottom: 10px; }
</style>
