import { readonly, shallowRef } from 'vue'

export const FOLDER_OPEN_MODE = Object.freeze({
  SINGLE_CLICK: 'single-click',
  DOUBLE_CLICK: 'double-click',
})

export const DEFAULT_FOLDER_OPEN_MODE = FOLDER_OPEN_MODE.DOUBLE_CLICK
export const FOLDER_OPEN_PREFERENCE_STORAGE_KEY = 'guangya.file-manager.folder-open-mode'

function resolveBrowserStorage() {
  if (typeof window === 'undefined') return null
  try {
    return window.localStorage
  } catch {
    return null
  }
}

export function normalizeFolderOpenMode(value) {
  return value === FOLDER_OPEN_MODE.SINGLE_CLICK
    ? FOLDER_OPEN_MODE.SINGLE_CLICK
    : DEFAULT_FOLDER_OPEN_MODE
}

export function readFolderOpenMode(storage = resolveBrowserStorage()) {
  try {
    return normalizeFolderOpenMode(storage?.getItem(FOLDER_OPEN_PREFERENCE_STORAGE_KEY))
  } catch {
    return DEFAULT_FOLDER_OPEN_MODE
  }
}

export function createFolderOpenPreference(storage = resolveBrowserStorage()) {
  const folderOpenMode = shallowRef(readFolderOpenMode(storage))

  function setFolderOpenMode(value) {
    const nextMode = normalizeFolderOpenMode(value)
    folderOpenMode.value = nextMode
    try {
      storage?.setItem(FOLDER_OPEN_PREFERENCE_STORAGE_KEY, nextMode)
    } catch {
      // localStorage 不可用时仍保留当前会话内的偏好。
    }
  }

  return {
    folderOpenMode: readonly(folderOpenMode),
    setFolderOpenMode,
  }
}

const sharedFolderOpenPreference = createFolderOpenPreference()

export function useFolderOpenPreference() {
  return sharedFolderOpenPreference
}
