import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { fileURLToPath } from 'node:url';
import { signHdhiveRequest } from './auto-share.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));

async function freePort() {
  const server = http.createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

async function waitUntil(check, timeout = 8_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error('等待备份任务状态超时');
}

test('备份任务扫描已有文件并只监控所选类型', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-sync-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  const stagedFolder = path.join(root, 'staged-folder');
  await Promise.all([
    fsp.mkdir(watchRoot, { recursive: true }),
    fsp.mkdir(archiveRoot, { recursive: true }),
    fsp.mkdir(path.join(stagedFolder, 'season 1'), { recursive: true }),
  ]);
  await Promise.all([
    fsp.writeFile(path.join(watchRoot, 'existing.jpg'), 'image'),
    fsp.writeFile(path.join(watchRoot, 'ignored.pdf'), 'document'),
    fsp.writeFile(path.join(stagedFolder, 'season 1', 'episode-01.png'), 'video-cover-1'),
    fsp.writeFile(path.join(stagedFolder, 'season 1', 'episode-02.png'), 'video-cover-2'),
    fsp.writeFile(path.join(stagedFolder, 'season 1', 'notes.bin'), 'ignored'),
  ]);

  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: {
      ...process.env,
      PORT: String(port),
      DATA_DIR: dataDir,
      GUANGYA_WATCH_ROOT: watchRoot,
      GUANGYA_ARCHIVE_ROOT: archiveRoot,
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });

  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });

  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const response = await fetch(`http://127.0.0.1:${port}/api/mappings`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        local_path: watchRoot,
        remote_path: '',
        scan_existing: true,
        sync_types: ['jpg', 'png'],
      }),
    });
    assert.equal(response.status, 200, await response.text());

    const initial = await fetch(`http://127.0.0.1:${port}/api/state`).then((value) => value.json());
    assert.equal(initial.pending, 1);
    assert.deepEqual(initial.mappings[0].sync_types, ['jpg', 'png']);

    await fsp.writeFile(path.join(watchRoot, 'new.png'), 'image-2');
    const afterImage = await waitUntil(async () => {
      const state = await fetch(`http://127.0.0.1:${port}/api/state`).then((value) => value.json());
      return state.pending === 2 ? state : null;
    });
    assert.equal(afterImage.pending, 2);

    await fsp.rename(stagedFolder, path.join(watchRoot, 'dropped-folder'));
    const afterFolderDrop = await waitUntil(async () => {
      const state = await fetch(`http://127.0.0.1:${port}/api/state`).then((value) => value.json());
      return state.pending === 4 ? state : null;
    });
    assert.equal(afterFolderDrop.pending, 4);

    await fsp.writeFile(path.join(watchRoot, 'new.txt'), 'document-2');
    await new Promise((resolve) => setTimeout(resolve, 1_800));
    const afterDocument = await fetch(`http://127.0.0.1:${port}/api/state`).then((value) => value.json());
    assert.equal(afterDocument.pending, 4);

    const mappingId = afterDocument.mappings[0].id;
    const updateResponse = await fetch(`http://127.0.0.1:${port}/api/mappings/${mappingId}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ sync_types: ['pdf', 'txt'] }),
    });
    assert.equal(updateResponse.status, 200, await updateResponse.text());
    const afterUpdate = await fetch(`http://127.0.0.1:${port}/api/state`).then((value) => value.json());
    assert.deepEqual(afterUpdate.mappings[0].sync_types, ['pdf', 'txt']);
    assert.equal(afterUpdate.pending, 2);

    const pollingResponse = await fetch(`http://127.0.0.1:${port}/api/mappings/${mappingId}`, {
      method: 'PATCH',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ monitor_mode: 'polling' }),
    });
    assert.equal(pollingResponse.status, 200, await pollingResponse.text());
    await fsp.writeFile(path.join(watchRoot, 'polled.pdf'), 'document-3');
    const afterPolling = await waitUntil(async () => {
      const state = await fetch(`http://127.0.0.1:${port}/api/state`).then((value) => value.json());
      return state.pending === 3 ? state : null;
    }, 12_000);
    assert.equal(afterPolling.mappings[0].monitor_mode, 'polling');
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('重建同一备份任务会复用已确认上传和分享绑定', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-sync-history-reuse-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  const filePath = path.join(watchRoot, 'existing.jpg');
  await Promise.all([
    fsp.mkdir(watchRoot, { recursive: true }),
    fsp.mkdir(archiveRoot, { recursive: true }),
    fsp.mkdir(dataDir, { recursive: true }),
  ]);
  await fsp.writeFile(filePath, 'already uploaded');

  async function startServer(port) {
    const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
      cwd: path.resolve(here, '..'),
      env: {
        ...process.env,
        PORT: String(port),
        DATA_DIR: dataDir,
        GUANGYA_WATCH_ROOT: watchRoot,
        GUANGYA_ARCHIVE_ROOT: archiveRoot,
        GUANGYA_TOKEN: '',
      },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let output = '';
    child.stdout.on('data', (chunk) => { output += chunk; });
    child.stderr.on('data', (chunk) => { output += chunk; });
    await waitUntil(() => output.includes('Guangya Web listening'));
    return child;
  }

  async function stopServer(child) {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
  }

  let child;
  try {
    child = await startServer(await freePort());
    await stopServer(child);
    child = null;

    const oldMappingId = 'mapping-old';
    const currentMappingId = 'mapping-current';
    const stat = await fsp.stat(filePath);
    const database = new DatabaseSync(path.join(dataDir, 'state.sqlite3'));
    database.prepare(`
      INSERT INTO uploaded_files
        (mapping_id, file_path, size, modified_ms, task_id, remote_file_id, status, item_json,
         remote_parent_id, remote_dir, relative_path, uploaded_at)
      VALUES (?, ?, ?, ?, ?, ?, 'cloud_confirmed', NULL, ?, '', ?, ?)
    `).run(
      oldMappingId,
      path.resolve(filePath),
      stat.size,
      String(stat.mtimeMs),
      'task-old',
      'file-old',
      'parent-1',
      'existing.jpg',
      Math.floor(Date.now() / 1000),
    );
    database.prepare(`
      INSERT INTO auto_share_targets
        (mapping_id, target_key, target_type, remote_target_id, title, share_id, share_url, updated_at)
      VALUES (?, 'existing.jpg', 'file', 'file-old', 'existing.jpg', 'share-old', 'https://www.guangyapan.com/s/share-old', ?)
    `).run(oldMappingId, Math.floor(Date.now() / 1000));
    database.close();
    await fsp.writeFile(path.join(dataDir, 'config.json'), JSON.stringify({
      mappings: [{
        id: currentMappingId,
        local_path: watchRoot,
        remote_path: '',
        remote_parent_id: 'parent-1',
        enabled: true,
        source_policy: 'keep',
        scan_existing: true,
        sync_types: ['jpg'],
        monitor_mode: 'native',
        auto_share: true,
      }],
      saved_shares: [],
    }, null, 2));

    const port = await freePort();
    child = await startServer(port);
    const restored = await fetch(`http://127.0.0.1:${port}/api/state`).then((value) => value.json());
    assert.equal(restored.pending, 0);

    const verified = new DatabaseSync(path.join(dataDir, 'state.sqlite3'));
    assert.equal(verified.prepare("SELECT COUNT(*) AS count FROM uploaded_files WHERE mapping_id = ? AND status = 'cloud_confirmed'").get(currentMappingId).count, 1);
    assert.equal(verified.prepare('SELECT share_id FROM auto_share_targets WHERE mapping_id = ? AND target_key = ?').get(currentMappingId, 'existing.jpg').share_id, 'share-old');

    const removed = await fetch(`http://127.0.0.1:${port}/api/mappings/${currentMappingId}`, { method: 'DELETE' });
    assert.equal(removed.status, 200, await removed.text());
    assert.equal(verified.prepare("SELECT COUNT(*) AS count FROM uploaded_files WHERE mapping_id = ? AND status = 'cloud_confirmed'").get(currentMappingId).count, 1);
    verified.close();
  } finally {
    if (child) await stopServer(child);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Docker Web 会话保存到 SQLite 并在重启后恢复', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-sync-session-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot, { recursive: true }), fsp.mkdir(archiveRoot, { recursive: true })]);

  async function startServer(port) {
    const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
      cwd: path.resolve(here, '..'),
      env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, GUANGYA_TOKEN: '' },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let output = '';
    child.stdout.on('data', (chunk) => { output += chunk; });
    child.stderr.on('data', (chunk) => { output += chunk; });
    await waitUntil(() => output.includes('Guangya Web listening'));
    return child;
  }

  async function stopServer(child) {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
  }

  let child;
  try {
    const firstPort = await freePort();
    child = await startServer(firstPort);
    const authResponse = await fetch(`http://127.0.0.1:${firstPort}/api/auth`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ token: 'persisted-test-token' }),
    });
    assert.equal(authResponse.status, 200, await authResponse.text());
    const configResponse = await fetch(`http://127.0.0.1:${firstPort}/api/hdhive/config`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ base_url: 'https://hdhive.example.test', secret: 'local-only-secret' }),
    });
    const configRaw = await configResponse.text();
    assert.equal(configResponse.status, 200, configRaw);
    const configured = JSON.parse(configRaw);
    assert.equal(configured.configured, true);
    assert.equal('secret' in configured, false);
    const instanceId = configured.instance_id;
    await stopServer(child);
    child = null;

    const secondPort = await freePort();
    child = await startServer(secondPort);
    const restored = await fetch(`http://127.0.0.1:${secondPort}/api/state`).then((value) => value.json());
    assert.equal(restored.logged_in, true);
    assert.equal(restored.hdhive.configured, true);
    assert.equal(restored.hdhive.base_url, 'https://hdhive.example.test');
    assert.equal(restored.hdhive.instance_id, instanceId);
    assert.equal('secret' in restored.hdhive, false);
    await fsp.access(path.join(dataDir, 'state.sqlite3'));
  } finally {
    if (child) await stopServer(child);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Web 文件选择器可以浏览配置根目录内的全部可访问文件', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-server-files-test-'));
  const watchRoot = path.join(root, 'watch');
  const otherRoot = path.join(root, 'other');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([
    fsp.mkdir(path.join(watchRoot, 'videos'), { recursive: true }),
    fsp.mkdir(otherRoot, { recursive: true }),
    fsp.mkdir(archiveRoot, { recursive: true }),
  ]);
  await Promise.all([
    fsp.writeFile(path.join(watchRoot, 'cover.jpg'), 'image'),
    fsp.writeFile(path.join(watchRoot, 'videos', 'demo.mp4'), 'video'),
    fsp.writeFile(path.join(otherRoot, 'outside-watch.txt'), 'server file'),
  ]);
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, GUANGYA_FILE_ROOTS: root, GUANGYA_TOKEN: 'test-token' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const rootResponse = await fetch(`http://127.0.0.1:${port}/api/server-files`).then((value) => value.json());
    assert.equal(rootResponse.display_path, await fsp.realpath(root));
    assert.equal(rootResponse.items.some((item) => item.name === 'watch' && item.type === 'directory'), true);
    assert.equal(rootResponse.items.some((item) => item.name === 'other' && item.type === 'directory'), true);
    assert.equal(rootResponse.items.some((item) => item.name === 'data'), false);

    const childResponse = await fetch(`http://127.0.0.1:${port}/api/server-files?path=${encodeURIComponent(otherRoot)}`).then((value) => value.json());
    assert.equal(childResponse.items[0].name, 'outside-watch.txt');

    const protectedResponse = await fetch(`http://127.0.0.1:${port}/api/server-files?path=${encodeURIComponent(dataDir)}`);
    assert.equal(protectedResponse.status, 400);

    const escapeResponse = await fetch(`http://127.0.0.1:${port}/api/server-files?path=${encodeURIComponent(path.dirname(root))}`);
    assert.equal(escapeResponse.status, 400);
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Web 扫码登录保存刷新令牌并在重启后自动续期', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-device-login-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot, { recursive: true }), fsp.mkdir(archiveRoot, { recursive: true })]);

  const tokenRequests = [];
  let refreshCount = 0;
  const accountServer = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const body = chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
    response.writeHead(200, { 'content-type': 'application/json' });
    if (request.url === '/v1/auth/device/code') {
      response.end(JSON.stringify({ data: { device_code: 'device-1', user_code: 'ABCD', verification_uri_complete: 'https://example.test/authorize', expires_in: 120, interval: 1 } }));
      return;
    }
    if (request.url === '/v1/auth/token') {
      tokenRequests.push(body);
      if (body.grant_type === 'refresh_token') {
        refreshCount += 1;
        response.end(JSON.stringify({ access_token: `refreshed-access-token-${refreshCount}`, refresh_token: `refresh-token-${refreshCount + 1}` }));
      }
      else response.end(JSON.stringify({ access_token: 'device-access-token', refresh_token: 'refresh-token-1' }));
      return;
    }
    response.statusCode = 404;
    response.end(JSON.stringify({ error: 'not found' }));
  });
  accountServer.listen(0, '127.0.0.1');
  await once(accountServer, 'listening');
  const accountPort = accountServer.address().port;

  const apiServer = http.createServer((request, response) => {
    response.setHeader('content-type', 'application/json');
    if (request.url === '/userres/v1/file/get_file_list') {
      if (request.headers.authorization === 'Bearer refreshed-access-token-1') {
        response.end(JSON.stringify({ code: 117, msg: 'token expired' }));
      } else {
        response.end(JSON.stringify({ code: 0, data: { list: [], total: 0 } }));
      }
      return;
    }
    response.statusCode = 404;
    response.end(JSON.stringify({ code: 404, msg: 'not found' }));
  });
  apiServer.listen(0, '127.0.0.1');
  await once(apiServer, 'listening');
  const apiPort = apiServer.address().port;

  async function startServer(port) {
    const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
      cwd: path.resolve(here, '..'),
      env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, GUANGYA_ACCOUNT_BASE: `http://127.0.0.1:${accountPort}`, GUANGYA_API_BASE: `http://127.0.0.1:${apiPort}`, GUANGYA_TOKEN: '' },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let output = '';
    child.stdout.on('data', (chunk) => { output += chunk; });
    child.stderr.on('data', (chunk) => { output += chunk; });
    await waitUntil(() => output.includes('Guangya Web listening'));
    return child;
  }

  async function stopServer(child) {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
  }

  let child;
  try {
    const firstPort = await freePort();
    child = await startServer(firstPort);
    const deviceLogin = await fetch(`http://127.0.0.1:${firstPort}/api/auth/device/start`, { method: 'POST' }).then((value) => value.json());
    assert.equal(deviceLogin.device_code, 'device-1');
    assert.equal(deviceLogin.verification_uri_complete, 'https://example.test/authorize');

    const pollResponse = await fetch(`http://127.0.0.1:${firstPort}/api/auth/device/poll`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ device_code: deviceLogin.device_code }),
    });
    const pollPayload = await pollResponse.json();
    assert.equal(pollResponse.status, 200, JSON.stringify(pollPayload));
    assert.deepEqual(pollPayload, { authenticated: true });
    assert.equal((await fetch(`http://127.0.0.1:${firstPort}/api/state`).then((value) => value.json())).logged_in, true);
    await stopServer(child);
    child = null;

    const secondPort = await freePort();
    child = await startServer(secondPort);
    await waitUntil(() => tokenRequests.some((request) => request.grant_type === 'refresh_token'));
    assert.equal((await fetch(`http://127.0.0.1:${secondPort}/api/state`).then((value) => value.json())).logged_in, true);
    assert.equal(tokenRequests.some((request) => request.refresh_token === 'refresh-token-1'), true);
    const filesResponse = await fetch(`http://127.0.0.1:${secondPort}/api/files`);
    assert.equal(filesResponse.status, 200, await filesResponse.text());
    assert.equal(refreshCount, 2);
  } finally {
    if (child) await stopServer(child);
    await Promise.all([
      new Promise((resolve) => accountServer.close(resolve)),
      new Promise((resolve) => apiServer.close(resolve)),
    ]);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('浏览器文件落盘后立即返回并由后台队列完成云端上传', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-browser-upload-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot, { recursive: true }), fsp.mkdir(archiveRoot, { recursive: true })]);

  let taskRequested = false;
  let taskRequestCount = 0;
  const apiServer = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const url = new URL(request.url, 'http://127.0.0.1');
    response.setHeader('content-type', 'application/json');
    if (url.pathname === '/userres/v1/file/create_dir') {
      response.end(JSON.stringify({ code: 0, data: { fileId: 'folder-1' } }));
      return;
    }
    if (url.pathname === '/userres/v1/get_res_center_token') {
      response.end(JSON.stringify({ code: 156, data: { taskId: 'task-1' } }));
      return;
    }
    if (url.pathname === '/userres/v1/file/get_info_by_task_id') {
      taskRequested = true;
      taskRequestCount += 1;
      if (taskRequestCount <= 2) response.end(JSON.stringify({ code: 999, msg: '文件上传中' }));
      else response.end(JSON.stringify({ code: 0, data: { fileId: 'remote-1' } }));
      return;
    }
    response.statusCode = 404;
    response.end(JSON.stringify({ code: 404, msg: 'not found' }));
  });
  apiServer.listen(0, '127.0.0.1');
  await once(apiServer, 'listening');
  const apiPort = apiServer.address().port;

  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, GUANGYA_API_BASE: `http://127.0.0.1:${apiPort}`, GUANGYA_TOKEN: 'test-token', GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS: '1000', GUANGYA_CLOUD_CONFIRM_POLL_MS: '10' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });

  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const uploadRequest = fetch(`http://127.0.0.1:${port}/api/upload?fileName=demo.txt&relativePath=folder%2Fdemo.txt&lastModified=1234`, {
      method: 'POST',
      headers: { 'content-type': 'text/plain' },
      body: Buffer.from('browser-file'),
    });
    const uploadResponse = await Promise.race([
      uploadRequest,
      new Promise((_, reject) => setTimeout(() => reject(new Error('上传接口仍在等待云端任务')), 1_000)),
    ]);
    const uploadPayload = await uploadResponse.json();
    assert.equal(uploadResponse.status, 202, JSON.stringify(uploadPayload));
    assert.deepEqual(uploadPayload, { queued: 1, skipped: 0, fileName: 'demo.txt' });
    await waitUntil(() => taskRequested).catch((error) => { throw new Error(`${error.message}\n${output}`); });

    await waitUntil(async () => {
      const current = await fetch(`http://127.0.0.1:${port}/api/state`).then((value) => value.json());
      return current.pending === 0 && current.active_uploads === 0;
    });
    assert.equal(taskRequestCount, 3);
    assert.doesNotMatch(output, /上传失败/);
    assert.deepEqual(await fsp.readdir(path.join(dataDir, 'manual-uploads')), []);

    const duplicateResponse = await fetch(`http://127.0.0.1:${port}/api/upload?fileName=demo.txt&relativePath=folder%2Fdemo.txt&lastModified=1234`, {
      method: 'POST',
      headers: { 'content-type': 'text/plain' },
      body: Buffer.from('browser-file'),
    });
    assert.equal(duplicateResponse.status, 200);
    assert.equal((await duplicateResponse.json()).skipped, 1);
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await new Promise((resolve) => apiServer.close(resolve));
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('同一电视剧多季复用顶层文件夹分享并按静默窗口聚合事件', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-auto-share-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot, { recursive: true }), fsp.mkdir(archiveRoot, { recursive: true })]);

  const folders = new Map();
  let shareCount = 0;
  const sharePayloads = [];
  const shareRecords = [];
  const apiServer = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const input = chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
    response.setHeader('content-type', 'application/json');
    if (request.url === '/userres/v1/file/create_dir') {
      const key = `${input.parentId || ''}/${input.dirName}`;
      const fileId = folders.get(key) || `folder-${folders.size + 1}`;
      folders.set(key, fileId);
      response.end(JSON.stringify({ code: 0, data: { fileId } }));
      return;
    }
    if (request.url === '/userres/v1/get_res_center_token') {
      response.end(JSON.stringify({ code: 156, data: { taskId: `task-${input.name}` } }));
      return;
    }
    if (request.url === '/userres/v1/file/get_info_by_task_id') {
      response.end(JSON.stringify({ code: 0, data: { fileId: `remote-${input.taskId}` } }));
      return;
    }
    if (request.url === '/userres/v1/get_share_list') {
      response.end(JSON.stringify({ code: 0, data: { total: shareRecords.length, list: shareRecords.map((entry, index) => ({ id: index + 1, shareStatus: 1, title: entry.title, shareId: entry.shareId, shareUrl: entry.shareUrl })) } }));
      return;
    }
    if (request.url === '/userres/v1/get_share_access_token') {
      const entry = shareRecords.find((record) => record.shareId === input.shareId);
      response.end(JSON.stringify(entry ? { code: 0, data: { accessToken: `access:${entry.shareId}` } } : { code: 404, msg: 'share not found' }));
      return;
    }
    if (request.url === '/userres/v1/get_share_page_files_list') {
      const entry = shareRecords.find((record) => `access:${record.shareId}` === input.accessToken);
      response.end(JSON.stringify(entry ? { code: 0, data: { total: entry.fileIds.length, list: entry.fileIds.map((fileId) => ({ fileId })) } } : { code: 404, msg: 'share not found' }));
      return;
    }
    if (request.url === '/userres/v1/delete_share') {
      for (const id of [...input.ids].map(Number).sort((left, right) => right - left)) shareRecords.splice(id - 1, 1);
      response.end(JSON.stringify({ code: 0, data: {} }));
      return;
    }
    if (request.url === '/userres/v1/share_file') {
      shareCount += 1;
      sharePayloads.push(input);
      const manual = input.title === '手动投稿电视剧';
      const data = manual
        ? { shareId: '1927007413038006365', shareUrl: 'https://www.guangyapan.com/s/1927007413038006365_manualCode' }
        : { shareId: 'stable-tv-share', shareUrl: 'https://www.guangyapan.com/s/stable-tv-share' };
      shareRecords.push({ ...data, shareId: data.shareUrl.split('/s/')[1], title: input.title, fileIds: input.fileIds.map(String) });
      response.end(JSON.stringify({
        code: 0,
        data,
      }));
      return;
    }
    response.statusCode = 404;
    response.end(JSON.stringify({ code: 404, msg: 'not found' }));
  });
  apiServer.listen(0, '127.0.0.1');
  await once(apiServer, 'listening');

  const hdhiveEvents = [];
  const secret = '0123456789abcdef';
  const hdhiveServer = http.createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    const raw = Buffer.concat(chunks).toString('utf8');
    const timestamp = request.headers['x-guangya-timestamp'];
    const expected = signHdhiveRequest(secret, request.method, request.url, raw, timestamp);
    assert.equal(request.headers['x-guangya-signature'], expected);
    assert.equal(request.headers['x-guangya-instance-id'], 'test-instance');
    response.setHeader('content-type', 'application/json');
    if (request.method === 'POST' && request.url === '/api/integrations/guangya-sync/events') {
      const event = JSON.parse(raw);
      hdhiveEvents.push(event);
      response.statusCode = 202;
      response.end(JSON.stringify({ data: { event_id: event.event_id, status: 'accepted' } }));
      return;
    }
    if (request.method === 'GET' && request.url.startsWith('/api/integrations/guangya-sync/events/')) {
      response.end(JSON.stringify({ data: { status: 'completed', action: hdhiveEvents.length === 1 ? 'created' : 'updated' } }));
      return;
    }
    response.statusCode = 404;
    response.end(JSON.stringify({ error: 'not found' }));
  });
  hdhiveServer.listen(0, '127.0.0.1');
  await once(hdhiveServer, 'listening');

  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: {
      ...process.env,
      PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot,
      GUANGYA_API_BASE: `http://127.0.0.1:${apiServer.address().port}`, GUANGYA_TOKEN: 'test-token', GUANGYA_AUTO_SHARE_QUIET_MS: '1000',
      HDHIVE_BASE_URL: `http://127.0.0.1:${hdhiveServer.address().port}`, HDHIVE_GUANGYA_SYNC_SECRET: secret, HDHIVE_GUANGYA_SYNC_INSTANCE_ID: 'test-instance',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const mappingResponse = await fetch(`http://127.0.0.1:${port}/api/mappings`, {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ local_path: watchRoot, remote_path: '', scan_existing: true, sync_types: ['mkv'], auto_share: true }),
    });
    assert.equal(mappingResponse.status, 200, await mappingResponse.text());

    await fsp.mkdir(path.join(watchRoot, 'tvname', 'season 1'), { recursive: true });
    await Promise.all([
      fsp.writeFile(path.join(watchRoot, 'tvname', 'season 1', 's01.mkv'), 'episode-1'),
      fsp.writeFile(path.join(watchRoot, 'tvname', 'season 1', 's02.mkv'), 'episode-2'),
    ]);
    await waitUntil(() => hdhiveEvents.length === 1, 15_000).catch((error) => { throw new Error(`${error.message}\n${output}`); });
    assert.equal(shareCount, 1);
    assert.equal(sharePayloads[0].trafficLimit, '0');
    assert.equal(typeof sharePayloads[0].trafficLimit, 'string');
    assert.equal(sharePayloads[0].shareType, 0);
    assert.equal(sharePayloads[0].code, '');
    assert.equal(sharePayloads[0].autoFillCode, false);
    assert.match(sharePayloads[0].shareTemplate, /\{\{filename\}\}/);
    assert.match(sharePayloads[0].shareTemplate, /\{\{link\}\}/);
    assert.equal(hdhiveEvents[0].target_key, 'tvname');
    assert.equal(hdhiveEvents[0].target_type, 'folder');
    assert.deepEqual(hdhiveEvents[0].change_hint.added.sort(), ['tvname/season 1/s01.mkv', 'tvname/season 1/s02.mkv']);

    await fsp.mkdir(path.join(watchRoot, 'tvname', 'season 2'), { recursive: true });
    await fsp.writeFile(path.join(watchRoot, 'tvname', 'season 2', 's01.mkv'), 'episode-3');
    await waitUntil(() => hdhiveEvents.length === 2, 15_000).catch((error) => { throw new Error(`${error.message}\n${output}`); });
    assert.equal(shareCount, 1);
    assert.equal(hdhiveEvents[1].share_id, 'stable-tv-share');
    assert.equal(hdhiveEvents[1].intent, 'update');
    assert.deepEqual(hdhiveEvents[1].change_hint.added, ['tvname/season 2/s01.mkv']);
    assert.notEqual(hdhiveEvents[0].event_id, hdhiveEvents[1].event_id);

    const manualResponse = await fetch(`http://127.0.0.1:${port}/api/share`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ file_ids: ['manual-folder-1'], title: '手动投稿电视剧', target_type: 'folder' }),
    });
    const manualRaw = await manualResponse.text();
    assert.equal(manualResponse.status, 200, manualRaw);
    const manualResult = JSON.parse(manualRaw);
    assert.equal(manualResult.hdhive_status, 'accepted');
    assert.ok(manualResult.hdhive_event_id);
    await waitUntil(() => hdhiveEvents.length === 3, 10_000);
    assert.equal(shareCount, 2);
    assert.equal(hdhiveEvents[2].mapping_id, '__manual__');
    assert.equal(hdhiveEvents[2].target_key, '手动投稿电视剧');
    assert.equal(hdhiveEvents[2].target_type, 'folder');
    assert.equal(hdhiveEvents[2].remote_target_id, 'manual-folder-1');
    assert.equal(hdhiveEvents[2].share_id, '1927007413038006365_manualCode');
    assert.equal(hdhiveEvents[2].intent, 'new');

    const duplicateResponse = await fetch(`http://127.0.0.1:${port}/api/share`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ file_ids: ['manual-folder-1'], title: '手动投稿电视剧', target_type: 'folder' }),
    });
    const duplicateResult = await duplicateResponse.json();
    assert.equal(duplicateResponse.status, 200);
    assert.equal(duplicateResult.reused_existing, true);
    assert.equal(duplicateResult.share_url, 'https://www.guangyapan.com/s/1927007413038006365_manualCode');
    await waitUntil(() => hdhiveEvents.length === 4, 10_000);
    assert.equal(shareCount, 2);
    assert.equal(hdhiveEvents[3].intent, 'update');

    const sharesResult = await fetch(`http://127.0.0.1:${port}/api/shares`).then((response) => response.json());
    assert.equal(sharesResult.total, 2);
    const deleteShareResponse = await fetch(`http://127.0.0.1:${port}/api/shares/delete`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ ids: [2] }),
    });
    assert.equal(deleteShareResponse.status, 200);
    const remainingShares = await fetch(`http://127.0.0.1:${port}/api/shares`).then((response) => response.json());
    assert.equal(remainingShares.total, 1);
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await Promise.all([
      new Promise((resolve) => apiServer.close(resolve)),
      new Promise((resolve) => hdhiveServer.close(resolve)),
    ]);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Web 接收分享可读取目录并转存到指定云盘目录', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-received-share-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot, { recursive: true }), fsp.mkdir(archiveRoot, { recursive: true })]);
  const apiServer = http.createServer(async (request, response) => {
    const body = JSON.parse(await new Promise((resolve) => {
      let value = '';
      request.on('data', (chunk) => { value += chunk; });
      request.on('end', () => resolve(value || '{}'));
    }));
    response.setHeader('content-type', 'application/json');
    if (request.url === '/userres/v1/get_share_access_token') {
      assert.equal(body.shareId, '1926585463106830337_al8cmYXLP9l33ld2');
      assert.equal(body.code, 'iv5k');
      response.end(JSON.stringify({ data: { accessToken: 'received-share-token' } }));
      return;
    }
    if (request.url === '/userres/v1/get_share_page_files_list') {
      assert.equal(body.accessToken, 'received-share-token');
      response.end(JSON.stringify({ data: { total: 1, list: [{ fileId: 'folder-1', fileName: 'TV Show', resType: 2 }] } }));
      return;
    }
    if (request.url === '/userres/v1/restore_share') {
      assert.deepEqual(body, { accessToken: 'received-share-token', fileIds: ['folder-1'], parentId: 'destination-1' });
      response.end(JSON.stringify({ data: { taskId: 'restore-task-1' } }));
      return;
    }
    if (request.url === '/userres/v1/get_share_download_url') {
      assert.deepEqual(body, { fileId: 'file-1', accessToken: 'received-share-token' });
      response.end(JSON.stringify({ data: { downloadUrl: 'https://download.example.test/file-1.mkv' } }));
      return;
    }
    if (request.url === '/userres/v1/get_res_download_url') {
      assert.deepEqual(body, { fileId: 'cloud-file-1' });
      response.end(JSON.stringify({ data: { signedURL: 'https://download.example.test/cloud-file-1.mkv' } }));
      return;
    }
    if (request.url === '/scheduler/v1/create_packaging_task') {
      if (body.accessToken) {
        assert.deepEqual(body, { fileIds: ['folder-1'], accessToken: 'received-share-token' });
        response.end(JSON.stringify({ data: { taskId: 'package-task-1' } }));
      } else {
        assert.deepEqual(body, { fileIds: ['cloud-folder-1'] });
        response.end(JSON.stringify({ data: { taskId: 'cloud-package-task-1' } }));
      }
      return;
    }
    if (request.url === '/scheduler/v1/query_packaging_task') {
      if (body.accessToken) {
        assert.deepEqual(body, { taskId: 'package-task-1', accessToken: 'received-share-token' });
        response.end(JSON.stringify({ data: { signedURL: 'https://download.example.test/folder-1.zip' } }));
      } else {
        assert.deepEqual(body, { taskId: 'cloud-package-task-1' });
        response.end(JSON.stringify({ data: { signedURL: 'https://download.example.test/cloud-folder-1.zip' } }));
      }
      return;
    }
    if (request.url === '/userres/v1/get_task_status') {
      assert.equal(body.taskId, 'restore-task-1');
      response.end(JSON.stringify({ data: { status: 2, detail: { code: 0 } } }));
      return;
    }
    response.statusCode = 404;
    response.end(JSON.stringify({ msg: 'not found' }));
  });
  apiServer.listen(0, '127.0.0.1');
  await once(apiServer, 'listening');
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, GUANGYA_API_BASE: `http://127.0.0.1:${apiServer.address().port}`, GUANGYA_TOKEN: 'test-token' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const openedResponse = await fetch(`http://127.0.0.1:${port}/api/received-share/open`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ url: 'https://www.guangyapan.com/s/1926585463106830337_al8cmYXLP9l33ld2?code=iv5k#/share' }),
    });
    assert.equal(openedResponse.status, 200, await openedResponse.clone().text());
    const opened = await openedResponse.json();
    assert.equal(opened.share_id, '1926585463106830337_al8cmYXLP9l33ld2');
    assert.equal(opened.files.list[0].fileId, 'folder-1');
    const restoredResponse = await fetch(`http://127.0.0.1:${port}/api/received-share/restore`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ access_token: opened.access_token, file_ids: ['folder-1'], parent_id: 'destination-1' }),
    });
    assert.equal(restoredResponse.status, 200, await restoredResponse.text());
    const singleDownloadResponse = await fetch(`http://127.0.0.1:${port}/api/received-share/download`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ access_token: opened.access_token, file_ids: ['file-1'], packaged: false }),
    });
    assert.equal(singleDownloadResponse.status, 200, await singleDownloadResponse.clone().text());
    assert.equal((await singleDownloadResponse.json()).download_url, 'https://download.example.test/file-1.mkv');
    const packageDownloadResponse = await fetch(`http://127.0.0.1:${port}/api/received-share/download`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ access_token: opened.access_token, file_ids: ['folder-1'], packaged: true }),
    });
    assert.equal(packageDownloadResponse.status, 200, await packageDownloadResponse.clone().text());
    assert.equal((await packageDownloadResponse.json()).download_url, 'https://download.example.test/folder-1.zip');
    const cloudFileDownloadResponse = await fetch(`http://127.0.0.1:${port}/api/files/download`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ file_ids: ['cloud-file-1'], packaged: false }),
    });
    assert.equal(cloudFileDownloadResponse.status, 200, await cloudFileDownloadResponse.clone().text());
    assert.equal((await cloudFileDownloadResponse.json()).download_url, 'https://download.example.test/cloud-file-1.mkv');
    const cloudFolderDownloadResponse = await fetch(`http://127.0.0.1:${port}/api/files/download`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ file_ids: ['cloud-folder-1'], packaged: true }),
    });
    assert.equal(cloudFolderDownloadResponse.status, 200, await cloudFolderDownloadResponse.clone().text());
    assert.equal((await cloudFolderDownloadResponse.json()).download_url, 'https://download.example.test/cloud-folder-1.zip');
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await new Promise((resolve) => apiServer.close(resolve));
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Web 管理端鉴权覆盖静态页面和 API，并拒绝跨站变更请求', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-admin-auth-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot, { recursive: true }), fsp.mkdir(archiveRoot, { recursive: true })]);
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, LISTEN_HOST: '127.0.0.1', GUANGYA_ADMIN_USERNAME: 'operator', GUANGYA_ADMIN_PASSWORD: 'correct horse battery staple' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  const authorization = `Basic ${Buffer.from('operator:correct horse battery staple').toString('base64')}`;
  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const staticUnauthorized = await fetch(`http://127.0.0.1:${port}/`);
    assert.equal(staticUnauthorized.status, 401);
    assert.match(staticUnauthorized.headers.get('www-authenticate') || '', /^Basic /);
    assert.equal((await fetch(`http://127.0.0.1:${port}/api/state`)).status, 401);
    assert.equal((await fetch(`http://127.0.0.1:${port}/api/state`, { headers: { authorization: `Basic ${Buffer.from('operator:wrong').toString('base64')}` } })).status, 401);
    assert.equal((await fetch(`http://127.0.0.1:${port}/api/state`, { headers: { authorization } })).status, 200);

    const crossOrigin = await fetch(`http://127.0.0.1:${port}/api/queue/pause`, { method: 'POST', headers: { authorization, origin: 'https://evil.example' } });
    assert.equal(crossOrigin.status, 403);
    const sameOrigin = await fetch(`http://127.0.0.1:${port}/api/queue/pause`, { method: 'POST', headers: { authorization, origin: `http://127.0.0.1:${port}` } });
    assert.equal(sameOrigin.status, 200, await sameOrigin.text());
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await fsp.rm(root, { recursive: true, force: true });
  }

  const unsafePort = await freePort();
  const unsafeChild = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: { ...process.env, PORT: String(unsafePort), DATA_DIR: path.join(root, 'unsafe-data'), GUANGYA_WATCH_ROOT: path.join(root, 'unsafe-watch'), GUANGYA_ARCHIVE_ROOT: path.join(root, 'unsafe-archive'), LISTEN_HOST: '0.0.0.0', GUANGYA_ADMIN_PASSWORD: '' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let unsafeOutput = '';
  unsafeChild.stderr.on('data', (chunk) => { unsafeOutput += chunk; });
  await once(unsafeChild, 'exit');
  assert.match(unsafeOutput, /只允许监听回环地址/);
});

test('Web 无密码回环模式拒绝非回环 Host header', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-loopback-host-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot), fsp.mkdir(archiveRoot)]);
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, LISTEN_HOST: '127.0.0.1', GUANGYA_ADMIN_PASSWORD: '' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  const requestWithHost = (host) => new Promise((resolve, reject) => {
    const request = http.request({ hostname: '127.0.0.1', port, path: '/api/state', headers: { host } }, (response) => {
      let body = '';
      response.on('data', (chunk) => { body += chunk; });
      response.on('end', () => resolve({ status: response.statusCode, body }));
    });
    request.on('error', reject);
    request.end();
  });
  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    assert.equal((await fetch(`http://127.0.0.1:${port}/api/state`)).status, 200);
    assert.equal((await requestWithHost(`localhost:${port}`)).status, 200);
    assert.equal((await requestWithHost(`[::1]:${port}`)).status, 200);
    const rebound = await requestWithHost(`attacker.example:${port}`);
    assert.equal(rebound.status, 403);
    assert.match(JSON.parse(rebound.body).error, /回环 Host/);
    assert.equal((await requestWithHost(`127.0.0.2:${port}`)).status, 403);
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Web 文件根目录使用真实路径并阻止符号链接越界', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-realpath-test-'));
  const realRoot = path.join(root, 'real-root');
  const aliasRoot = path.join(root, 'root-alias');
  const outsideRoot = path.join(root, 'outside');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(realRoot), fsp.mkdir(outsideRoot)]);
  await Promise.all([fsp.writeFile(path.join(realRoot, 'inside.txt'), 'inside'), fsp.writeFile(path.join(outsideRoot, 'secret.txt'), 'secret')]);
  await Promise.all([fsp.symlink(realRoot, aliasRoot), fsp.symlink(outsideRoot, path.join(realRoot, 'escape'))]);
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: realRoot, GUANGYA_ARCHIVE_ROOT: realRoot, GUANGYA_FILE_ROOTS: aliasRoot, GUANGYA_TOKEN: 'test-token' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const canonical = await fetch(`http://127.0.0.1:${port}/api/server-files?path=${encodeURIComponent(realRoot)}`).then((response) => response.json());
    assert.equal(canonical.path, await fsp.realpath(realRoot));
    assert.equal(canonical.roots[0], await fsp.realpath(aliasRoot));
    assert.equal(canonical.items.some((item) => item.name === 'inside.txt'), true);
    assert.equal(canonical.items.some((item) => item.name === 'escape'), false);
    const escaped = await fetch(`http://127.0.0.1:${port}/api/server-files?path=${encodeURIComponent(path.join(aliasRoot, 'escape'))}`);
    assert.equal(escaped.status, 400);
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Web 归档为每次冲突生成唯一名称且绝不覆盖旧文件', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-archive-collision-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot), fsp.mkdir(archiveRoot)]);
  const source = path.join(watchRoot, 'episode.txt');
  await fsp.writeFile(source, 'new upload');
  const sourceStat = await fsp.stat(source);
  const timestamp = Math.round(sourceStat.mtimeMs);
  const originalArchive = path.join(archiveRoot, 'episode.txt');
  const timestampArchive = path.join(archiveRoot, `episode-${timestamp}.txt`);
  await Promise.all([fsp.writeFile(originalArchive, 'old one'), fsp.writeFile(timestampArchive, 'old two')]);
  const apiServer = http.createServer(async (request, response) => {
    const body = JSON.parse(await new Promise((resolve) => { let value = ''; request.on('data', (chunk) => { value += chunk; }); request.on('end', () => resolve(value || '{}')); }));
    response.setHeader('content-type', 'application/json');
    if (request.url === '/userres/v1/get_res_center_token') response.end(JSON.stringify({ code: 156, data: { taskId: `task-${body.name}` } }));
    else if (request.url === '/userres/v1/file/get_info_by_task_id') response.end(JSON.stringify({ code: 0, data: { fileId: 'remote-episode' } }));
    else { response.statusCode = 404; response.end(JSON.stringify({ code: 404, msg: 'not found' })); }
  });
  apiServer.listen(0, '127.0.0.1');
  await once(apiServer, 'listening');
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, GUANGYA_FILE_ROOTS: root, GUANGYA_API_BASE: `http://127.0.0.1:${apiServer.address().port}`, GUANGYA_TOKEN: 'test-token', GUANGYA_FILE_STABILITY_MS: '200' },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const mappingResponse = await fetch(`http://127.0.0.1:${port}/api/mappings`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ local_path: watchRoot, remote_path: '', archive_path: archiveRoot, source_policy: 'archive', scan_existing: true, sync_types: ['txt'] }) });
    assert.equal(mappingResponse.status, 200, await mappingResponse.text());
    const uniqueArchive = path.join(archiveRoot, `episode-${timestamp}-2.txt`);
    await waitUntil(async () => { try { return (await fsp.readFile(uniqueArchive, 'utf8')) === 'new upload'; } catch { return false; } }, 10_000).catch((error) => { throw new Error(`${error.message}\n${output}`); });
    assert.equal(await fsp.readFile(originalArchive, 'utf8'), 'old one');
    assert.equal(await fsp.readFile(timestampArchive, 'utf8'), 'old two');
    await assert.rejects(fsp.access(source));
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await new Promise((resolve) => apiServer.close(resolve));
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Web 重启后先恢复未确认任务，再重新上传期间变化的源文件', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-pending-recovery-test-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([fsp.mkdir(watchRoot), fsp.mkdir(archiveRoot)]);
  const sourceFile = path.join(watchRoot, 'pending.txt');
  await fsp.writeFile(sourceFile, 'pending upload');
  let cloudConfirmed = false;
  let uploadTokenRequests = 0;
  let confirmRequests = 0;
  const apiServer = http.createServer(async (request, response) => {
    const body = JSON.parse(await new Promise((resolve) => { let value = ''; request.on('data', (chunk) => { value += chunk; }); request.on('end', () => resolve(value || '{}')); }));
    response.setHeader('content-type', 'application/json');
    if (request.url === '/userres/v1/get_res_center_token') {
      uploadTokenRequests += 1;
      response.end(JSON.stringify({ code: 156, data: { taskId: `pending-task-${uploadTokenRequests}` } }));
    } else if (request.url === '/userres/v1/file/get_info_by_task_id') {
      assert.match(body.taskId, /^pending-task-\d+$/);
      confirmRequests += 1;
      response.end(JSON.stringify(cloudConfirmed ? { code: 0, data: { fileId: `confirmed-${body.taskId}` } } : { code: 999, msg: '文件上传中' }));
    } else { response.statusCode = 404; response.end(JSON.stringify({ code: 404, msg: 'not found' })); }
  });
  apiServer.listen(0, '127.0.0.1');
  await once(apiServer, 'listening');

  async function startServer(port) {
    const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
      cwd: path.resolve(here, '..'),
      env: { ...process.env, PORT: String(port), DATA_DIR: dataDir, GUANGYA_WATCH_ROOT: watchRoot, GUANGYA_ARCHIVE_ROOT: archiveRoot, GUANGYA_FILE_ROOTS: root, GUANGYA_API_BASE: `http://127.0.0.1:${apiServer.address().port}`, GUANGYA_TOKEN: 'test-token', GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS: '1000', GUANGYA_CLOUD_CONFIRM_POLL_MS: '10', GUANGYA_FILE_STABILITY_MS: '200' },
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let output = '';
    child.stdout.on('data', (chunk) => { output += chunk; });
    child.stderr.on('data', (chunk) => { output += chunk; });
    await waitUntil(() => output.includes('Guangya Web listening'));
    return { child, output: () => output };
  }
  async function stopServer(child) { child.kill(); await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]); }

  let running;
  try {
    const firstPort = await freePort();
    running = await startServer(firstPort);
    const mappingResponse = await fetch(`http://127.0.0.1:${firstPort}/api/mappings`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ local_path: watchRoot, remote_path: '', source_policy: 'delete', scan_existing: true, sync_types: ['txt'] }) });
    assert.equal(mappingResponse.status, 200, await mappingResponse.text());
    await waitUntil(async () => {
      const current = await fetch(`http://127.0.0.1:${firstPort}/api/state`).then((response) => response.json());
      return uploadTokenRequests === 1 && current.pending === 1 && current.active_uploads === 0;
    }, 10_000).catch((error) => { throw new Error(`${error.message}\n${running.output()}`); });
    await fsp.access(sourceFile);
    await fsp.writeFile(sourceFile, 'pending upload changed while confirming');
    await new Promise((resolve) => setTimeout(resolve, 500));
    assert.equal(uploadTokenRequests, 1);
    await stopServer(running.child);
    running = null;
    const pendingDb = new DatabaseSync(path.join(dataDir, 'state.sqlite3'));
    assert.equal(pendingDb.prepare('SELECT status FROM uploaded_files').get().status, 'oss_complete');
    pendingDb.close();

    cloudConfirmed = true;
    const secondPort = await freePort();
    running = await startServer(secondPort);
    await waitUntil(async () => {
      const current = await fetch(`http://127.0.0.1:${secondPort}/api/state`).then((response) => response.json());
      return uploadTokenRequests === 2 && current.pending === 0 && current.active_uploads === 0;
    }, 10_000).catch((error) => { throw new Error(`${error.message}\n${running.output()}`); });
    assert.equal(uploadTokenRequests, 2);
    assert.ok(confirmRequests > 1);
    await assert.rejects(fsp.access(sourceFile));
    await stopServer(running.child);
    running = null;
    const confirmedDb = new DatabaseSync(path.join(dataDir, 'state.sqlite3'));
    const confirmed = confirmedDb.prepare('SELECT status, remote_file_id FROM uploaded_files').get();
    assert.equal(confirmed.status, 'cloud_confirmed');
    assert.equal(confirmed.remote_file_id, 'confirmed-pending-task-2');
    confirmedDb.close();
  } finally {
    if (running) await stopServer(running.child);
    await new Promise((resolve) => apiServer.close(resolve));
    await fsp.rm(root, { recursive: true, force: true });
  }
});
