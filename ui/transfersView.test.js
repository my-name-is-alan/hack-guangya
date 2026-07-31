import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const transfersViewSource = readFile(new URL('./views/TransfersView.vue', import.meta.url), 'utf8')

test('TransfersView exposes truncated task details by mouse and keyboard', async () => {
  const source = await transfersViewSource

  assert.match(source, /<a-tooltip :title="item\.stage" :trigger="\['hover', 'focus'\]"/)
  assert.match(source, /<span class="transfer-detail" tabindex="0" :aria-label="`任务详情：\$\{item\.stage\}`"/)
  assert.match(source, /\.transfer-detail:focus-visible/)
})

test('TransfersView displays download failure reasons instead of hiding them behind paths', async () => {
  const source = await transfersViewSource

  assert.match(source, /if \(item\.status === 'failed' && item\.error\) return `失败原因：\$\{item\.error\}`/)
  assert.match(source, /<a-tooltip :title="downloadDetail\(item\)" :trigger="\['hover', 'focus'\]"/)
  assert.match(source, /text-overflow:\s*ellipsis/)
})

test('TransfersView labels the backend window speed and shows a measuring state', async () => {
  const source = await transfersViewSource

  assert.match(source, /最近约 5 秒的已确认上传速度/)
  assert.match(source, /item\.state === 'uploading'/)
  assert.match(source, /测速中…/)
})
