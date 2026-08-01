import assert from 'node:assert/strict'
import test from 'node:test'
import {
  fileListCacheKey,
  shouldApplyFileListResponse,
  withFileListTimeout,
} from './fileListRequest.js'

test('文件目录只接受当前目录的最新响应', () => {
  assert.equal(shouldApplyFileListResponse(3, 3, 'folder-a', 'folder-a'), true)
  assert.equal(shouldApplyFileListResponse(2, 3, 'folder-a', 'folder-a'), false)
  assert.equal(shouldApplyFileListResponse(3, 3, 'folder-a', 'folder-b'), false)
  assert.notEqual(fileListCacheKey('folder-a', 0), fileListCacheKey('folder-b', 0))
  assert.notEqual(fileListCacheKey('folder-a', 0), fileListCacheKey('folder-a', 1))
})

test('文件目录请求超时后不会继续阻塞界面', async () => {
  await assert.rejects(
    () => withFileListTimeout(new Promise(() => {}), 10),
    /文件目录加载超过 1 秒，请重试/,
  )
  assert.equal(await withFileListTimeout(Promise.resolve('ok'), 100), 'ok')
})
