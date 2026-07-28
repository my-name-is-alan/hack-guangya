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
