import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const panelSource = readFile(
  new URL('./components/settings/HdhiveSettingsPanel.vue', import.meta.url),
  'utf8',
)

test('HDHive switch persists immediately and rolls back when saving fails', async () => {
  const source = await panelSource

  assert.match(source, /@change="saveEnabled"/)
  assert.match(source, /async function saveEnabled\(enabled: boolean\)/)
  assert.match(source, /bridge\.invoke\('update_hdhive_config',\s*\{[\s\S]*?enabled,/)
  assert.match(source, /form\.enabled = session\.state\.hdhive\?\.enabled !== false/)
  assert.match(source, /message\.success\(form\.enabled \? 'HDHive 已开启' : 'HDHive 已关闭'\)/)
})

test('HDHive settings exposes the persisted instance id and complete binding guide', async () => {
  const source = await panelSource

  assert.match(source, /session\.state\.hdhive\?\.instance_id/)
  assert.match(source, /同步实例 ID/)
  assert.match(source, /navigator\.clipboard\.writeText\(instanceId\.value\)/)
  assert.match(source, /HDHive 管理后台 → 光鸭同步 → 添加账号/)
  assert.match(source, /Telegram 数字 ID/)
  assert.match(source, /HMAC 密钥/)
})
