import { onBeforeUnmount, onMounted } from 'vue'

const editableSelector = 'input, textarea, select, [contenteditable="true"], [role="textbox"]'
const overlaySelector = '[role="dialog"], [role="menu"], [aria-modal="true"]'

export function isFileShortcutTypingTarget(target) {
  return Boolean(target?.closest?.(editableSelector))
}

export function isFileShortcutOverlayTarget(target) {
  return Boolean(target?.closest?.(overlaySelector))
}

export function resolveFileShortcut(event, state = {}) {
  if (
    event.defaultPrevented
    || event.isComposing
    || state.blocked
    || isFileShortcutTypingTarget(event.target)
    || isFileShortcutOverlayTarget(event.target)
  ) return ''

  const key = String(event.key || '').toLowerCase()
  const primaryModifier = Boolean(event.ctrlKey || event.metaKey)
  const exactPrimaryModifier = primaryModifier && !event.altKey && !event.shiftKey

  if (event.altKey && key === 'arrowup' && state.canGoBack) return 'goBack'
  if (event.altKey) return ''
  if (event.repeat && (key === 'f2' || key === 'delete' || (exactPrimaryModifier && key === 'v'))) return ''
  if (exactPrimaryModifier && key === 'a' && state.fileCount) return 'selectAll'
  if (exactPrimaryModifier && key === 'c' && state.selectedCount) return 'copy'
  if (exactPrimaryModifier && key === 'x' && state.selectedCount) return 'cut'
  if (exactPrimaryModifier && key === 'v' && state.clipboardCount) return 'paste'
  if (key === 'f2' && state.selectedCount) return 'rename'
  if (key === 'delete' && state.selectedCount) return 'delete'
  if (key === 'backspace' && state.canGoBack) return 'goBack'
  if (key === 'f5') return 'refresh'
  if (key === 'escape' && (state.selectedCount || state.contextMenuOpen)) return 'clearSelection'

  return ''
}

export function useFileKeyboardShortcuts(options) {
  function handleKeydown(event) {
    const action = resolveFileShortcut(event, options.getState())
    if (!action) return
    event.preventDefault()
    options.actions[action]?.()
  }

  onMounted(() => window.addEventListener('keydown', handleKeydown))
  onBeforeUnmount(() => window.removeEventListener('keydown', handleKeydown))
}
