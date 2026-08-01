import assert from 'node:assert/strict';
import { once } from 'node:events';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { startTestServer, stopTestServer, waitUntil } from './test-helpers.mjs';

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
}

function sendJson(response, payload, status = 200) {
  response.writeHead(status, { 'content-type': 'application/json' });
  response.end(JSON.stringify(payload));
}

async function requestJson(base, pathname, { method = 'GET', body, status = 200 } = {}) {
  const response = await fetch(`${base}${pathname}`, {
    method,
    headers: body === undefined ? undefined : { 'content-type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await response.text();
  assert.equal(response.status, status, text);
  return text ? JSON.parse(text) : {};
}

test('Web resource routes preserve the official PC 1.0.2 upstream contracts', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-resource-contract-'));
  const calls = [];
  const operationTasks = [];
  let operationNumber = 0;
  let releaseWait;
  const waitGate = new Promise((resolve) => { releaseWait = resolve; });
  const taskEndpoints = new Set([
    '/userres/v1/file/create_dir',
    '/userres/v1/file/delete_file',
    '/userres/v1/file/recycle_file',
    '/userres/v1/file/clear_recycle_bin',
  ]);

  const upstream = http.createServer(async (request, response) => {
    const body = await readJson(request);
    calls.push({ method: request.method, path: request.url, body });

    if (request.url === '/userres/v1/get_task_status') {
      if (body.taskId === 'wait-delete-task') await waitGate;
      return sendJson(response, { code: 0, data: { status: 2, detail: { code: 0 } } });
    }
    if (taskEndpoints.has(request.url)) {
      const waitsForRelease = request.url === '/userres/v1/file/delete_file'
        && body.fileIds?.includes('normal-delete');
      const taskId = waitsForRelease ? 'wait-delete-task' : `file-operation-${++operationNumber}`;
      operationTasks.push({ path: request.url, body, taskId });
      return sendJson(response, { code: 0, data: { taskId, fileId: 'created-folder' } });
    }

    const supported = new Set([
      '/assets/v1/get_assets',
      '/misc/v1/get_global_config',
      '/userres/v1/file/get_file_detail',
      '/userres/v1/get_user_action',
      '/userres/v1/file/get_file_list',
      '/cloudcollection/v1/list_task',
      '/cloudcollection/v1/resolve_res',
      '/cloudcollection/v1/create_task',
      '/cloudcollection/v2/delete_task',
      '/cloudcollection/v2/retry_task',
      '/nd.bizcloudcollection.s/v1/get_task_statistics',
      '/userres/v1/update_share',
      '/userres/v1/delete_invalid_share',
      '/userres/v1/set_direct_link',
      '/userres/v1/unset_direct_link',
      '/userres/v1/get_direct_link',
      '/userres/v1/delete_share',
    ]);
    if (supported.has(request.url)) return sendJson(response, { code: 0, data: { accepted: true } });
    return sendJson(response, { code: 404, msg: `unexpected ${request.url}` }, 404);
  });

  let instance;
  try {
    upstream.listen(0, '127.0.0.1');
    await once(upstream, 'listening');
    instance = await startTestServer(root, {
      GUANGYA_API_BASE: `http://127.0.0.1:${upstream.address().port}`,
      GUANGYA_TOKEN: 'resource-contract-token',
    });
    const base = `http://127.0.0.1:${instance.port}`;

    await requestJson(base, '/api/assets');
    await requestJson(base, '/api/global-config');
    await requestJson(base, '/api/files/create-folder', {
      method: 'POST',
      body: { parent_id: 'parent-1', dir_name: 'Documents', fail_if_name_exist: false },
    });
    await requestJson(base, '/api/files/detail?fileId=file-1');
    await requestJson(base, '/api/recent?cursor=recent%3Anext&pageSize=25&fileTypes=1,2,1&excludeFileTypes=4&excludeFileTypes=5,4');
    await requestJson(base, '/api/recycle?page=2&pageSize=75');

    let deleteSettled = false;
    const pendingDelete = requestJson(base, '/api/files/delete', {
      method: 'POST',
      body: { file_ids: ['normal-delete', 'normal-delete', 'normal-delete-2'] },
    }).then((value) => {
      deleteSettled = true;
      return value;
    });
    await waitUntil(() => calls.some((call) => call.path === '/userres/v1/get_task_status'
      && call.body.taskId === 'wait-delete-task'));
    await new Promise((resolve) => setTimeout(resolve, 30));
    assert.equal(deleteSettled, false, 'local delete responded before the upstream operation completed');
    releaseWait();
    await pendingDelete;

    await requestJson(base, '/api/recycle/restore', {
      method: 'POST',
      body: { file_ids: ['trashed-1', 'trashed-1'] },
    });
    await requestJson(base, '/api/recycle/delete', {
      method: 'POST',
      body: { file_ids: ['trashed-2'] },
    });
    await requestJson(base, '/api/recycle/clear', { method: 'POST', body: {} });

    await requestJson(base, '/api/offline?page=9&cursor=opaque-next&pageSize=25&status=1&status=2,1&statuses=3');
    await requestJson(base, '/api/offline?page=3&pageSize=40', { status: 400 });
    await requestJson(base, '/api/offline?page=0&cursor=&pageSize=100');
    await requestJson(base, '/api/offline/resolve', {
      method: 'POST',
      body: { url: '  magnet:?xt=urn:btih:resolve-me  ' },
    });
    await requestJson(base, '/api/offline', {
      method: 'POST',
      body: {
        url: 'magnet:?xt=urn:btih:create-me',
        parent_id: 'offline-parent',
        new_name: 'Selected files',
        file_indexes: [3, 1, 3, 0],
      },
    });
    await requestJson(base, '/api/offline/cancel', {
      method: 'POST',
      body: { task_ids: ['task-1', 'task-1', 'task-2'] },
    });
    await requestJson(base, '/api/offline/delete', {
      method: 'POST',
      body: { task_ids: ['task-3'] },
    });
    await requestJson(base, '/api/offline/retry', {
      method: 'POST',
      body: { task_ids: ['task-4', 'task-4'] },
    });
    await requestJson(base, '/api/offline/statistics');

    await requestJson(base, '/api/shares/update', {
      method: 'POST',
      body: {
        id: 'share-1',
        validate_duration: 86_400,
        download_type: 1,
        traffic_limit: '4294967296',
      },
    });
    await requestJson(base, '/api/shares/delete-invalid', { method: 'POST', body: {} });
    await requestJson(base, '/api/direct-link/set', { method: 'POST', body: { file_id: 'file-2' } });
    await requestJson(base, '/api/direct-link/unset', { method: 'POST', body: { file_id: 'file-3' } });
    await requestJson(base, '/api/direct-link/get', {
      method: 'POST',
      body: { file_id: 'file-4', short_link: true },
    });
    await requestJson(base, '/api/shares/delete', {
      method: 'POST',
      body: { ids: ['share-2', 'share-2', 'share-3'] },
    });

    assert.deepEqual(calls.filter((call) => call.path === '/assets/v1/get_assets').map((call) => call.body), [{}]);
    assert.deepEqual(calls.filter((call) => call.path === '/misc/v1/get_global_config').map((call) => call.body), [{}]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/file/create_dir').map((call) => call.body), [{
      parentId: 'parent-1', dirName: 'Documents', failIfNameExist: false,
    }]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/file/get_file_detail').map((call) => call.body), [{ fileId: 'file-1' }]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/get_user_action').map((call) => call.body), [{
      cursor: 'recent:next', pageSize: 25, fileTypes: [1, 2], excludeFileTypes: [4, 5],
    }]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/file/get_file_list').map((call) => call.body), [{
      page: 2, pageSize: 75, parentId: '', dirType: 4, orderBy: 12, sortType: 1,
    }]);
    assert.deepEqual(operationTasks.map(({ path: operationPath, body }) => ({ path: operationPath, body })), [
      { path: '/userres/v1/file/create_dir', body: { parentId: 'parent-1', dirName: 'Documents', failIfNameExist: false } },
      { path: '/userres/v1/file/delete_file', body: { fileIds: ['normal-delete', 'normal-delete-2'] } },
      { path: '/userres/v1/file/recycle_file', body: { fileIds: ['trashed-1'] } },
      { path: '/userres/v1/file/delete_file', body: { fileIds: ['trashed-2'] } },
      { path: '/userres/v1/file/clear_recycle_bin', body: {} },
    ]);
    const checkedTaskIds = calls.filter((call) => call.path === '/userres/v1/get_task_status').map((call) => call.body.taskId);
    assert.deepEqual(checkedTaskIds, operationTasks.map((operation) => operation.taskId));

    assert.deepEqual(calls.filter((call) => call.path === '/cloudcollection/v1/list_task').map((call) => call.body), [
      { cursor: 'opaque-next', pageSize: 25, status: [1, 2, 3] },
      { cursor: '', pageSize: 100 },
    ]);
    assert.deepEqual(calls.filter((call) => call.path === '/cloudcollection/v1/resolve_res').map((call) => call.body), [{
      url: 'magnet:?xt=urn:btih:resolve-me',
    }]);
    assert.deepEqual(calls.filter((call) => call.path === '/cloudcollection/v1/create_task').map((call) => call.body), [{
      url: 'magnet:?xt=urn:btih:create-me',
      parentId: 'offline-parent',
      fileIndexes: [3, 1, 0],
      newName: 'Selected files',
    }]);
    assert.deepEqual(calls.filter((call) => call.path === '/cloudcollection/v2/delete_task').map((call) => call.body), [
      { taskIds: ['task-1', 'task-2'] },
      { taskIds: ['task-3'] },
    ]);
    assert.deepEqual(calls.filter((call) => call.path === '/cloudcollection/v2/retry_task').map((call) => call.body), [
      { taskIds: ['task-4'] },
    ]);
    assert.deepEqual(calls.filter((call) => call.path === '/nd.bizcloudcollection.s/v1/get_task_statistics').map((call) => call.body), [{}]);

    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/update_share').map((call) => call.body), [{
      id: 'share-1', validateDuration: 86_400, downloadType: 1, trafficLimit: '4294967296',
    }]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/delete_invalid_share').map((call) => call.body), [{}]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/set_direct_link').map((call) => call.body), [{ fileId: 'file-2' }]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/unset_direct_link').map((call) => call.body), [{ fileId: 'file-3' }]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/get_direct_link').map((call) => call.body), [{ fileId: 'file-4', shortLink: true }]);
    assert.deepEqual(calls.filter((call) => call.path === '/userres/v1/delete_share').map((call) => call.body), [{ ids: ['share-2', 'share-3'] }]);
    assert.ok(calls.every((call) => call.method === 'POST'), 'all upstream API calls must use POST');

    const retryCount = calls.filter((call) => call.path === '/cloudcollection/v2/retry_task').length;
    await requestJson(base, '/api/offline/retry', {
      method: 'POST',
      body: { task_ids: ['task-5', { invalid: true }] },
      status: 400,
    });
    assert.equal(calls.filter((call) => call.path === '/cloudcollection/v2/retry_task').length, retryCount);
    const updateCount = calls.filter((call) => call.path === '/userres/v1/update_share').length;
    await requestJson(base, '/api/shares/update', {
      method: 'POST',
      body: { id: 'share-4', validate_duration: 12, download_type: 2, traffic_limit: -1 },
      status: 400,
    });
    await requestJson(base, '/api/shares/update', {
      method: 'POST',
      body: { id: 'share-4', validate_duration: 0, download_type: 0, traffic_limit: '1125899906842625' },
      status: 400,
    });
    assert.equal(calls.filter((call) => call.path === '/userres/v1/update_share').length, updateCount);

    const listCount = calls.filter((call) => call.path === '/cloudcollection/v1/list_task').length;
    for (const invalidStatus of [6, 255]) {
      await requestJson(base, `/api/offline?cursor=&status=${invalidStatus}`, { status: 400 });
    }
    assert.equal(calls.filter((call) => call.path === '/cloudcollection/v1/list_task').length, listCount);

    const resolveCount = calls.filter((call) => call.path === '/cloudcollection/v1/resolve_res').length;
    await requestJson(base, '/api/offline/resolve', {
      method: 'POST', body: { url: 'ftp://example.test/file.iso' }, status: 400,
    });
    await requestJson(base, '/api/offline/resolve', {
      method: 'POST', body: { url: 'https://example.test/file\nname.iso' }, status: 400,
    });
    await requestJson(base, '/api/offline/resolve', {
      method: 'POST', body: { url: `https://example.test/${'a'.repeat(8200)}` }, status: 400,
    });
    assert.equal(calls.filter((call) => call.path === '/cloudcollection/v1/resolve_res').length, resolveCount);

    const createCount = calls.filter((call) => call.path === '/cloudcollection/v1/create_task').length;
    await requestJson(base, '/api/offline', {
      method: 'POST',
      body: { url: 'https://example.test/file.iso', file_indexes: [0] },
      status: 400,
    });
    await requestJson(base, '/api/offline', {
      method: 'POST',
      body: { url: 'magnet:?xt=urn:btih:invalid-index', file_indexes: [0, -1] },
      status: 400,
    });
    await requestJson(base, '/api/offline', {
      method: 'POST',
      body: { url: 'magnet:?xt=urn:btih:string-index', file_indexes: ['0'] },
      status: 400,
    });
    await requestJson(base, '/api/offline', {
      method: 'POST',
      body: { url: 'https://example.test/file.iso', parent_id: '../invalid' },
      status: 400,
    });
    await requestJson(base, '/api/offline', {
      method: 'POST',
      body: { url: 'https://example.test/file.iso', new_name: 'bad/name.iso' },
      status: 400,
    });
    assert.equal(calls.filter((call) => call.path === '/cloudcollection/v1/create_task').length, createCount);
  } finally {
    releaseWait?.();
    await stopTestServer(instance);
    if (upstream.listening) await new Promise((resolve) => upstream.close(resolve));
    await fsp.rm(root, { recursive: true, force: true });
  }
});
