import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import {
  createFolderOpenPreference,
  DEFAULT_FOLDER_OPEN_MODE,
  FOLDER_OPEN_MODE,
  FOLDER_OPEN_PREFERENCE_STORAGE_KEY,
} from './composables/useFolderOpenPreference.js'

function createMemoryStorage(initialEntries = []) {
  const values = new Map(initialEntries)
  return {
    getItem(key) {
      return values.get(key) ?? null
    },
    setItem(key, value) {
      values.set(key, value)
    },
  }
}

test('folder open preference defaults to double click', () => {
  const preference = createFolderOpenPreference(createMemoryStorage())

  assert.equal(DEFAULT_FOLDER_OPEN_MODE, FOLDER_OPEN_MODE.DOUBLE_CLICK)
  assert.equal(preference.folderOpenMode.value, FOLDER_OPEN_MODE.DOUBLE_CLICK)
})

test('folder open preference persists and restores single click mode', () => {
  const storage = createMemoryStorage()
  const preference = createFolderOpenPreference(storage)

  preference.setFolderOpenMode(FOLDER_OPEN_MODE.SINGLE_CLICK)

  assert.equal(
    storage.getItem(FOLDER_OPEN_PREFERENCE_STORAGE_KEY),
    FOLDER_OPEN_MODE.SINGLE_CLICK,
  )
  assert.equal(
    createFolderOpenPreference(storage).folderOpenMode.value,
    FOLDER_OPEN_MODE.SINGLE_CLICK,
  )
})

test('folder open preference falls back safely for invalid or unavailable storage', () => {
  const invalidStorage = createMemoryStorage([
    [FOLDER_OPEN_PREFERENCE_STORAGE_KEY, 'unsupported-mode'],
  ])
  assert.equal(
    createFolderOpenPreference(invalidStorage).folderOpenMode.value,
    FOLDER_OPEN_MODE.DOUBLE_CLICK,
  )

  const unavailableStorage = {
    getItem() {
      throw new Error('storage unavailable')
    },
    setItem() {
      throw new Error('storage unavailable')
    },
  }
  const preference = createFolderOpenPreference(unavailableStorage)
  assert.doesNotThrow(() => preference.setFolderOpenMode(FOLDER_OPEN_MODE.SINGLE_CLICK))
  assert.equal(preference.folderOpenMode.value, FOLDER_OPEN_MODE.SINGLE_CLICK)
})

test('settings exposes the folder open preference as a dedicated panel', async () => {
  const [viewSource, panelSource] = await Promise.all([
    readFile(new URL('./views/SystemSettingsView.vue', import.meta.url), 'utf8'),
    readFile(new URL('./components/settings/PreferenceSettingsPanel.vue', import.meta.url), 'utf8'),
  ])

  assert.match(viewSource, /<a-tab-pane key="preference">/)
  assert.match(viewSource, /<PreferenceSettingsPanel\s*\/>/)
  assert.match(panelSource, />偏好设置</)
  assert.match(panelSource, />单击打开</)
  assert.match(panelSource, />双击打开</)
})
