import assert from 'node:assert/strict';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { once } from 'node:events';
import { createWebDavHandler, WebDavError } from './webdav.mjs';
import { startTestServer, stopTestServer } from './test-helpers.mjs';

function inMemoryBackend() {
  const entries = new Map();
  let nextId = 1;
  const add = (parentId, name, isDirectory, content = Buffer.alloc(0)) => {
    const id = String(nextId++);
    const entry = { id, parentId, name, isDirectory, content: Buffer.from(content), modifiedAt: Date.now() };
    entries.set(id, entry);
    return entry;
  };
  const raw = (entry) => ({
    fileId: entry.id,
    fileName: entry.name,
    resType: entry.isDirectory ? 2 : 1,
    fileSize: entry.content.length,
    updatedAt: entry.modifiedAt,
  });
  const removeTree = (id) => {
    for (const child of [...entries.values()].filter((entry) => entry.parentId === id)) removeTree(child.id);
    entries.delete(id);
  };
  return {
    entries,
    async listChildren(parentId) {
      return [...entries.values()].filter((entry) => entry.parentId === parentId).map(raw);
    },
    async createDirectory({ parentId, name }) {
      return raw(add(parentId, name, true));
    },
    async deleteEntry({ entry }) {
      removeTree(entry.id);
    },
    async moveEntry({ entry, parentId, name, existing }) {
      Object.assign(entries.get(entry.id), { parentId, name, modifiedAt: Date.now() });
      if (existing) removeTree(existing.id);
    },
    async copyEntry({ entry, parentId, name, existing }) {
      const source = entries.get(entry.id);
      add(parentId, name, source.isDirectory, source.content);
      if (existing) removeTree(existing.id);
    },
    async putFile({ request, parentId, name, existing }) {
      const chunks = [];
      for await (const chunk of request) chunks.push(chunk);
      const created = add(parentId, name, false, Buffer.concat(chunks));
      if (existing) removeTree(existing.id);
      return { id: created.id };
    },
    async readFile({ response, entry, headOnly }) {
      const record = entries.get(entry.id);
      response.writeHead(200, {
        'content-type': 'application/octet-stream',
        'content-length': String(record.content.length),
      });
      response.end(headOnly ? undefined : record.content);
    },
  };
}

async function startWebDavServer(t, backend) {
  const handler = createWebDavHandler({ prefix: '/dav', ...backend });
  const server = http.createServer(async (request, response) => {
    try {
      await handler(request, response, new URL(request.url, 'http://127.0.0.1'));
    } catch (error) {
      response.writeHead(error.statusCode || 500, error.headers || {});
      response.end(error.message);
    }
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  t.after(() => server.close());
  return `http://127.0.0.1:${server.address().port}`;
}

test('WebDAV 协议层支持目录与文件完整 CRUD', async (t) => {
  const backend = inMemoryBackend();
  const handler = createWebDavHandler({ prefix: '/dav', ...backend });
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    try {
      await handler(request, response, url);
    } catch (error) {
      const status = error instanceof WebDavError ? error.statusCode : 500;
      response.writeHead(status, error.headers || {});
      response.end(error.message);
    }
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  t.after(() => server.close());
  const base = `http://127.0.0.1:${server.address().port}`;

  const options = await fetch(`${base}/dav/`, { method: 'OPTIONS' });
  assert.equal(options.status, 204);
  assert.match(options.headers.get('allow'), /PROPFIND/);
  assert.equal(options.headers.get('dav'), '1, 2');

  const root = await fetch(`${base}/dav/`, { method: 'PROPFIND', headers: { depth: '1' } });
  assert.equal(root.status, 207);
  assert.match(await root.text(), /光鸭云盘/);

  const collection = await fetch(`${base}/dav/文档`, { method: 'MKCOL' });
  assert.equal(collection.status, 201);

  const created = await fetch(`${base}/dav/%E6%96%87%E6%A1%A3/note.txt`, {
    method: 'PUT',
    body: 'hello webdav',
  });
  assert.equal(created.status, 201);

  const browserRoot = await fetch(`${base}/dav/`);
  assert.equal(browserRoot.status, 200);
  assert.match(browserRoot.headers.get('content-type'), /text\/html/);
  assert.match(await browserRoot.text(), /%E6%96%87%E6%A1%A3\//);

  const browserDirectory = await fetch(`${base}/dav/%E6%96%87%E6%A1%A3/`);
  assert.equal(browserDirectory.status, 200);
  assert.match(await browserDirectory.text(), /note\.txt/);

  const rootHead = await fetch(`${base}/dav/`, { method: 'HEAD' });
  assert.equal(rootHead.status, 200);
  assert.equal(await rootHead.text(), '');

  const read = await fetch(`${base}/dav/%E6%96%87%E6%A1%A3/note.txt`);
  assert.equal(read.status, 200);
  assert.equal(await read.text(), 'hello webdav');

  const moved = await fetch(`${base}/dav/%E6%96%87%E6%A1%A3/note.txt`, {
    method: 'MOVE',
    headers: { destination: `${base}/dav/moved.txt` },
  });
  assert.equal(moved.status, 201);

  const copied = await fetch(`${base}/dav/moved.txt`, {
    method: 'COPY',
    headers: { destination: `${base}/dav/copied.txt` },
  });
  assert.equal(copied.status, 201);
  assert.equal(await (await fetch(`${base}/dav/copied.txt`)).text(), 'hello webdav');

  const removed = await fetch(`${base}/dav/moved.txt`, { method: 'DELETE' });
  assert.equal(removed.status, 204);
  assert.equal((await fetch(`${base}/dav/moved.txt`)).status, 404);
});

test('WebDAV MOVE 遵守 Overwrite: F', async (t) => {
  const backend = inMemoryBackend();
  backend.entries.set('1', { id: '1', parentId: '', name: 'a.txt', isDirectory: false, content: Buffer.from('a'), modifiedAt: Date.now() });
  backend.entries.set('2', { id: '2', parentId: '', name: 'b.txt', isDirectory: false, content: Buffer.from('b'), modifiedAt: Date.now() });
  const handler = createWebDavHandler({ prefix: '/dav', ...backend });
  const server = http.createServer(async (request, response) => {
    try {
      await handler(request, response, new URL(request.url, 'http://127.0.0.1'));
    } catch (error) {
      response.writeHead(error.statusCode || 500, error.headers || {});
      response.end(error.message);
    }
  });
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  t.after(() => server.close());
  const base = `http://127.0.0.1:${server.address().port}`;
  const response = await fetch(`${base}/dav/a.txt`, {
    method: 'MOVE',
    headers: { destination: `${base}/dav/b.txt`, overwrite: 'F' },
  });
  assert.equal(response.status, 412);
});

test('WebDAV 同源 COPY 不会删除源文件，覆盖失败也保留旧目标', async (t) => {
  const backend = inMemoryBackend();
  backend.entries.set('1', { id: '1', parentId: '', name: 'a.txt', isDirectory: false, content: Buffer.from('source'), modifiedAt: Date.now() });
  backend.entries.set('2', { id: '2', parentId: '', name: 'b.txt', isDirectory: false, content: Buffer.from('target'), modifiedAt: Date.now() });
  backend.copyEntry = async () => { throw new Error('模拟云端复制失败'); };
  let deletes = 0;
  const originalDelete = backend.deleteEntry;
  backend.deleteEntry = async (...args) => { deletes += 1; return originalDelete(...args); };
  const base = await startWebDavServer(t, backend);

  const selfCopy = await fetch(`${base}/dav/a.txt`, {
    method: 'COPY',
    headers: { destination: `${base}/dav/a.txt` },
  });
  assert.equal(selfCopy.status, 204);
  assert.equal(backend.entries.get('1').content.toString(), 'source');

  const failedOverwrite = await fetch(`${base}/dav/a.txt`, {
    method: 'COPY',
    headers: { destination: `${base}/dav/b.txt` },
  });
  assert.equal(failedOverwrite.status, 500);
  assert.equal(deletes, 0, '协议层不应在复制成功前删除旧目标');
  assert.equal(backend.entries.get('2').content.toString(), 'target');
});

test('WebDAV 写锁会阻止无令牌变更并支持续期和解锁', async (t) => {
  const backend = inMemoryBackend();
  backend.entries.set('100', { id: '100', parentId: '', name: 'locked.txt', isDirectory: false, content: Buffer.from('old'), modifiedAt: Date.now() });
  const base = await startWebDavServer(t, backend);
  const locked = await fetch(`${base}/dav/locked.txt`, {
    method: 'LOCK',
    headers: { timeout: 'Second-120' },
    body: '<D:lockinfo xmlns:D="DAV:"/>',
  });
  assert.equal(locked.status, 200, await locked.clone().text());
  const tokenHeader = locked.headers.get('lock-token');
  assert.match(tokenHeader || '', /^<opaquelocktoken:/);
  const token = tokenHeader.slice(1, -1);

  assert.equal((await fetch(`${base}/dav/locked.txt`, { method: 'PUT', body: 'blocked' })).status, 423);
  const refreshed = await fetch(`${base}/dav/locked.txt`, {
    method: 'LOCK',
    headers: { if: `(<${token}>)`, timeout: 'Second-180' },
  });
  assert.equal(refreshed.status, 200);
  assert.equal(refreshed.headers.get('lock-token'), `<${token}>`);
  const changed = await fetch(`${base}/dav/locked.txt`, {
    method: 'PUT',
    headers: { if: `(<${token}>)` },
    body: 'changed',
  });
  assert.equal(changed.status, 204);
  assert.equal(await (await fetch(`${base}/dav/locked.txt`)).text(), 'changed');
  assert.equal((await fetch(`${base}/dav/locked.txt`, {
    method: 'UNLOCK',
    headers: { 'lock-token': `<${token}>` },
  })).status, 204);
});

test('WebDAV 深度锁回报真实 Depth 且拒绝覆盖已有子锁', async (t) => {
  const backend = inMemoryBackend();
  backend.entries.set('dir', { id: 'dir', parentId: '', name: 'dir', isDirectory: true, content: Buffer.alloc(0), modifiedAt: Date.now() });
  backend.entries.set('child', { id: 'child', parentId: 'dir', name: 'child.txt', isDirectory: false, content: Buffer.from('child'), modifiedAt: Date.now() });
  const base = await startWebDavServer(t, backend);

  const childLock = await fetch(`${base}/dav/dir/child.txt`, {
    method: 'LOCK',
    headers: { depth: '0' },
    body: '<D:lockinfo xmlns:D="DAV:"/>',
  });
  assert.equal(childLock.status, 200, await childLock.clone().text());

  const overlapping = await fetch(`${base}/dav/dir`, {
    method: 'LOCK',
    headers: { depth: 'infinity' },
    body: '<D:lockinfo xmlns:D="DAV:"/>',
  });
  assert.equal(overlapping.status, 423);

  const shallow = await fetch(`${base}/dav/dir`, {
    method: 'LOCK',
    headers: { depth: '0' },
    body: '<D:lockinfo xmlns:D="DAV:"/>',
  });
  assert.equal(shallow.status, 200, await shallow.clone().text());
  assert.match(await shallow.text(), /<D:depth>0<\/D:depth>/);
});

test('WebDAV 删除或覆盖目录时保护其中的后代锁', async (t) => {
  const backend = inMemoryBackend();
  backend.entries.set('source', { id: 'source', parentId: '', name: 'source.txt', isDirectory: false, content: Buffer.from('source'), modifiedAt: Date.now() });
  backend.entries.set('target', { id: 'target', parentId: '', name: 'target', isDirectory: true, content: Buffer.alloc(0), modifiedAt: Date.now() });
  backend.entries.set('target-child', { id: 'target-child', parentId: 'target', name: 'child.txt', isDirectory: false, content: Buffer.from('target child'), modifiedAt: Date.now() });
  const base = await startWebDavServer(t, backend);

  const childLock = await fetch(`${base}/dav/target/child.txt`, {
    method: 'LOCK',
    headers: { depth: '0' },
    body: '<D:lockinfo xmlns:D="DAV:"/>',
  });
  assert.equal(childLock.status, 200, await childLock.clone().text());

  assert.equal((await fetch(`${base}/dav/target`, { method: 'DELETE' })).status, 423);
  const overwrite = await fetch(`${base}/dav/source.txt`, {
    method: 'MOVE',
    headers: { destination: `${base}/dav/target` },
  });
  assert.equal(overwrite.status, 423);
  assert.equal(backend.entries.has('source'), true);
  assert.equal(backend.entries.has('target-child'), true);
});

test('WebDAV PROPPATCH 明确拒绝未实现的属性写入', async (t) => {
  const backend = inMemoryBackend();
  backend.entries.set('1', { id: '1', parentId: '', name: 'a.txt', isDirectory: false, content: Buffer.from('a'), modifiedAt: Date.now() });
  const base = await startWebDavServer(t, backend);
  const response = await fetch(`${base}/dav/a.txt`, {
    method: 'PROPPATCH',
    headers: { 'content-type': 'application/xml' },
    body: '<D:propertyupdate xmlns:D="DAV:"/>',
  });
  assert.equal(response.status, 403);
});

test('Docker WebDAV 与管理端口和管理员凭据隔离', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-webdav-server-'));
  const adminPassword = 'administrator password';
  const webdavPassword = 'correct horse battery staple';
  const instance = await startTestServer(root, {
    GUANGYA_ADMIN_USERNAME: 'operator',
    GUANGYA_ADMIN_PASSWORD: adminPassword,
  });
  t.after(async () => {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  });
  const base = `http://127.0.0.1:${instance.port}`;
  const davBase = `http://127.0.0.1:${instance.webdavPort}/dav/`;
  const authorization = `Basic ${Buffer.from(`operator:${adminPassword}`).toString('base64')}`;
  assert.equal((await fetch(`${base}/dav/`, { method: 'OPTIONS', headers: { authorization } })).status, 404);
  assert.equal((await fetch(davBase, { method: 'OPTIONS' })).status, 503);

  const saved = await fetch(`${base}/api/mount/credentials`, {
    method: 'POST',
    headers: { authorization, 'content-type': 'application/json' },
    body: JSON.stringify({ username: 'storage-user', password: webdavPassword }),
  });
  assert.equal(saved.status, 200, await saved.clone().text());
  assert.equal((await fetch(davBase, { method: 'OPTIONS', headers: { authorization } })).status, 401);

  const webdavAuthorization = `Basic ${Buffer.from(`storage-user:${webdavPassword}`).toString('base64')}`;
  const options = await fetch(davBase, { method: 'OPTIONS', headers: { authorization: webdavAuthorization } });
  assert.equal(options.status, 204);
  assert.match(options.headers.get('allow'), /MOVE/);
  const mount = await (await fetch(`${base}/api/mount`, { headers: { authorization } })).json();
  assert.equal(mount.endpoint, davBase);
  assert.equal(mount.username, 'storage-user');
  assert.equal(mount.password, '');
});
