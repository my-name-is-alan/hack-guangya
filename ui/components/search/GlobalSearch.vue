<script setup lang="ts">
import { computed, nextTick, reactive, shallowRef, useTemplateRef, watch } from 'vue'
import { useRouter } from 'vue-router'
import {
  AudioOutlined,
  CloseOutlined,
  FileImageOutlined,
  FileOutlined,
  FileTextOutlined,
  FileZipOutlined,
  FolderOutlined,
  LoadingOutlined,
  SearchOutlined,
  VideoCameraOutlined,
} from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText, formatSize, formatTime, isFolder, pick, unwrapData } from '../../formatters.js'

const open = defineModel<boolean>('open', { required: true })
const router = useRouter()
const searchInput = useTemplateRef<any>('searchInput')
const searchDialog = useTemplateRef<HTMLElement>('searchDialog')
const query = shallowRef('')
const fileType = shallowRef('all')
const extension = shallowRef('')
const loading = shallowRef(false)
const loadingMore = shallowRef(false)
const error = shallowRef('')
const results = shallowRef<any[]>([])
const searched = shallowRef(false)
const currentPage = shallowRef(0)
const remoteTotal = shallowRef(0)
const remoteLoaded = shallowRef(0)
let requestSequence = 0
let previouslyFocused: HTMLElement | null = null

const filters = [
  { key: 'all', label: '全部' },
  { key: 'folder', label: '文件夹' },
  { key: 'image', label: '图片' },
  { key: 'video', label: '视频' },
  { key: 'audio', label: '音频' },
  { key: 'document', label: '文档' },
  { key: 'archive', label: '压缩包' },
]

const hasCondition = computed(() => Boolean(
  query.value.trim()
  || fileType.value !== 'all'
  || extension.value.trim(),
))
const hasMore = computed(() => searched.value && remoteLoaded.value < remoteTotal.value)

function resultIcon(record: any) {
  if (isFolder(record)) return FolderOutlined
  const suffix = String(pick(record, ['fileSuffix', 'extension'], '')).toLowerCase()
  if (/^(jpg|jpeg|png|gif|webp|bmp|svg|heic|avif)$/.test(suffix)) return FileImageOutlined
  if (/^(mp4|mov|mkv|avi|webm|m4v)$/.test(suffix)) return VideoCameraOutlined
  if (/^(mp3|wav|flac|aac|m4a|ogg|opus)$/.test(suffix)) return AudioOutlined
  if (/^(zip|rar|7z|tar|gz)$/.test(suffix)) return FileZipOutlined
  if (/^(pdf|doc|docx|xls|xlsx|ppt|pptx|txt|md)$/.test(suffix)) return FileTextOutlined
  return FileOutlined
}

async function searchPage(page: number, append: boolean) {
  const sequence = ++requestSequence
  if (!hasCondition.value) {
    loading.value = false
    loadingMore.value = false
    results.value = []
    searched.value = false
    error.value = ''
    currentPage.value = 0
    remoteTotal.value = 0
    remoteLoaded.value = 0
    return
  }
  if (append) loadingMore.value = true
  else loading.value = true
  error.value = ''
  try {
    const data = unwrapData(await bridge.invoke('search_files', {
      query: query.value.trim(),
      file_type: fileType.value === 'all' ? '' : fileType.value,
      extension: extension.value.trim().replace(/^\./, '').toLowerCase(),
      page,
    }))
    if (sequence !== requestSequence) return
    const list = data.list || data.items || data.results || []
    const next = Array.isArray(list) ? list : []
    if (append) {
      const records = new Map(results.value.map(record => [String(pick(record, ['fileId', 'id'], record.fileName)), record]))
      next.forEach(record => records.set(String(pick(record, ['fileId', 'id'], record.fileName)), record))
      results.value = [...records.values()]
    }
    else {
      results.value = next
    }
    currentPage.value = page
    remoteTotal.value = Number(data.remote_total ?? data.remoteTotal ?? data.total ?? next.length)
    const pageRemoteCount = Number(data.remote_count ?? data.remoteCount ?? data.page_size ?? data.pageSize ?? next.length)
    remoteLoaded.value = append ? remoteLoaded.value + pageRemoteCount : pageRemoteCount
    searched.value = true
  }
  catch (reason) {
    if (sequence !== requestSequence) return
    if (!append) results.value = []
    searched.value = true
    error.value = errorText(reason)
  }
  finally {
    if (sequence === requestSequence) {
      loading.value = false
      loadingMore.value = false
    }
  }
}

function search() {
  return searchPage(0, false)
}

function loadMore() {
  if (!hasMore.value || loadingMore.value) return
  return searchPage(currentPage.value + 1, true)
}

function close() {
  open.value = false
}

function handleDialogKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault()
    close()
    return
  }
  if (event.key !== 'Tab') return
  const focusable = [...(searchDialog.value?.querySelectorAll<HTMLElement>(
    'button:not([disabled]), input:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
  ) || [])].filter(element => element.offsetParent !== null)
  if (!focusable.length) return
  const first = focusable[0]
  const last = focusable.at(-1)!
  if (event.shiftKey && document.activeElement === first) {
    event.preventDefault()
    last.focus()
  }
  else if (!event.shiftKey && document.activeElement === last) {
    event.preventDefault()
    first.focus()
  }
}

function showInFiles(record: any) {
  const parentId = String(pick(record, ['parentId', 'parent_id'], ''))
  const parentName = String(pick(record, ['parentName', 'parent_name', 'parentPath'], ''))
  const focus = String(pick(record, ['fileId', 'id'], ''))
  const routeQuery = {
    ...(parentId ? { parent: parentId } : {}),
    ...(parentName ? { parentName } : {}),
    ...(focus ? { focus } : {}),
  }
  void router.push({ name: 'files', query: routeQuery })
  close()
}

watch([query, fileType, extension], (_value, _oldValue, onCleanup) => {
  const timer = window.setTimeout(search, 260)
  onCleanup(() => window.clearTimeout(timer))
})

watch(open, async value => {
  if (!value) {
    await nextTick()
    previouslyFocused?.focus()
    previouslyFocused = null
    return
  }
  previouslyFocused = document.activeElement instanceof HTMLElement ? document.activeElement : null
  await nextTick()
  searchInput.value?.focus?.()
})
</script>

<template>
  <Teleport to="body">
    <Transition name="search-overlay">
      <section v-if="open" ref="searchDialog" class="global-search" role="dialog" aria-modal="true" aria-label="搜索整个云盘" @keydown="handleDialogKeydown">
        <header class="search-header">
          <div class="search-field">
            <SearchOutlined />
            <a-input ref="searchInput" v-model:value="query" variant="borderless" size="large" placeholder="搜索整个云盘" @press-enter="search()" />
            <kbd>Esc</kbd>
          </div>
          <a-button type="text" aria-label="关闭搜索" @click="close"><CloseOutlined /></a-button>
        </header>

        <div class="search-filters">
          <button v-for="filter in filters" :key="filter.key" type="button" :class="{ active: fileType === filter.key }" :aria-pressed="fileType === filter.key" @click="fileType = filter.key">{{ filter.label }}</button>
          <span class="filter-note">类型与后缀会逐页筛选云端结果</span>
          <label>后缀 <input v-model="extension" aria-label="文件后缀" placeholder="例如 mp4" /></label>
        </div>

        <main class="search-results">
          <div v-if="loading" class="search-state"><LoadingOutlined spin /> 正在搜索整个云盘…</div>
          <a-result v-else-if="error && !results.length" status="error" title="搜索失败" :sub-title="error"><template #extra><a-button @click="search()">重试</a-button></template></a-result>
          <div v-else-if="searched && !results.length" class="search-empty">
            <a-empty description="当前页没有匹配的文件" />
            <a-button v-if="hasMore" :loading="loadingMore" @click="loadMore">继续搜索后续结果</a-button>
          </div>
          <div v-else-if="!searched" class="search-state"><SearchOutlined /> 输入文件名，或直接选择类型与后缀</div>
          <template v-else>
            <button v-for="record in results" :key="pick(record, ['fileId', 'id'], record.fileName)" type="button" class="search-result" @click="showInFiles(record)">
              <span class="result-icon"><component :is="resultIcon(record)" /></span>
              <span class="result-main">
                <strong>{{ pick(record, ['fileName', 'name'], '未命名文件') }}</strong>
                <small>{{ pick(record, ['path', 'parentPath', 'fullPath'], '全部文件') }}</small>
              </span>
              <span class="result-meta">{{ isFolder(record) ? '文件夹' : formatSize(Number(pick(record, ['fileSize', 'size'], '0'))) }}</span>
              <span class="result-meta">{{ formatTime(Number(pick(record, ['lastUpdateTime', 'updatedAt', 'modifyTime'], '0'))) }}</span>
            </button>
            <div v-if="error || hasMore" class="search-results-footer">
              <span v-if="error">{{ error }}</span>
              <a-button v-if="hasMore" :loading="loadingMore" @click="loadMore">加载更多</a-button>
            </div>
          </template>
        </main>
      </section>
    </Transition>
  </Teleport>
</template>

<style scoped>
.global-search { position: fixed; z-index: 3000; inset: 0; overflow: auto; color: var(--text-1, #262626); background: color-mix(in srgb, var(--app-bg, #fafafa) 97%, transparent); backdrop-filter: blur(18px); }
.search-header { display: grid; width: min(920px, calc(100% - 48px)); grid-template-columns:1fr auto; align-items: center; gap: 14px; margin: 48px auto 0; }
.search-field { display: flex; align-items: center; gap: 12px; padding: 8px 14px; border-bottom: 2px solid var(--text-1, #262626); }
.search-field :deep(.ant-input) { font-size: 22px; }
.search-field kbd { padding: 3px 7px; border: 1px solid var(--line, #e5e5e5); border-radius: 5px; color: var(--text-3, #737373); background: var(--surface, #fff); font-size: 11px; }
.search-filters { display: flex; width: min(920px, calc(100% - 48px)); align-items: center; gap: 6px; margin: 22px auto; flex-wrap: wrap; }
.search-filters button { padding: 6px 12px; border: 0; border-radius: 8px; color: var(--text-2, #525252); background: transparent; cursor: pointer; }
.search-filters button:hover, .search-filters button.active { color: var(--primary-strong, #171717); background: var(--primary-soft, #f5f5f5); }
.filter-note { color: var(--text-3, #737373); font-size: 11px; }
.search-filters label { display: flex; align-items: center; gap: 6px; margin-left: auto; color: var(--text-3, #737373); font-size: 12px; }
.search-filters input { width: 110px; padding: 6px 8px; border: 0; border-bottom: 1px solid var(--line, #e5e5e5); background: transparent; }
.search-filters input:focus-visible { border-bottom-color: var(--primary, #262626); outline: 2px solid var(--primary, #262626); outline-offset: 2px; }
.search-results { width: min(920px, calc(100% - 48px)); margin: 0 auto 64px; }
.search-result { display: grid; width: 100%; grid-template-columns:36px minmax(0,1fr) 110px 160px; align-items: center; gap: 12px; padding: 13px 8px; border: 0; border-bottom: 1px solid var(--line, #e5e5e5); background: transparent; text-align: left; cursor: default; }
.search-result:hover { background: var(--surface-hover, #f5f5f5); }
.result-icon { display: grid; width: 32px; height: 32px; place-items: center; border-radius: 9px; color: var(--primary-strong, #171717); background: var(--primary-soft, #f5f5f5); }
.result-main { min-width: 0; }
.result-main strong, .result-main small { display: block; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.result-main small, .result-meta { margin-top: 3px; color: var(--text-3, #737373); font-size: 12px; }
.search-state { display: flex; align-items: center; justify-content: center; gap: 10px; min-height: 260px; color: var(--text-3, #737373); }
.search-empty { display: grid; min-height: 260px; place-content: center; justify-items: center; gap: 16px; }
.search-results-footer { display: flex; align-items: center; justify-content: center; gap: 12px; padding: 20px; color: var(--danger, #ef4444); font-size: 12px; }
.search-overlay-enter-active, .search-overlay-leave-active { transition: opacity .16s ease, transform .16s ease; }
.search-overlay-enter-from, .search-overlay-leave-to { opacity: 0; transform: translateY(-8px); }
@media (max-width: 720px) {
  .search-header, .search-filters, .search-results { width: calc(100% - 28px); }
  .search-header { margin-top: 22px; }
  .search-result { grid-template-columns:36px minmax(0,1fr); }
  .result-meta { display: none; }
  .search-filters label { width: 100%; margin: 8px 0 0; }
}
</style>
