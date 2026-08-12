import assert from 'node:assert/strict';
import { once } from 'node:events';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { startTestServer, stopTestServer } from './test-helpers.mjs';

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
}

function sendJson(response, payload, status = 200) {
  response.writeHead(status, { 'content-type': 'application/json' });
  response.end(JSON.stringify(payload));
}

async function requestJson(base, pathname, options = {}) {
  const response = await fetch(`${base}${pathname}`, {
    ...options,
    headers: options.body == null ? options.headers : { 'content-type': 'application/json', ...(options.headers || {}) },
    body: options.body == null || typeof options.body === 'string' ? options.body : JSON.stringify(options.body),
  });
  const payload = await response.json();
  assert.equal(response.ok, true, JSON.stringify(payload));
  return payload;
}

test('opaque access-token refresh keeps polling the original recycle clear task without another POST', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-recycle-refresh-'));
  let refreshCalls = 0;
  let clearPosts = 0;
  let expireClearStatusOnce = true;
  const checkedTasks = [];

  const accountServer = http.createServer(async (request, response) => {
    const body = request.method === 'POST' ? await readJson(request) : {};
    if (request.url === '/v1/auth/token' && body.grant_type?.includes('device_code')) {
      return sendJson(response, { access_token: 'opaque-access-old', refresh_token: 'opaque-refresh' });
    }
    if (request.url === '/v1/auth/token' && body.grant_type === 'refresh_token') {
      refreshCalls += 1;
      return sendJson(response, { access_token: 'opaque-access-new', refresh_token: 'opaque-refresh-next' });
    }
    if (request.url === '/v1/user/me') return sendJson(response, { error: 'profile unavailable' }, 404);
    return sendJson(response, { error: 'not found' }, 404);
  });
  const apiServer = http.createServer(async (request, response) => {
    const body = await readJson(request);
    if (request.url === '/userres/v1/file/clear_recycle_bin') {
      clearPosts += 1;
      return sendJson(response, { code: 0, data: { taskId: 'clear-task-one' } });
    }
    if (request.url === '/userres/v1/get_task_status') {
      checkedTasks.push(body.taskId);
      if (expireClearStatusOnce && request.headers.authorization === 'Bearer opaque-access-old') {
        expireClearStatusOnce = false;
        return sendJson(response, { code: 110, msg: 'expired while polling clear task' });
      }
      return sendJson(response, { code: 0, data: { status: 1 } });
    }
    if (request.url === '/userres/v1/file/get_file_list') {
      if (request.headers.authorization === 'Bearer opaque-access-old') {
        return sendJson(response, { code: 110, msg: 'expired' });
      }
      return sendJson(response, { code: 0, data: { list: [], total: 0 } });
    }
    return sendJson(response, { code: 404, msg: 'not found' }, 404);
  });

  let instance;
  try {
    accountServer.listen(0, '127.0.0.1');
    apiServer.listen(0, '127.0.0.1');
    await Promise.all([once(accountServer, 'listening'), once(apiServer, 'listening')]);
    instance = await startTestServer(root, {
      GUANGYA_ACCOUNT_BASE: `http://127.0.0.1:${accountServer.address().port}`,
      GUANGYA_API_BASE: `http://127.0.0.1:${apiServer.address().port}`,
      GUANGYA_TOKEN: '',
      GUANGYA_RECYCLE_CLEAR_DEADLINE_MS: '1000',
      GUANGYA_RECYCLE_CLEAR_POLL_MS: '10',
    });
    const base = `http://127.0.0.1:${instance.port}`;
    await requestJson(base, '/api/auth/device/poll', {
      method: 'POST',
      body: { device_code: 'device-code' },
    });

    const first = await requestJson(base, '/api/recycle/clear', { method: 'POST', body: {} });
    assert.equal(first.status, 'pending');
    assert.equal(first.taskId, 'clear-task-one');

    await requestJson(base, '/api/files');
    assert.equal(refreshCalls, 1);

    const second = await requestJson(base, '/api/recycle/clear', { method: 'POST', body: {} });
    assert.equal(second.status, 'pending');
    assert.equal(second.taskId, 'clear-task-one');
    assert.equal(clearPosts, 1);
    assert.ok(checkedTasks.length > 1);
    assert.deepEqual(new Set(checkedTasks), new Set(['clear-task-one']));
  } finally {
    await stopTestServer(instance);
    await Promise.all([
      new Promise((resolve) => accountServer.close(resolve)),
      new Promise((resolve) => apiServer.close(resolve)),
    ]);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Web route requires explicit force_retry before replacing an unknown clear submission', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-recycle-force-'));
  let clearPosts = 0;
  const apiServer = http.createServer(async (request, response) => {
    await readJson(request);
    if (request.url === '/userres/v1/file/clear_recycle_bin') {
      clearPosts += 1;
      if (clearPosts === 1) {
        request.socket.destroy();
        return;
      }
      return sendJson(response, { code: 0, data: {} });
    }
    return sendJson(response, { code: 404, msg: 'not found' }, 404);
  });

  let instance;
  try {
    apiServer.listen(0, '127.0.0.1');
    await once(apiServer, 'listening');
    instance = await startTestServer(root, {
      GUANGYA_API_BASE: `http://127.0.0.1:${apiServer.address().port}`,
      GUANGYA_TOKEN: 'opaque-account-token',
      GUANGYA_REQUEST_TIMEOUT_MS: '5000',
      GUANGYA_RECYCLE_CLEAR_DEADLINE_MS: '1000',
    });
    const base = `http://127.0.0.1:${instance.port}`;
    const first = await requestJson(base, '/api/recycle/clear', { method: 'POST', body: {} });
    assert.equal(first.status, 'unknown');
    assert.equal(first.force_retry_required, true);

    const guarded = await requestJson(base, '/api/recycle/clear', { method: 'POST', body: {} });
    assert.equal(guarded.status, 'unknown');
    assert.equal(clearPosts, 1);

    const forced = await requestJson(base, '/api/recycle/clear', {
      method: 'POST', body: { force_retry: true },
    });
    assert.equal(forced.status, 'completed');
    assert.equal(clearPosts, 2);
  } finally {
    await stopTestServer(instance);
    await new Promise((resolve) => apiServer.close(resolve));
    await fsp.rm(root, { recursive: true, force: true });
  }
});
