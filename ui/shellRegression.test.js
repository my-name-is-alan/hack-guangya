import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const read = (relative) => readFile(new URL(relative, import.meta.url), 'utf8')

test('organizer task table remains constrained by its card', async () => {
  const source = await read('./views/OrganizerView.vue')

  assert.match(source, /class="organizer-spin"/)
  assert.match(source, /:scroll="\{ x: 990 \}"/)
  assert.doesNotMatch(source, /fixed: 'right'/)
  assert.match(source, /\.organizer-spin, \.organizer-page \{ width: 100%; min-width: 0; max-width: 100%; \}/)
  assert.match(source, /\.jobs-block \{ overflow: hidden;/)
  assert.match(source, /\.jobs-block :deep\(\.ant-table-wrapper\) \{ width: 100%; min-width: 0; max-width: 100%; margin-inline: 0; \}/)
})

test('settings entry stays lightweight and constrains the actual tabs body', async () => {
  const [settings, account] = await Promise.all([
    read('./views/SystemSettingsView.vue'),
    read('./components/settings/AccountSettingsPanel.vue'),
  ])

  assert.match(settings, /defineAsyncComponent\(\(\) => import\('\.\.\/components\/settings\/AccountSettingsPanel\.vue'\)\)/)
  assert.match(settings, /\.settings-tabs > :deep\(\.ant-tabs-body-holder\)/)
  assert.doesNotMatch(settings, /\.ant-tabs-content-holder/)
  assert.doesNotMatch(account, /onMounted\(refreshAccountData\)/)
  assert.match(account, /@click="refreshAccountData"/)
})

test('the app suppresses the native browser context menu globally', async () => {
  const source = await read('./RootApp.vue')

  assert.match(source, /document\.addEventListener\('contextmenu', preventNativeContextMenu\)/)
  assert.match(source, /document\.removeEventListener\('contextmenu', preventNativeContextMenu\)/)
  assert.match(source, /function preventNativeContextMenu\(event: MouseEvent\) \{\s*event\.preventDefault\(\)/)
})

test('route switches mount synchronously so repeated navigation cannot strand a blank page', async () => {
  const source = await read('./components/shell/AppShell.vue')

  assert.match(source, /<RouterView\s*\/>/)
  assert.doesNotMatch(source, /<Transition[^>]*mode=["']out-in["']/)
  assert.doesNotMatch(source, /route-fade/)
})
