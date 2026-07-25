import assert from 'node:assert/strict';
import { once } from 'node:events';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { startTestServer, stopTestServer } from './test-helpers.mjs';

function chunkedJsonRequest(port, chunks) {
  return new Promise((resolve, reject) => {
    const request = http.request({
      host: '127.0.0.1',
      port,
      path: '/api/access/unlock',
      method: 'POST',
      headers: { 'content-type': 'application/json' },
    }, (response) => {
      const body = [];
      response.on('data', (chunk) => body.push(chunk));
      response.on('end', () => resolve({
        status: response.statusCode,
        headers: response.headers,
        body: Buffer.concat(body).toString('utf8'),
      }));
    });
    request.on('error', reject);
    for (const chunk of chunks) request.write(chunk);
    request.end();
  });
}

test('未认证访问码接口拒绝 Content-Length 与 chunked 超大请求体', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-body-limit-test-'));
  let instance;
  try {
    instance = await startTestServer(root, { GUANGYA_ADMIN_PASSWORD: 'correct horse battery staple' });
    const response = await fetch(`http://127.0.0.1:${instance.port}/api/access/unlock`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: Buffer.alloc(5 * 1024, 97),
    });
    assert.equal(response.status, 413, await response.clone().text());
    assert.match((await response.json()).error, /4096/);

    const chunked = await chunkedJsonRequest(instance.port, [
      Buffer.alloc(3 * 1024, 97),
      Buffer.alloc(2 * 1024, 98),
    ]);
    assert.equal(chunked.status, 413, chunked.body);
    assert.match(chunked.body, /4096/);
  } finally {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('备份监听不会沿目录符号链接上传任务目录外文件', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-watcher-boundary-test-'));
  const outsideRoot = path.join(root, 'outside');
  await fsp.mkdir(outsideRoot, { recursive: true });
  const outsideFile = path.join(outsideRoot, 'outside.txt');
  await fsp.writeFile(outsideFile, 'must stay outside');

  let uploadRequests = 0;
  const apiServer = http.createServer(async (request, response) => {
    for await (const _chunk of request) { /* drain request */ }
    if (request.url === '/userres/v1/get_res_center_token') uploadRequests += 1;
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ code: 156, data: { taskId: 'unexpected-task' } }));
  });
  apiServer.listen(0, '127.0.0.1');
  await once(apiServer, 'listening');

  let instance;
  try {
    instance = await startTestServer(root, {
      GUANGYA_API_BASE: `http://127.0.0.1:${apiServer.address().port}`,
      GUANGYA_TOKEN: 'test-token',
    });
    const mapping = await fetch(`http://127.0.0.1:${instance.port}/api/mappings`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        local_path: instance.watchRoot,
        remote_path: '',
        scan_existing: false,
        sync_types: ['txt'],
      }),
    });
    assert.equal(mapping.status, 200, await mapping.clone().text());

    const link = path.join(instance.watchRoot, 'escape');
    await fsp.symlink(outsideRoot, link, process.platform === 'win32' ? 'junction' : 'dir');
    await new Promise((resolve) => setTimeout(resolve, 2_000));

    assert.equal(uploadRequests, 0);
    assert.equal(await fsp.readFile(outsideFile, 'utf8'), 'must stay outside');
  } finally {
    await stopTestServer(instance);
    await new Promise((resolve) => apiServer.close(resolve));
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('HDHive 地址拒绝 URL 注入并支持部署主机允许列表', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-hdhive-url-test-'));
  let instance;
  try {
    instance = await startTestServer(root, { HDHIVE_ALLOWED_HOSTS: 'hdhive.internal:8080' });
    const base = `http://127.0.0.1:${instance.port}`;
    for (const invalid of [
      'http://hdhive.internal:8080/base?next=metadata',
      'http://hdhive.internal:8080/base#fragment',
      'http://user:secret@hdhive.internal:8080',
      'http://169.254.169.254/latest/meta-data',
    ]) {
      const response = await fetch(`${base}/api/hdhive/config`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ base_url: invalid }),
      });
      assert.equal(response.status, 400, `${invalid}: ${await response.text()}`);
    }

    const accepted = await fetch(`${base}/api/hdhive/config`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ base_url: 'http://hdhive.internal:8080/base/', secret: 'test-secret' }),
    });
    assert.equal(accepted.status, 200, await accepted.clone().text());
    assert.equal((await accepted.json()).base_url, 'http://hdhive.internal:8080/base');
  } finally {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  }
});
