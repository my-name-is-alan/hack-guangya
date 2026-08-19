<script setup lang="ts">
import { computed, reactive, shallowRef } from 'vue'
import { message } from 'antdv-next'
import { ExperimentOutlined, PlayCircleOutlined } from '@antdv-next/icons'
import { bridge } from '../../bridge.js'
import { errorText, unwrapData } from '../../formatters.js'

const open = defineModel<boolean>('open', { default: false })

interface ParsedName {
  title: string
  cn_name?: string
  en_name?: string
  year?: number | null
  media_type: string
  season?: number | null
  episode?: number | null
  episode_end?: number | null
  tmdb_id?: number | null
  video_format?: string
  release_group?: string
}

interface MatchPreview {
  ready: boolean
  message: string
  title?: string
  original_title?: string
  year?: number | null
  tmdb_id?: number | null
  media_type?: string
  candidates?: Array<{ tmdb_id: number, title: string, year?: number | null, media_type: string }>
}

interface TestRow {
  name: string
  parsed: ParsedName
  match?: MatchPreview
}

const form = reactive({
  names: '',
  recognition_words: '',
  media_type: '',
  with_match: true,
})
const running = shallowRef(false)
const rows = shallowRef<TestRow[]>([])
const nameCount = computed(() => form.names.split(/\r?\n/).map((line) => line.trim()).filter(Boolean).length)

function seLabel(parsed: ParsedName) {
  if (parsed.season == null && parsed.episode == null) return parsed.media_type === 'movie' ? '电影' : '—'
  const season = parsed.season == null ? 'S?' : `S${String(parsed.season).padStart(2, '0')}`
  if (parsed.episode == null) return season
  const episode = `E${String(parsed.episode).padStart(2, '0')}`
  const end = parsed.episode_end == null ? '' : `-E${String(parsed.episode_end).padStart(2, '0')}`
  return `${season}${episode}${end}`
}

async function runTest() {
  if (!nameCount.value) {
    message.warning('请先粘贴要测试的文件名（每行一个）')
    return
  }
  running.value = true
  try {
    const result = unwrapData(await bridge.invoke('test_media_recognition', {
      input: {
        names: form.names.split(/\r?\n/).map((line) => line.trim()).filter(Boolean),
        recognition_words: form.recognition_words,
        media_type: form.media_type,
        with_match: form.with_match,
      },
    })) as { items: TestRow[] }
    rows.value = Array.isArray(result?.items) ? result.items : []
    if (!rows.value.length) message.warning('没有可展示的解析结果')
  } catch (reason) {
    message.error(errorText(reason))
  } finally {
    running.value = false
  }
}
</script>

<template>
  <a-modal v-model:open="open" title="识别测试工具" width="min(940px, 96vw)" :footer="null">
    <div class="recognition-test">
      <div class="dialog-lead">
        <div>
          <strong>解析文件名并预览 TMDB 匹配</strong>
          <span>使用当前识别设置；临时识别词只在本工具内生效，可先在这里调试规则再保存到全局设置。</span>
        </div>
        <ExperimentOutlined class="dialog-icon" />
      </div>

      <div class="test-form">
        <a-textarea
          v-model:value="form.names"
          class="names-input"
          :auto-size="{ minRows: 4, maxRows: 10 }"
          :spellcheck="false"
          placeholder="每行一个文件名或目录名，例如：&#10;凡人修仙传.The.Immortal.Ascension.2020.S01E05.2160p.WEB-DL.mkv&#10;【幻月字幕组】【天国大魔境】【01】【1080P】.mp4"
        />
        <a-textarea
          v-model:value="form.recognition_words"
          class="words-input"
          :auto-size="{ minRows: 4, maxRows: 10 }"
          :spellcheck="false"
          placeholder="临时识别词（可选，格式与设置里的自定义识别词一致）：&#10;屏蔽词&#10;被替换词 => 替换词&#10;(?i)^Alias\.(\d+) => Show.S01E\1"
        />
        <div class="test-controls">
          <a-select
            v-model:value="form.media_type"
            class="type-select"
            :options="[
              { value: '', label: '类型：自动判断' },
              { value: 'movie', label: '类型：电影' },
              { value: 'tv', label: '类型：剧集' },
            ]"
          />
          <a-checkbox v-model:checked="form.with_match">TMDB 匹配预览（消耗 API 调用）</a-checkbox>
          <span class="flex-spacer" />
          <a-button type="primary" :loading="running" :disabled="!nameCount" @click="runTest">
            <PlayCircleOutlined />解析 {{ nameCount ? `${nameCount} 条` : '' }}
          </a-button>
        </div>
      </div>

      <div v-if="rows.length" class="test-results">
        <article v-for="row in rows" :key="row.name" class="result-card">
          <header :title="row.name">{{ row.name }}</header>
          <div class="parsed-grid">
            <span><small>标题</small><strong>{{ row.parsed.title || '—' }}</strong></span>
            <span><small>中文名 / 英文名</small><strong>{{ [row.parsed.cn_name, row.parsed.en_name].filter(Boolean).join(' / ') || '—' }}</strong></span>
            <span><small>年份</small><strong>{{ row.parsed.year ?? '—' }}</strong></span>
            <span><small>类型 / 季集</small><strong>{{ row.parsed.media_type === 'tv' ? '剧集' : '电影' }} · {{ seLabel(row.parsed) }}</strong></span>
            <span><small>技术信息</small><strong>{{ [row.parsed.video_format, row.parsed.release_group].filter(Boolean).join(' · ') || '—' }}</strong></span>
            <span><small>内嵌 TMDB ID</small><strong>{{ row.parsed.tmdb_id ?? '—' }}</strong></span>
          </div>
          <div v-if="row.match" class="match-row" :class="row.match.ready ? 'match-ok' : 'match-fail'">
            <a-tag :color="row.match.ready ? 'success' : 'warning'">{{ row.match.ready ? '匹配成功' : '未自动匹配' }}</a-tag>
            <template v-if="row.match.ready">
              <strong>{{ row.match.title }}</strong>
              <span>({{ row.match.year ?? '?' }}) · {{ row.match.media_type === 'tv' ? '剧集' : '电影' }} · TMDB {{ row.match.tmdb_id }}</span>
            </template>
            <template v-else>
              <span>{{ row.match.message }}</span>
              <span v-if="row.match.candidates?.length" class="candidate-list">
                候选：{{ row.match.candidates.map((candidate) => `${candidate.title}${candidate.year ? `(${candidate.year})` : ''}`).join(' / ') }}
              </span>
            </template>
          </div>
        </article>
      </div>
      <a-empty v-else-if="!running" description="解析结果会显示在这里" class="test-empty" />
    </div>
  </a-modal>
</template>

<style scoped>
.recognition-test { padding-top: 4px; }
.dialog-lead { display: flex; align-items: flex-start; justify-content: space-between; gap: 18px; margin-bottom: 14px; }
.dialog-lead strong, .dialog-lead span { display: block; }
.dialog-lead strong { font-size: 16px; }
.dialog-lead span { margin-top: 5px; color: var(--text-3, #737373); font-size: 12px; line-height: 1.55; }
.dialog-icon { color: var(--primary, #262626); font-size: 24px; }
.test-form { display: grid; grid-template-columns: 1.2fr 1fr; gap: 10px; }
.names-input, .words-input { font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 12px; }
.test-controls { display: flex; grid-column: 1 / -1; align-items: center; gap: 14px; }
.type-select { width: 150px; }
.flex-spacer { flex: 1; }
.test-results { display: grid; gap: 10px; max-height: 46vh; margin-top: 14px; overflow: auto; padding-right: 4px; scrollbar-width: thin; }
.result-card { display: grid; gap: 8px; padding: 10px 12px; border: 1px solid var(--line, #e5e5e5); border-radius: 10px; background: var(--surface, #fff); }
.result-card header { overflow: hidden; color: var(--text-2, #525252); font-family: ui-monospace, SFMono-Regular, Consolas, monospace; font-size: 11px; text-overflow: ellipsis; white-space: nowrap; }
.parsed-grid { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 6px 12px; }
.parsed-grid > span { display: grid; gap: 2px; min-width: 0; }
.parsed-grid small { color: var(--text-3, #737373); font-size: 10px; }
.parsed-grid strong { overflow: hidden; font-size: 12px; text-overflow: ellipsis; white-space: nowrap; }
.match-row { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; padding: 7px 9px; border-radius: 8px; background: var(--surface-muted, #fafafa); font-size: 12px; }
.match-row span { color: var(--text-2, #525252); }
.candidate-list { overflow-wrap: anywhere; }
.test-empty { margin-top: 20px; }
@media (max-width: 720px) {
  .test-form { grid-template-columns: 1fr; }
  .parsed-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); }
}
</style>
