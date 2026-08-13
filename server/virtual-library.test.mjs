import assert from 'node:assert/strict';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import {
  createVirtualLibraryService,
  normalizeStrmBaseUrl,
  strmContent,
  strmFileName,
  strmRequestFileId,
  strmSignature,
  strmUrlFor,
  verifyStrmSignature,
  virtualFileKind,
} from './virtual-library.mjs';
import { startTestServer, stopTestServer, waitUntil } from './test-helpers.mjs';

async function waitForSync(service, id) {
  const deadline = Date.now() + 5_000;
  while (Date.now() < deadline) {
    const status = service.info().statuses[id];
    if (status && !status.running) {
      if (status.error) throw new Error(status.error);
      return status;
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error('等待虚拟库同步超时');
}

function signSecret(database) {
  return database.prepare('SELECT sign_secret FROM virtual_library_settings WHERE id = 1').get().sign_secret;
}

function fakeStrmExchange(method = 'GET') {
  const state = { statusCode: 0, headers: {}, body: '' };
  return {
    request: { method, headers: {} },
    response: {
      writeHead(statusCode, headers = {}) { state.statusCode = statusCode; state.headers = headers; },
      end(payload) { state.body = String(payload || ''); },
    },
    state,
  };
}

test('STRM 直链地址、签名与请求路径解析', () => {
  assert.equal(normalizeStrmBaseUrl(''), '');
  assert.equal(normalizeStrmBaseUrl('http://192.168.1.10:8080/'), 'http://192.168.1.10:8080');
  assert.equal(normalizeStrmBaseUrl('https://nas.example.com/guangya/'), 'https://nas.example.com/guangya');
  assert.throws(() => normalizeStrmBaseUrl('ftp://192.168.1.10'), /HTTP\(S\)/);
  assert.throws(() => normalizeStrmBaseUrl('http://user:pass@192.168.1.10'), /HTTP\(S\)/);
  assert.throws(() => normalizeStrmBaseUrl('http://192.168.1.10?x=1'), /HTTP\(S\)/);

  const secret = 'test-secret';
  const url = strmUrlFor('http://192.168.1.10:8080', secret, 'file-1');
  assert.equal(url, `http://192.168.1.10:8080/strm/file-1?sign=${strmSignature(secret, 'file-1')}`);
  assert.equal(strmContent(url), `${url}\n`);
  assert.throws(() => strmUrlFor('', secret, 'file-1'), /STRM 直链地址/);

  assert.equal(verifyStrmSignature(secret, 'file-1', strmSignature(secret, 'file-1')), true);
  assert.equal(verifyStrmSignature(secret, 'file-1', strmSignature(secret, 'file-2')), false);
  assert.equal(verifyStrmSignature(secret, 'file-1', ''), false);
  assert.equal(verifyStrmSignature('', 'file-1', strmSignature('', 'file-1')), false);

  assert.equal(strmRequestFileId('/strm/file-1'), 'file-1');
  assert.equal(strmRequestFileId('/strm/file%3A1'), 'file:1');
  assert.equal(strmRequestFileId('/strm/'), '');
  assert.equal(strmRequestFileId('/strm/a/b'), '');
  assert.equal(strmRequestFileId('/strm/%2e%2e'), '');

  assert.equal(virtualFileKind('Movie.2026.2160p.mkv'), 'strm');
  assert.equal(virtualFileKind('album.FLAC'), 'strm');
  assert.equal(virtualFileKind('movie.nfo'), 'metadata');
  assert.equal(virtualFileKind('notes.txt'), '');
  assert.equal(strmFileName('Movie.2026.2160p.mkv'), 'Movie.2026.2160p.strm');
});

test('同步生成签名直链 STRM，遵守元数据开关并保留非托管文件', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-virtual-library-'));
  const database = new DatabaseSync(':memory:');
  const downloads = [];
  let children = [
    { fileId: 'movie-1', fileName: 'Movie.2026.mkv', resType: 1, fileSize: 10_000, utime: 11 },
    { fileId: 'nfo-1', fileName: 'Movie.2026.nfo', resType: 1, fileSize: 12, utime: 12 },
    { fileId: 'poster-1', fileName: 'poster.jpg', resType: 1, fileSize: 8, utime: 13 },
    { fileId: 'notes-1', fileName: 'notes.txt', resType: 1, fileSize: 5, utime: 14 },
  ];
  const service = createVirtualLibraryService({
    database,
    root,
    strmBaseUrl: 'http://192.168.1.10:8080',
    cloud: {
      listChildren: async () => children,
      getDownloadUrl: async (id) => `https://download.invalid/${id}`,
    },
    fetchImpl: async (url) => {
      downloads.push(String(url));
      return new Response(Buffer.from(`metadata:${url}`));
    },
  });
  t.after(async () => {
    service.close();
    database.close();
    await fsp.rm(root, { recursive: true, force: true });
  });

  const target = path.join(root, 'movies');
  const mapping = {
    id: 'movies',
    name: '电影',
    source_dir_id: 'cloud-movies',
    source_path: '/电影',
    local_path: target,
    include_metadata: false,
    enabled: true,
  };
  service.upsert(mapping);
  assert.throws(() => service.upsert({ ...mapping, id: 'nested-library', local_path: path.join(target, 'nested') }), /不能与其他配置相同或互相包含/);
  service.sync(mapping.id);
  let status = await waitForSync(service, mapping.id);
  assert.equal(status.strm_files, 1);
  assert.equal(status.metadata_files, 0);
  const secret = signSecret(database);
  const expectedUrl = `http://192.168.1.10:8080/strm/movie-1?sign=${strmSignature(secret, 'movie-1')}`;
  assert.equal(await fsp.readFile(path.join(target, 'Movie.2026.strm'), 'utf8'), `${expectedUrl}\n`);
  await assert.rejects(fsp.access(path.join(target, 'Movie.2026.nfo')));
  assert.deepEqual(downloads, []);

  service.upsert({ ...mapping, include_metadata: true });
  service.sync(mapping.id);
  status = await waitForSync(service, mapping.id);
  assert.equal(status.metadata_files, 2);
  assert.match(await fsp.readFile(path.join(target, 'Movie.2026.nfo'), 'utf8'), /metadata:https:\/\/download\.invalid\/nfo-1/);
  await fsp.writeFile(path.join(target, 'kept-by-user.txt'), 'keep');

  // 修改直链地址后，下一次同步会重写全部 STRM 内容。
  service.updateSettings({ strm_base_url: 'https://nas.example.com' });
  service.sync(mapping.id);
  await waitForSync(service, mapping.id);
  assert.equal(
    await fsp.readFile(path.join(target, 'Movie.2026.strm'), 'utf8'),
    `https://nas.example.com/strm/movie-1?sign=${strmSignature(secret, 'movie-1')}\n`,
  );

  children = [{ fileId: 'poster-1', fileName: 'poster.jpg', resType: 1, fileSize: 8, utime: 13 }];
  service.sync(mapping.id);
  await waitForSync(service, mapping.id);
  await assert.rejects(fsp.access(path.join(target, 'Movie.2026.strm')));
  await assert.rejects(fsp.access(path.join(target, 'Movie.2026.nfo')));
  assert.equal(await fsp.readFile(path.join(target, 'kept-by-user.txt'), 'utf8'), 'keep');
});

test('未配置 STRM 直链地址时同步立即报错', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-virtual-nobase-'));
  const database = new DatabaseSync(':memory:');
  const service = createVirtualLibraryService({
    database,
    root,
    cloud: { listChildren: async () => [], getDownloadUrl: async () => '' },
  });
  t.after(async () => {
    service.close();
    database.close();
    await fsp.rm(root, { recursive: true, force: true });
  });
  assert.equal(service.info().strm_configured, false);
  service.upsert({ id: 'movies', name: '电影', source_dir_id: 'cloud-movies', source_path: '/电影', local_path: path.join(root, 'movies'), enabled: true });
  assert.throws(() => service.sync('movies'), /STRM 直链地址/);
});

test('/strm 端点校验签名并 302 到云盘直链', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-strm-endpoint-'));
  const database = new DatabaseSync(':memory:');
  const requestedIds = [];
  const service = createVirtualLibraryService({
    database,
    root,
    strmBaseUrl: 'http://127.0.0.1:8080',
    cloud: {
      listChildren: async () => [],
      getDownloadUrl: async (id) => { requestedIds.push(id); return `https://cdn.invalid/${id}?signed=1`; },
    },
  });
  t.after(async () => {
    service.close();
    database.close();
    await fsp.rm(root, { recursive: true, force: true });
  });
  const secret = signSecret(database);
  const goodSign = strmSignature(secret, 'movie-1');

  const ok = fakeStrmExchange();
  await service.handleStrm(ok.request, ok.response, new URL(`http://localhost/strm/movie-1?sign=${goodSign}`));
  assert.equal(ok.state.statusCode, 302);
  assert.equal(ok.state.headers.location, 'https://cdn.invalid/movie-1?signed=1');
  assert.equal(ok.state.headers['cache-control'], 'no-store');
  assert.deepEqual(requestedIds, ['movie-1']);

  const head = fakeStrmExchange('HEAD');
  await service.handleStrm(head.request, head.response, new URL(`http://localhost/strm/movie-1?sign=${goodSign}`));
  assert.equal(head.state.statusCode, 302);

  const badSign = fakeStrmExchange();
  await service.handleStrm(badSign.request, badSign.response, new URL('http://localhost/strm/movie-1?sign=deadbeef'));
  assert.equal(badSign.state.statusCode, 403);

  const missingSign = fakeStrmExchange();
  await service.handleStrm(missingSign.request, missingSign.response, new URL('http://localhost/strm/movie-1'));
  assert.equal(missingSign.state.statusCode, 403);

  const badPath = fakeStrmExchange();
  await service.handleStrm(badPath.request, badPath.response, new URL(`http://localhost/strm/a/b?sign=${goodSign}`));
  assert.equal(badPath.state.statusCode, 404);

  const badMethod = fakeStrmExchange('POST');
  await service.handleStrm(badMethod.request, badMethod.response, new URL(`http://localhost/strm/movie-1?sign=${goodSign}`));
  assert.equal(badMethod.state.statusCode, 405);

  const upstreamFailure = fakeStrmExchange();
  const failingDatabase = new DatabaseSync(':memory:');
  const failing = createVirtualLibraryService({
    database: failingDatabase,
    root: path.join(root, 'failing'),
    cloud: { listChildren: async () => [], getDownloadUrl: async () => { throw new Error('云端超时'); } },
  });
  t.after(() => { failing.close(); failingDatabase.close(); });
  await failing.handleStrm(
    upstreamFailure.request,
    upstreamFailure.response,
    new URL(`http://localhost/strm/movie-1?sign=${strmSignature(signSecret(failingDatabase), 'movie-1')}`),
  );
  assert.equal(upstreamFailure.state.statusCode, 502);
  assert.match(upstreamFailure.state.body, /云端超时/);
});

test('Web API 持久化虚拟库配置与 STRM 直链设置，/strm 免管理登录', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-virtual-api-'));
  const adminPassword = 'administrator password';
  const instance = await startTestServer(root, {
    GUANGYA_ADMIN_USERNAME: 'operator',
    GUANGYA_ADMIN_PASSWORD: adminPassword,
  });
  t.after(async () => {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  });
  const api = `http://127.0.0.1:${instance.port}`;
  const authorization = `Basic ${Buffer.from(`operator:${adminPassword}`).toString('base64')}`;

  assert.equal((await fetch(`${api}/api/virtual-library`)).status, 401);
  const ready = await waitUntil(async () => {
    const response = await fetch(`${api}/api/virtual-library`, { headers: { authorization } });
    if (response.status !== 200) return null;
    return response.json();
  });
  assert.equal(ready.strm_configured, false);
  assert.equal(ready.virtual_root, instance.virtualLibraryRoot);

  const target = path.join(instance.virtualLibraryRoot, 'movies');
  const createdResponse = await fetch(`${api}/api/virtual-library/mappings`, {
    method: 'POST',
    headers: { authorization, 'content-type': 'application/json' },
    body: JSON.stringify({ mapping: { id: 'movies', name: '电影', source_dir_id: 'cloud-movies', source_path: '/电影', local_path: target, include_metadata: false, enabled: true } }),
  });
  assert.equal(createdResponse.status, 200);
  const created = await createdResponse.json();
  assert.equal(created.mappings[0].local_path, target);

  const settingsResponse = await fetch(`${api}/api/virtual-library/settings`, {
    method: 'POST',
    headers: { authorization, 'content-type': 'application/json' },
    body: JSON.stringify({ refresh_minutes: 30, strm_base_url: 'http://192.168.1.10:8080/' }),
  });
  assert.equal(settingsResponse.status, 200);
  const settings = await settingsResponse.json();
  assert.equal(settings.refresh_minutes, 30);
  assert.equal(settings.strm_base_url, 'http://192.168.1.10:8080');
  assert.equal(settings.strm_configured, true);
  assert.equal(settings.strm_endpoint, 'http://192.168.1.10:8080/strm/');

  const invalidSettings = await fetch(`${api}/api/virtual-library/settings`, {
    method: 'POST',
    headers: { authorization, 'content-type': 'application/json' },
    body: JSON.stringify({ strm_base_url: 'ftp://192.168.1.10' }),
  });
  assert.equal(invalidSettings.status, 400);

  // /strm 不要求管理登录：非法签名返回 403 而不是 401。
  const unauthenticated = await fetch(`${api}/strm/some-file?sign=deadbeef`, { redirect: 'manual' });
  assert.equal(unauthenticated.status, 403);
  assert.equal((await fetch(`${api}/strm/`, { redirect: 'manual' })).status, 404);

  const outsideResponse = await fetch(`${api}/api/virtual-library/mappings`, {
    method: 'POST',
    headers: { authorization, 'content-type': 'application/json' },
    body: JSON.stringify({ mapping: { id: 'outside', source_dir_id: 'cloud-outside', source_path: '/外部', local_path: path.join(root, 'outside') } }),
  });
  assert.equal(outsideResponse.status, 400);
});
