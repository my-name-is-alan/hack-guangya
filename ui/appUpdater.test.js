import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import { normalizeUpdateMetadata } from './updateMetadata.js'

const read = (path) => readFile(new URL(path, import.meta.url), 'utf8')

test('desktop updater is mounted independently from login and is exposed in settings', async () => {
  const [root, settings] = await Promise.all([
    read('./RootApp.vue'),
    read('./views/SystemSettingsView.vue'),
  ])

  assert.match(root, /updater\.initialize\(\)/)
  assert.match(root, /<AppUpdatePrompt\s*\/>/)
  assert.match(settings, /v-if="isTauri" key="update"/)
  assert.match(settings, /<UpdateSettingsPanel\s*\/>/)
})

test('updater store checks on startup, reports progress and only installs after confirmation', async () => {
  const source = await read('./stores/updater.ts')

  assert.match(source, /localStorage\.getItem\(AUTO_CHECK_KEY\) !== 'false'/)
  assert.match(source, /bridge\.invoke\('get_app_version'\)/)
  assert.match(source, /bridge\.invoke\('fetch_app_update'\)/)
  assert.match(source, /bridge\.invoke\('install_app_update'\)/)
  assert.match(source, /payload\?\.type !== 'app-update'/)
  assert.match(source, /if \(autoCheckEnabled\.value\) await checkForUpdates\(true\)/)
  assert.match(source, /normalizeUpdateMetadata\(await bridge\.invoke\('fetch_app_update'\)\)/)
})

test('no-update responses never become a phantom update modal', () => {
  assert.equal(normalizeUpdateMetadata(null), null)
  assert.equal(normalizeUpdateMetadata(undefined), null)
  assert.equal(normalizeUpdateMetadata({}), null)
  assert.equal(normalizeUpdateMetadata({ data: null }), null)
  assert.deepEqual(
    normalizeUpdateMetadata({ version: ' 0.1.22 ', current_version: '0.1.21', notes: null }),
    { version: '0.1.22', current_version: '0.1.21', notes: '' },
  )
})

test('Tauri updater is signed and points to the latest GitHub release manifest', async () => {
  const [config, cargo, rust, permissions, packager] = await Promise.all([
    read('../src-tauri/tauri.conf.json'),
    read('../src-tauri/Cargo.toml'),
    read('../src-tauri/src/main.rs'),
    read('../src-tauri/permissions/app.toml'),
    read('../scripts/create-updater-manifest.mjs'),
  ])
  const parsed = JSON.parse(config)

  assert.equal(parsed.bundle.createUpdaterArtifacts, true)
  assert.equal(parsed.plugins.updater.windows.installMode, 'passive')
  assert.equal(parsed.plugins.updater.endpoints[0], 'https://github.com/my-name-is-alan/hack-guangya/releases/latest/download/latest.json')
  assert.match(parsed.plugins.updater.pubkey, /^dW50cnVzdGVk/)
  assert.match(cargo, /tauri-plugin-updater = "2\.10\.1"/)
  assert.match(rust, /plugin\(tauri_plugin_updater::Builder::new\(\)\.build\(\)\)/)
  assert.match(rust, /fetch_app_update/)
  assert.match(rust, /download_and_install/)
  assert.match(permissions, /"install_app_update"/)
  assert.match(packager, /release\/latest\.json|latest\.json/)
})
