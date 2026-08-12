import assert from 'node:assert/strict'
import test from 'node:test'
import {
  requestRecycleBinClear,
  subscribeRecycleBinClear,
  waitForRecycleBinClear,
} from './recycleBinClearOperation.js'

function deferred() {
  let resolve
  let reject
  const promise = new Promise((accept, decline) => {
    resolve = accept
    reject = decline
  })
  return { promise, reject, resolve }
}

test('清空回收站并发点击只执行一次实际请求', async () => {
  const pending = deferred()
  const states = []
  let calls = 0
  const unsubscribe = subscribeRecycleBinClear(active => states.push(active))
  const first = requestRecycleBinClear(() => {
    calls += 1
    return pending.promise
  })
  const duplicate = requestRecycleBinClear(() => {
    calls += 1
    return Promise.resolve()
  })

  assert.equal(first.started, true)
  assert.equal(duplicate.started, false)
  assert.equal(first.promise, duplicate.promise)
  assert.equal(calls, 0)
  await Promise.resolve()
  assert.equal(calls, 1)
  pending.resolve()
  await Promise.all([first.promise, duplicate.promise])
  assert.deepEqual(states, [false, true, false])
  unsubscribe()
})

test('清空失败会释放共享状态并允许重试', async () => {
  const states = []
  const unsubscribe = subscribeRecycleBinClear(active => states.push(active))
  const failed = requestRecycleBinClear(() => Promise.reject(new Error('清空失败')))
  await assert.rejects(failed.promise, /清空失败/)

  let retried = 0
  const retry = requestRecycleBinClear(async () => { retried += 1 })
  assert.equal(retry.started, true)
  await retry.promise
  assert.equal(retried, 1)
  assert.deepEqual(states, [false, true, false, true, false])
  unsubscribe()
})

test('导航卸载再挂载会继承进行中状态并等待同一个请求', async () => {
  const pending = deferred()
  const firstViewStates = []
  const secondViewStates = []
  const unsubscribeFirst = subscribeRecycleBinClear(active => firstViewStates.push(active))
  const clear = requestRecycleBinClear(() => pending.promise)
  unsubscribeFirst()

  const unsubscribeSecond = subscribeRecycleBinClear(active => secondViewStates.push(active))
  let listCanLoad = false
  const waiting = waitForRecycleBinClear().then(() => { listCanLoad = true })
  await Promise.resolve()
  assert.equal(listCanLoad, false)
  assert.deepEqual(firstViewStates, [false, true])
  assert.deepEqual(secondViewStates, [true])

  pending.resolve()
  await Promise.all([clear.promise, waiting])
  assert.equal(listCanLoad, true)
  assert.deepEqual(secondViewStates, [true, false])
  unsubscribeSecond()
})

test('视图订阅异常不会把已经成功的清空请求改成失败', async () => {
  const unsubscribe = subscribeRecycleBinClear(() => { throw new Error('视图已销毁') })
  const clear = requestRecycleBinClear(async () => 'cleared')
  assert.equal(await clear.promise, 'cleared')
  unsubscribe()
})

test('未结束的清空请求始终保持 singleflight，不会重复执行破坏操作', async () => {
  const pending = deferred()
  let calls = 0
  const stuck = requestRecycleBinClear(() => {
    calls += 1
    return pending.promise
  })
  const duplicate = requestRecycleBinClear(async () => { calls += 1 })

  await Promise.resolve()
  assert.equal(calls, 1)
  assert.equal(duplicate.started, false)
  assert.equal(duplicate.promise, stuck.promise)

  pending.resolve()
  await Promise.all([stuck.promise, duplicate.promise])
})
