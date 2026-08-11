import assert from 'node:assert/strict'
import test from 'node:test'
import { offlineProgress } from './formatters.js'

test('已完成的离线任务始终收口到 100%', () => {
  assert.equal(offlineProgress({ status: 2, progress: 20 }), 100)
  assert.equal(offlineProgress({ status: 'completed', progress: 34 }), 100)
})

test('运行中的离线任务仍按上游百分比或大小计算', () => {
  assert.equal(offlineProgress({ status: 1, progress: 34 }), 34)
  assert.equal(offlineProgress({ status: 1, progress: 0.42 }), 42)
  assert.equal(offlineProgress({ status: 1, downloadedSize: 25, totalSize: 100 }), 25)
  assert.equal(offlineProgress({ status: 0 }), null)
})
