import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const [panel, sessionStore] = await Promise.all([
  readFile(new URL('./components/settings/TransferSettingsPanel.vue', import.meta.url), 'utf8'),
  readFile(new URL('./stores/session.ts', import.meta.url), 'utf8'),
])

test('传输设置区分文件任务并发和单文件 OSS 分片并发', () => {
  assert.match(panel, /同时上传文件数/)
  assert.match(panel, /单文件分片并发/)
  assert.match(panel, /form\.oss_part_concurrency/)
  assert.match(panel, /oss_part_concurrency:\s*Number\(/)
  assert.match(panel, /:min="1" :max="8"/)
  assert.match(panel, /提高并发会增加内存和连接占用/)
  assert.match(sessionStore, /oss_part_concurrency:\s*4/)
})

test('传输设置在窄窗口回落为单列', () => {
  assert.match(panel, /@media \(max-width: 720px\)/)
  assert.match(panel, /\.two-columns \{ grid-template-columns: 1fr; gap: 0; \}/)
})
