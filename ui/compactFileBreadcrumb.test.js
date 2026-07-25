import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'
import {
  buildCompactBreadcrumbLayout,
  COMPACT_BREADCRUMB_MAX_LEVELS,
} from './components/files/compactFileBreadcrumb.ts'

const componentSource = readFile(
  new URL('./components/files/CompactFileBreadcrumb.vue', import.meta.url),
  'utf8',
)

function createPath(length) {
  return Array.from({ length }, (_, index) => ({
    id: index ? `folder-${index}` : '',
    name: index ? `目录 ${index}` : '全部文件',
  }))
}

test('paths with no more than four levels remain fully visible', () => {
  const path = createPath(COMPACT_BREADCRUMB_MAX_LEVELS)
  const layout = buildCompactBreadcrumbLayout(path)

  assert.equal(layout.collapsed, false)
  assert.deepEqual(layout.hidden, [])
  assert.deepEqual(layout.visible.map(item => item.index), [0, 1, 2, 3])
  assert.deepEqual(layout.visible.map(item => item.segment), path)
})

test('deep paths keep the root and last three levels while hiding middle ancestors', () => {
  const path = createPath(7)
  const layout = buildCompactBreadcrumbLayout(path)

  assert.equal(layout.collapsed, true)
  assert.deepEqual(layout.visible.map(item => item.index), [0, 4, 5, 6])
  assert.deepEqual(layout.hidden.map(item => item.index), [1, 2, 3])
  assert.deepEqual(layout.hidden.map(item => item.segment.name), ['目录 1', '目录 2', '目录 3'])
  assert.equal(path.length, 7, 'layout calculation must not mutate the source path')
})

test('component exposes an accessible dropdown and emits index/id navigation targets', async () => {
  const source = await componentSource

  assert.match(source, /<ADropdown[\s\S]*:menu="\{ items: hiddenMenuItems, onClick: handleHiddenMenuClick \}"/)
  assert.match(source, /aria-haspopup="menu"/)
  assert.match(source, /aria-current="page"/)
  assert.match(source, /emit\('navigate', \{ index, id: segment\.id \}\)/)
  assert.match(source, /text-overflow: ellipsis/)
})
