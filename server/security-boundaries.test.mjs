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

function streamedUpload(port, { chunks, intervalMs }) {
  return new Promise((resolve, reject) => {
    let stopped = false;
    const totalBytes = chunks.reduce((sum, chunk) => sum + Buffer.byteLength(chunk), 0);
    const request = http.request({
      host: '127.0.0.1',
      port,
      path: '/api/upload?fileName=slow.txt&relativePath=slow.txt&lastModified=1234',
      method: 'POST',
      headers: { 'content-length': String(totalBytes) },
    }, (response) => {
      const body = [];
      response.on('data', (chunk) => body.push(chunk));
      response.on('end', () => {
        stopped = true;
        resolve({
          status: response.statusCode,
          body: Buffer.concat(body).toString('utf8'),
        });
      });
    });
    request.on('error', (error) => {
      stopped = true;
      reject(error);
    });
    let index = 0;
    const writeNext = () => {
      if (stopped) return;
      if (index >= chunks.length) {
        request.end();
        return;
      }
      request.write(chunks[index]);
      index += 1;
      setTimeout(writeNext, intervalMs);
    };
    writeNext();
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

test('浏览器上传按接收空闲时间防护而不限制请求总时长', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-slow-upload-test-'));
  let instance;
  try {
    instance = await startTestServer(root, {
      GUANGYA_TOKEN: 'test-token',
      GUANGYA_REQUEST_TIMEOUT_MS: '200',
    });
    const startedAt = Date.now();
    const response = await streamedUpload(instance.port, {
      chunks: ['a', 'b', 'c', 'd', 'e'],
      intervalMs: 80,
    });
    assert.ok(Date.now() - startedAt > 200, '测试上传总时长应超过配置的空闲超时');
    assert.equal(response.status, 202, response.body);
    assert.deepEqual(JSON.parse(response.body), { queued: 1, skipped: 0, fileName: 'slow.txt' });
    await assert.rejects(streamedUpload(instance.port, {
      chunks: ['a', 'b'],
      intervalMs: 350,
    }), /socket hang up|ECONNRESET/i, '上传长时间没有任何数据时仍应关闭连接');
  } finally {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('代理协议仅在显式 trusted proxy 配置下参与同源校验', async () => {
  const untrustedRoot = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-untrusted-proxy-test-'));
  const trustedRoot = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-trusted-proxy-test-'));
  let untrusted;
  let trusted;
  try {
    untrusted = await startTestServer(untrustedRoot);
    const untrustedResponse = await fetch(`http://127.0.0.1:${untrusted.port}/api/queue/pause`, {
      method: 'POST',
      headers: {
        origin: `https://127.0.0.1:${untrusted.port}`,
        'x-forwarded-proto': 'https',
      },
    });
    assert.equal(untrustedResponse.status, 403);
    await stopTestServer(untrusted);
    untrusted = null;

    trusted = await startTestServer(trustedRoot, { GUANGYA_TRUST_PROXY: '1' });
    const trustedResponse = await fetch(`http://127.0.0.1:${trusted.port}/api/queue/pause`, {
      method: 'POST',
      headers: {
        origin: `https://127.0.0.1:${trusted.port}`,
        forwarded: 'for=192.0.2.44;proto=https',
      },
    });
    assert.equal(trustedResponse.status, 200, await trustedResponse.clone().text());
  } finally {
    await stopTestServer(untrusted);
    await stopTestServer(trusted);
    await Promise.all([
      fsp.rm(untrustedRoot, { recursive: true, force: true }),
      fsp.rm(trustedRoot, { recursive: true, force: true }),
    ]);
  }
});

test('非法 Host 不会让 HTTP 处理器越过异常边界崩溃', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-invalid-host-test-'));
  let instance;
  try {
    instance = await startTestServer(root);
    const malformed = await new Promise((resolve, reject) => {
      const request = http.request({
        host: '127.0.0.1',
        port: instance.port,
        path: '/api/state',
        headers: { host: '[' },
      }, (response) => {
        const chunks = [];
        response.on('data', (chunk) => chunks.push(chunk));
        response.on('end', () => resolve({ status: response.statusCode, body: Buffer.concat(chunks).toString('utf8') }));
      });
      request.on('error', reject);
      request.end();
    });
    assert.equal(malformed.status, 400, malformed.body);
    assert.equal((await fetch(`http://127.0.0.1:${instance.port}/api/state`)).status, 200, '非法 Host 后服务仍应可用');
  } finally {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Unix 数据目录及敏感状态文件使用私有权限', { skip: process.platform === 'win32' }, async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-private-state-test-'));
  const dataDir = path.join(root, 'data');
  await fsp.mkdir(dataDir, { recursive: true, mode: 0o755 });
  await fsp.writeFile(path.join(dataDir, 'config.json'), JSON.stringify({ mappings: [], saved_shares: [] }), { mode: 0o644 });
  await fsp.chmod(dataDir, 0o755);
  await fsp.chmod(path.join(dataDir, 'config.json'), 0o644);
  let instance;
  try {
    instance = await startTestServer(root);
    const mode = async (target) => (await fsp.stat(target)).mode & 0o777;
    assert.equal(await mode(dataDir), 0o700);
    assert.equal(await mode(path.join(dataDir, 'config.json')), 0o600);
    assert.equal(await mode(path.join(dataDir, 'state.sqlite3')), 0o600);
    assert.equal(await mode(path.join(dataDir, 'manual-uploads')), 0o700);
    for (const suffix of ['-wal', '-shm']) {
      const target = path.join(dataDir, `state.sqlite3${suffix}`);
      try {
        assert.equal(await mode(target), 0o600);
      } catch (error) {
        if (error.code !== 'ENOENT') throw error;
      }
    }
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
