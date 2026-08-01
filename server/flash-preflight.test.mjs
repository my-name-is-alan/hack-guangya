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

const here = path.dirname(fileURLToPath(import.meta.url));

async function freePort() {
  const server = http.createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

async function waitUntil(check, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error('等待后台秒传预检超时');
}

async function readJson(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
}

test('普通上传槽被占用时，后续文件仍会后台预检并直接秒传', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-flash-preflight-'));
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([
    fsp.mkdir(watchRoot, { recursive: true }),
    fsp.mkdir(archiveRoot, { recursive: true }),
  ]);

  const events = [];
  let allowFirstConfirm = false;
  let allowSecondConfirm = false;
  const apiServer = http.createServer(async (request, response) => {
    const body = await readJson(request);
    response.setHeader('content-type', 'application/json');
    if (request.url === '/userres/v1/get_res_center_token') {
      const taskId = `task-${path.parse(body.name).name}`;
      events.push(`token:${taskId}`);
      response.end(JSON.stringify({ code: 156, data: { taskId } }));
      return;
    }
    if (request.url === '/userres/v1/file/get_info_by_task_id') {
      if (body.taskId === 'task-first' && !allowFirstConfirm) {
        events.push('poll:task-first');
        response.end(JSON.stringify({ code: 147, msg: '文件上传中' }));
        return;
      }
      if (body.taskId === 'task-second' && !allowSecondConfirm) {
        events.push('poll:task-second');
        response.end(JSON.stringify({ code: 147, msg: '文件上传中' }));
        return;
      }
      events.push(`confirmed:${body.taskId}`);
      response.end(JSON.stringify({ code: 0, data: { fileId: `file-${body.taskId}` } }));
      return;
    }
    response.statusCode = 404;
    response.end(JSON.stringify({ code: 404, msg: 'not found' }));
  });
  apiServer.listen(0, '127.0.0.1');
  await once(apiServer, 'listening');

  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: {
      ...process.env,
      PORT: String(port),
      DATA_DIR: dataDir,
      GUANGYA_WATCH_ROOT: watchRoot,
      GUANGYA_ARCHIVE_ROOT: archiveRoot,
      GUANGYA_API_BASE: `http://127.0.0.1:${apiServer.address().port}`,
      GUANGYA_TOKEN: 'test-token',
      GUANGYA_FILE_STABILITY_MS: '200',
      GUANGYA_CLOUD_CONFIRM_POLL_MS: '20',
      GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS: '5000',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });

  try {
    await waitUntil(() => output.includes('Guangya Web listening'));
    const transferResponse = await fetch(`http://127.0.0.1:${port}/api/settings/transfer`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ upload_concurrency: 1 }),
    });
    assert.equal(transferResponse.status, 200, await transferResponse.text());

    const mappingResponse = await fetch(`http://127.0.0.1:${port}/api/mappings`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        local_path: watchRoot,
        remote_path: '',
        scan_existing: false,
        sync_types: ['txt'],
      }),
    });
    assert.equal(mappingResponse.status, 200, await mappingResponse.text());

    await fsp.writeFile(path.join(watchRoot, 'first.txt'), 'first');
    await waitUntil(() => events.includes('poll:task-first'));
    await fsp.writeFile(path.join(watchRoot, 'second.txt'), 'second');
    await fsp.writeFile(path.join(watchRoot, 'third.txt'), 'third');
    await waitUntil(() => events.includes('token:task-third')).catch((error) => {
      throw new Error(`${error.message}\n${JSON.stringify(events)}\n${output}`);
    });

    assert.equal(events.includes('confirmed:task-first'), false);
    assert.equal(events.includes('confirmed:task-second'), false);
    assert.ok(
      events.indexOf('token:task-third') > events.indexOf('poll:task-first'),
      JSON.stringify(events),
    );
    await waitUntil(() => {
      const database = new DatabaseSync(path.join(dataDir, 'state.sqlite3'));
      const count = database.prepare('SELECT COUNT(*) AS count FROM uploaded_files').get().count;
      database.close();
      return count === 3;
    });

    allowFirstConfirm = true;
    allowSecondConfirm = true;
    await waitUntil(async () => {
      const current = await fetch(`http://127.0.0.1:${port}/api/state`).then((response) => response.json());
      return current.pending === 0 && current.active_uploads === 0;
    }, 20_000).catch((error) => {
      throw new Error(`${error.message}\n${output}`);
    });

    const database = new DatabaseSync(path.join(dataDir, 'state.sqlite3'));
    const rows = database.prepare("SELECT file_path, status FROM uploaded_files ORDER BY file_path").all();
    database.close();
    assert.equal(rows.length, 3);
    assert.ok(rows.every((row) => row.status === 'cloud_confirmed'));
  } finally {
    child.kill();
    await Promise.race([once(child, 'exit'), new Promise((resolve) => setTimeout(resolve, 2_000))]);
    await new Promise((resolve) => apiServer.close(resolve));
    await fsp.rm(root, { recursive: true, force: true });
  }
});
