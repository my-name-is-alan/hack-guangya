import assert from 'node:assert/strict'
import { once } from 'node:events'
import fsp from 'node:fs/promises'
import http from 'node:http'
import os from 'node:os'
import path from 'node:path'
import test from 'node:test'
import { startTestServer, stopTestServer, waitUntil } from './test-helpers.mjs'

async function requestBody(request) {
  const chunks = []
  for await (const chunk of request) chunks.push(chunk)
  return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {}
}

function sendJson(response, payload) {
  response.writeHead(200, { 'content-type': 'application/json' })
  response.end(JSON.stringify(payload))
}

test('Web 离线文件名混淆会持久化任务并在成功后恢复原名称', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-offline-obfuscation-'))
  const calls = []
  const tasks = []
  const upstream = http.createServer(async (request, response) => {
    const body = await requestBody(request)
    calls.push({ path: request.url, body })
    if (request.url === '/cloudcollection/v1/create_task') {
      const sequence = tasks.length + 1
      const task = {
        taskId: `offline-obfuscated-${sequence}`,
        fileId: `offline-file-${sequence}`,
        fileName: body.newName,
        status: 2,
        progress: sequence === 1 ? 20 : 34,
      }
      tasks.push(task)
      return sendJson(response, { code: 0, data: { taskId: task.taskId } })
    }
    if (request.url === '/cloudcollection/v1/list_task') {
      return sendJson(response, {
        code: 0,
        data: {
          total: tasks.length,
          cursor: '',
          list: tasks,
        },
      })
    }
    if (request.url === '/userres/v1/file/rename') return sendJson(response, { code: 0, data: {} })
    return sendJson(response, { code: 0, data: {} })
  })

  let instance
  try {
    upstream.listen(0, '127.0.0.1')
    await once(upstream, 'listening')
    instance = await startTestServer(root, {
      GUANGYA_API_BASE: `http://127.0.0.1:${upstream.address().port}`,
      GUANGYA_TOKEN: 'offline-obfuscation-token',
    })
    const base = `http://127.0.0.1:${instance.port}`

    const enabled = await fetch(`${base}/api/settings/offline`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ filename_obfuscation_enabled: true }),
    }).then((response) => response.json())
    assert.equal(enabled.filename_obfuscation_enabled, true)

    const createdResponse = await fetch(`${base}/api/offline`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        url: 'ed2k://|file|Original.Movie.mkv|1024|0123456789ABCDEF0123456789ABCDEF|/',
        parent_id: 'target-folder',
      }),
    })
    assert.equal(createdResponse.status, 200, await createdResponse.clone().text())
    const created = await createdResponse.json()
    assert.equal(created.data.nameRestoreStatus, 'pending')
    assert.equal(created.data.originalName, 'Original.Movie.mkv')

    const ed2kCreateCall = calls.find((call) => call.path === '/cloudcollection/v1/create_task')
    assert.equal(ed2kCreateCall.body.parentId, 'target-folder')
    assert.match(ed2kCreateCall.body.url, /^ed2k:\/\/\|file\|gy_[a-f0-9]{20}\.mkv\|1024\|/)
    assert.doesNotMatch(ed2kCreateCall.body.url, /Original\.Movie/)
    assert.match(ed2kCreateCall.body.newName, /^gy_[a-f0-9]{20}\.mkv$/)
    assert.notEqual(ed2kCreateCall.body.newName, 'Original.Movie.mkv')

    await waitUntil(() => calls.some((call) => call.path === '/userres/v1/file/rename'))
    assert.deepEqual(calls.find((call) => call.path === '/userres/v1/file/rename').body, {
      fileId: 'offline-file-1',
      newName: 'Original.Movie.mkv',
    })

    const magnetResponse = await fetch(`${base}/api/offline`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        url: 'magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&dn=Original%20Magnet%20Folder&xl=1024',
        parent_id: 'target-folder',
        file_indexes: [0, 2],
      }),
    })
    assert.equal(magnetResponse.status, 200, await magnetResponse.clone().text())
    const createCalls = calls.filter((call) => call.path === '/cloudcollection/v1/create_task')
    const magnetCreateCall = createCalls.at(-1)
    assert.equal(magnetCreateCall.body.parentId, 'target-folder')
    assert.equal(magnetCreateCall.body.url, 'magnet:?xt=urn:btih:0123456789ABCDEF0123456789ABCDEF01234567&xl=1024')
    assert.doesNotMatch(magnetCreateCall.body.url, /(?:^|[?&])dn=/i)
    assert.deepEqual(magnetCreateCall.body.fileIndexes, [0, 2])
    assert.match(magnetCreateCall.body.newName, /^gy_[a-f0-9]{20}$/)
    assert.notEqual(magnetCreateCall.body.newName, 'Original Magnet Folder')

    await waitUntil(() => calls.some((call) => call.path === '/userres/v1/file/rename'
      && call.body.fileId === 'offline-file-2'))
    assert.deepEqual(calls.find((call) => call.path === '/userres/v1/file/rename'
      && call.body.fileId === 'offline-file-2').body, {
      fileId: 'offline-file-2',
      newName: 'Original Magnet Folder',
    })

    const listed = await fetch(`${base}/api/offline?cursor=&pageSize=100`).then((response) => response.json())
    assert.equal(listed.data.list[0].status, 2)
    assert.equal(listed.data.list[0].progress, 20)
    assert.equal(listed.data.list[0].fileName, 'Original.Movie.mkv')
    assert.equal(listed.data.list[0].nameRestoreStatus, 'restored')
    assert.equal(listed.data.list[1].progress, 34)
    assert.equal(listed.data.list[1].fileName, 'Original Magnet Folder')
    assert.equal(listed.data.list[1].nameRestoreStatus, 'restored')

    const settings = await fetch(`${base}/api/settings/offline`).then((response) => response.json())
    assert.equal(settings.filename_obfuscation_enabled, true)
    assert.equal(settings.pending_restores, 0)
  } finally {
    await stopTestServer(instance)
    if (upstream.listening) await new Promise((resolve) => upstream.close(resolve))
    await fsp.rm(root, { recursive: true, force: true })
  }
})
