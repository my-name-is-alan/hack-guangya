import assert from 'node:assert/strict'
import test from 'node:test'
import { resolveFileShortcut } from './composables/useFileKeyboardShortcuts.js'

const ready = {
  blocked: false,
  fileCount: 4,
  selectedCount: 2,
  clipboardCount: 1,
  canGoBack: true,
  contextMenuOpen: false,
}

test('file manager maps conventional clipboard and selection shortcuts', () => {
  assert.equal(resolveFileShortcut({ key: 'a', ctrlKey: true }, ready), 'selectAll')
  assert.equal(resolveFileShortcut({ key: 'c', ctrlKey: true }, ready), 'copy')
  assert.equal(resolveFileShortcut({ key: 'x', ctrlKey: true }, ready), 'cut')
  assert.equal(resolveFileShortcut({ key: 'v', ctrlKey: true }, ready), 'paste')
  assert.equal(resolveFileShortcut({ key: 'a', metaKey: true }, ready), 'selectAll')
  assert.equal(resolveFileShortcut({ key: 'c', metaKey: true }, ready), 'copy')
  assert.equal(resolveFileShortcut({ key: 'x', metaKey: true }, ready), 'cut')
  assert.equal(resolveFileShortcut({ key: 'v', metaKey: true }, ready), 'paste')
})

test('file manager only handles exact primary modifier clipboard shortcuts', () => {
  for (const key of ['a', 'c', 'x', 'v']) {
    assert.equal(resolveFileShortcut({ key, ctrlKey: true, shiftKey: true }, ready), '')
    assert.equal(resolveFileShortcut({ key, ctrlKey: true, altKey: true }, ready), '')
    assert.equal(resolveFileShortcut({ key, metaKey: true, shiftKey: true }, ready), '')
    assert.equal(resolveFileShortcut({ key, metaKey: true, altKey: true }, ready), '')
  }
})

test('file manager restores F2, Delete, refresh and parent navigation shortcuts', () => {
  assert.equal(resolveFileShortcut({ key: 'F2' }, ready), 'rename')
  assert.equal(resolveFileShortcut({ key: 'Delete' }, ready), 'delete')
  assert.equal(resolveFileShortcut({ key: 'F5' }, ready), 'refresh')
  assert.equal(resolveFileShortcut({ key: 'ArrowUp', altKey: true }, ready), 'goBack')
  assert.equal(resolveFileShortcut({ key: 'Backspace' }, ready), 'goBack')
})

test('file shortcuts leave editable fields and unavailable actions untouched', () => {
  const typingTarget = { closest: () => ({ tagName: 'INPUT' }) }
  assert.equal(resolveFileShortcut({ key: 'Delete', target: typingTarget }, ready), '')
  assert.equal(resolveFileShortcut({ key: 'F2' }, { ...ready, selectedCount: 0 }), '')
  assert.equal(resolveFileShortcut({ key: 'v', ctrlKey: true }, { ...ready, clipboardCount: 0 }), '')
  assert.equal(resolveFileShortcut({ key: 'F5' }, { ...ready, blocked: true }), '')
  assert.equal(resolveFileShortcut({ key: 'Delete', defaultPrevented: true }, ready), '')
  assert.equal(resolveFileShortcut({ key: 'Delete', isComposing: true }, ready), '')
  assert.equal(resolveFileShortcut({ key: 'Delete', repeat: true }, ready), '')
})

test('file shortcuts never pass through dialogs, menus or modal overlays', () => {
  const overlaySelectors = ['[role="dialog"]', '[role="menu"]', '[aria-modal="true"]']
  const shortcuts = [
    { key: 'a', ctrlKey: true },
    { key: 'c', ctrlKey: true },
    { key: 'x', ctrlKey: true },
    { key: 'v', ctrlKey: true },
    { key: 'F2' },
    { key: 'Delete' },
    { key: 'F5' },
    { key: 'ArrowUp', altKey: true },
    { key: 'Backspace' },
    { key: 'Escape' },
  ]

  for (const overlaySelector of overlaySelectors) {
    const target = {
      closest: selector => selector.split(',').map(value => value.trim()).includes(overlaySelector) ? {} : null,
    }
    for (const shortcut of shortcuts) {
      assert.equal(resolveFileShortcut({ ...shortcut, target }, ready), '')
    }
  }
})
