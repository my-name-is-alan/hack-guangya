import assert from 'node:assert/strict';
import crypto from 'node:crypto';
import { once } from 'node:events';
import fsp from 'node:fs/promises';
import http from 'node:http';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import {
  cloudPathsOverlap,
  createVirtualLibraryService,
  isBrowserUserAgent,
  normalizeEmbyPath,
  normalizeEmbyUpstream,
  normalizeStrmBaseUrl,
  strmContent,
  strmFileName,
  strmRequestFileId,
  strmSignature,
  strmUrlCredentials,
  strmUrlFor,
  verifyStrmSignature,
  virtualFileKind,
  pruneEmptyDirectories,
} from './virtual-library.mjs';
import { startTestServer, stopTestServer, waitUntil } from './test-helpers.mjs';

async function listen(server) {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return server.address().port;
}

async function websocketHandshake(port) {
  return new Promise((resolve, reject) => {
    const socket = net.connect({ host: '127.0.0.1', port }, () => {
      socket.write('GET /embywebsocket?api_key=test HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n');
    });
    let data = '';
    socket.setEncoding('utf8');
    socket.on('data', (chunk) => {
      data += chunk;
      if (data.includes('\r\n\r\n')) { socket.destroy(); resolve(data); }
    });
    socket.once('error', reject);
    socket.setTimeout(2_000, () => { socket.destroy(); reject(new Error('WebSocket handshake timeout')); });
  });
}

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

test('Emby 兼容网关：命中签名直链的播放请求 302 到 CDN，其余转发到 Emby', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-emby-gateway-'));
  const database = new DatabaseSync(':memory:');
  const observed = [];
  const openSockets = new Set();
  let strmUrlForMapped = '';
  const upstreamServer = http.createServer((request, response) => {
    observed.push({ method: request.method, url: request.url, token: request.headers['x-emby-token'] || '' });
    const playback = request.url.match(/^\/Items\/([^/?]+)\/PlaybackInfo/);
    if (playback) {
      const mediaPath = playback[1] === 'mapped-item' ? strmUrlForMapped : '/local/Other.mkv';
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ MediaSources: [{ Id: `${playback[1]}-source`, Path: mediaPath }] }));
      return;
    }
    response.writeHead(200, { 'content-type': 'text/plain', 'x-upstream': 'emby' });
    response.end(`upstream:${request.method}:${request.url}`);
  });
  upstreamServer.on('upgrade', (request, socket) => {
    const accept = crypto.createHash('sha1').update(`${request.headers['sec-websocket-key']}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
  });
  upstreamServer.on('connection', (socket) => {
    openSockets.add(socket);
    socket.once('close', () => openSockets.delete(socket));
  });
  const upstreamPort = await listen(upstreamServer);
  const service = createVirtualLibraryService({
    database,
    root,
    strmBaseUrl: 'http://192.168.1.10:18096',
    embyUpstream: `http://127.0.0.1:${upstreamPort}`,
    cloud: {
      listChildren: async () => [],
      getDownloadUrl: async (id) => `https://cdn.invalid/${id}?signed=1`,
    },
    fetchImpl: (target, options) => {
      if (String(target).startsWith('https://cdn.invalid/')) {
        return Promise.resolve(new Response('cdn-bytes', {
          status: 206,
          headers: { 'content-type': 'video/x-matroska', 'content-range': 'bytes 0-8/9', 'accept-ranges': 'bytes' },
        }));
      }
      return fetch(target, options);
    },
  });
  const secret = database.prepare('SELECT sign_secret FROM virtual_library_settings WHERE id = 1').get().sign_secret;
  strmUrlForMapped = `http://192.168.1.10:18096/strm/movie-1?sign=${strmSignature(secret, 'movie-1')}`;
  const gatewayServer = http.createServer(async (request, response) => {
    try { await service.handleGateway(request, response, new URL(request.url, 'http://localhost')); }
    catch (error) { response.writeHead(502); response.end(error.message); }
  });
  gatewayServer.on('upgrade', (request, socket, head) => service.proxyUpgrade(request, socket, head));
  gatewayServer.on('connection', (socket) => {
    openSockets.add(socket);
    socket.once('close', () => openSockets.delete(socket));
  });
  const gatewayPort = await listen(gatewayServer);
  t.after(async () => {
    service.close();
    database.close();
    for (const socket of openSockets) socket.destroy();
    openSockets.clear();
    gatewayServer.closeAllConnections();
    upstreamServer.closeAllConnections();
    await Promise.all([
      new Promise((resolve) => gatewayServer.close(resolve)),
      new Promise((resolve) => upstreamServer.close(resolve)),
    ]);
    await fsp.rm(root, { recursive: true, force: true });
  });

  const matched = await fetch(`http://127.0.0.1:${gatewayPort}/Videos/mapped-item/stream.mkv?MediaSourceId=mapped-item-source`, {
    headers: { 'x-emby-token': 'user-token' },
    redirect: 'manual',
  });
  assert.equal(matched.status, 302);
  assert.equal(matched.headers.get('location'), 'https://cdn.invalid/movie-1?signed=1');
  assert.ok(observed.some((item) => item.url.startsWith('/Items/mapped-item/PlaybackInfo') && item.token === 'user-token'));

  // 浏览器 UA：网关中转 CDN 数据并注入 CORS 头，不再 302。
  const browser = await fetch(`http://127.0.0.1:${gatewayPort}/Videos/mapped-item/stream.mkv`, {
    headers: {
      'x-emby-token': 'user-token',
      'user-agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0 Safari/537.36',
      range: 'bytes=0-8',
    },
    redirect: 'manual',
  });
  assert.equal(browser.status, 206);
  assert.equal(browser.headers.get('access-control-allow-origin'), '*');
  assert.equal(browser.headers.get('content-range'), 'bytes 0-8/9');
  assert.equal(await browser.text(), 'cdn-bytes');

  assert.equal(isBrowserUserAgent('Mozilla/5.0 (Windows NT 10.0) Chrome/126.0 Safari/537.36'), true);
  assert.equal(isBrowserUserAgent('Fileball/1.3.20'), false);
  assert.equal(isBrowserUserAgent('ExoPlayerLib/2.19.1'), false);
  assert.equal(isBrowserUserAgent(''), false);

  const unmatched = await fetch(`http://127.0.0.1:${gatewayPort}/Videos/local-item/stream.mkv`, { redirect: 'manual' });
  assert.equal(unmatched.status, 200);
  assert.equal(await unmatched.text(), 'upstream:GET:/Videos/local-item/stream.mkv');

  const ordinaryApi = await fetch(`http://127.0.0.1:${gatewayPort}/System/Info`);
  assert.equal(ordinaryApi.status, 200);
  assert.equal(await ordinaryApi.text(), 'upstream:GET:/System/Info');

  const hls = await fetch(`http://127.0.0.1:${gatewayPort}/Videos/mapped-item/master.m3u8`);
  assert.equal(hls.status, 200);
  assert.equal(await hls.text(), 'upstream:GET:/Videos/mapped-item/master.m3u8');

  const websocket = await websocketHandshake(gatewayPort);
  assert.match(websocket, /^HTTP\/1\.1 101 Switching Protocols/);

  assert.deepEqual(strmUrlCredentials(strmUrlForMapped), { fileId: 'movie-1', sign: strmSignature(secret, 'movie-1') });
  assert.equal(strmUrlCredentials('/visual_media/movie.mkv'), null);
  assert.equal(strmUrlCredentials('http://192.168.1.10:18096/other/movie-1?sign=x'), null);
  assert.equal(normalizeEmbyUpstream(''), 'http://127.0.0.1:8096');
  assert.throws(() => normalizeEmbyUpstream('http://127.0.0.1:8096/emby'), /不要包含路径/);
});

test('云端路径重叠判定与 Emby 内路径规范化', () => {
  assert.equal(cloudPathsOverlap('/媒体', '/媒体/电影'), true);
  assert.equal(cloudPathsOverlap('/媒体/电影', '/媒体'), true);
  assert.equal(cloudPathsOverlap('/媒体', '/媒体'), true);
  assert.equal(cloudPathsOverlap('/媒体', '/媒体库'), false);
  assert.equal(cloudPathsOverlap('/', '/任意/路径'), true);
  assert.equal(cloudPathsOverlap('', '/媒体'), false);
  assert.equal(normalizeEmbyPath('/visual_media/'), '/visual_media');
  assert.equal(normalizeEmbyPath('G:\\visual_media\\'), 'G:\\visual_media');
  assert.equal(normalizeEmbyPath('  '), '');
});

test('同步收集变更清单并通知 Emby，整理联动匹配触发同步', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-virtual-notify-'));
  const database = new DatabaseSync(':memory:');
  const embyRequests = [];
  let children = [
    { fileId: 'movie-1', fileName: 'Movie.2026.mkv', resType: 1, fileSize: 10_000, utime: 11 },
  ];
  let listCalls = 0;
  const service = createVirtualLibraryService({
    database,
    root,
    strmBaseUrl: 'http://192.168.1.10:18096',
    embyUpstream: 'http://127.0.0.1:8096',
    cloud: {
      listChildren: async () => { listCalls += 1; return children; },
      getDownloadUrl: async (id) => `https://download.invalid/${id}`,
    },
    fetchImpl: async (target, options = {}) => {
      if (String(target).includes('/emby/Library/Media/Updated')) {
        embyRequests.push({ headers: options.headers, body: JSON.parse(options.body) });
        return new Response(null, { status: 204 });
      }
      return new Response(Buffer.from('metadata'));
    },
  });
  t.after(async () => {
    service.close();
    database.close();
    await fsp.rm(root, { recursive: true, force: true });
  });
  service.updateSettings({ emby_api_key: 'emby-key' });
  service.upsert({
    id: 'movies',
    name: '电影',
    source_dir_id: 'cloud-movies',
    source_path: '/媒体/电影',
    local_path: path.join(root, 'movies'),
    emby_path: '/visual_media',
    include_metadata: false,
    enabled: true,
  });

  service.sync('movies');
  let status = await waitForSync(service, 'movies');
  assert.equal(status.changed_files, 1);
  await waitUntil(() => service.info().statuses.movies.emby_notified === 1 ? true : null, 3_000);
  assert.equal(embyRequests.length, 1);
  assert.equal(embyRequests[0].headers['x-emby-token'], 'emby-key');
  assert.deepEqual(embyRequests[0].body.Updates, [
    { Path: '/visual_media/Movie.2026.strm', UpdateType: 'Created' },
  ]);

  // 无变更的同步不再通知
  service.sync('movies');
  await waitForSync(service, 'movies');
  await new Promise((resolve) => setTimeout(resolve, 100));
  assert.equal(embyRequests.length, 1);

  // 云端换内容：新增 + 删除
  children = [{ fileId: 'movie-2', fileName: 'Other.2026.mkv', resType: 1, fileSize: 9_000, utime: 12 }];
  service.sync('movies');
  await waitForSync(service, 'movies');
  await waitUntil(() => embyRequests.length === 2 ? true : null, 3_000);
  assert.deepEqual(embyRequests[1].body.Updates.map((update) => update.UpdateType).sort(), ['Created', 'Deleted']);

  // 整理联动匹配：dirId 相等或路径重叠才触发
  assert.deepEqual(service.syncForCloudTarget({ dirId: 'cloud-movies', path: '/别处' }), ['movies']);
  await waitForSync(service, 'movies');
  assert.deepEqual(service.syncForCloudTarget({ dirId: 'other', path: '/媒体/电影/2026' }), ['movies']);
  await waitForSync(service, 'movies');
  assert.deepEqual(service.syncForCloudTarget({ dirId: 'other', path: '/无关目录' }), []);

  // 正在同步时 queueSync 合并为待重跑
  const before = listCalls;
  service.sync('movies');
  assert.equal(service.queueSync('movies'), true);
  await waitUntil(() => (listCalls >= before + 2 && !service.info().statuses.movies.running) ? true : null, 5_000);
  assert.ok(listCalls >= before + 2);
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
    body: JSON.stringify({ refresh_minutes: 30, strm_base_url: 'http://192.168.1.10:8080/', emby_upstream: 'http://127.0.0.1:18097' }),
  });
  assert.equal(settingsResponse.status, 200);
  const settings = await settingsResponse.json();
  assert.equal(settings.refresh_minutes, 30);
  assert.equal(settings.strm_base_url, 'http://192.168.1.10:8080');
  assert.equal(settings.strm_configured, true);
  assert.equal(settings.strm_endpoint, 'http://192.168.1.10:8080/strm/');
  assert.equal(settings.emby_upstream, 'http://127.0.0.1:18097');
  assert.equal(settings.gateway_running, true);
  assert.match(settings.gateway_endpoint, /^http:\/\/127\.0\.0\.1:\d+\/$/);

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

test('pruneEmptyDirectories 自底向上删除空目录并保留仍有文件的目录', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-vl-prune-'));
  await fsp.mkdir(path.join(root, '电影', '阿凡达'), { recursive: true });
  await fsp.mkdir(path.join(root, '电影', '保留'), { recursive: true });
  await fsp.writeFile(path.join(root, '电影', '保留', 'Movie.strm'), 'keep\n');
  const removed = await pruneEmptyDirectories(root);
  assert.ok(removed.includes('电影/阿凡达'));
  await assert.rejects(fsp.access(path.join(root, '电影', '阿凡达')));
  assert.equal(await fsp.readFile(path.join(root, '电影', '保留', 'Movie.strm'), 'utf8'), 'keep\n');
  await fsp.rm(root, { recursive: true, force: true });
});

test('同步删除过期 STRM 后会收回变空的电影文件夹', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-virtual-empty-dir-'));
  const database = new DatabaseSync(':memory:');
  let tree = {
    'cloud-movies': [{ fileId: 'avatar-dir', fileName: '阿凡达', resType: 2 }],
    'avatar-dir': [{ fileId: 'avatar-1', fileName: '阿凡达.mkv', resType: 1, fileSize: 10_000, utime: 11 }],
  };
  const service = createVirtualLibraryService({
    database,
    root,
    strmBaseUrl: 'http://192.168.1.10:18096',
    cloud: {
      listChildren: async (parentId) => tree[parentId] || [],
      getDownloadUrl: async (id) => `https://download.invalid/${id}`,
    },
  });
  t.after(async () => {
    service.close();
    database.close();
    await fsp.rm(root, { recursive: true, force: true });
  });
  const target = path.join(root, 'movies');
  service.upsert({
    id: 'movies',
    name: '电影',
    source_dir_id: 'cloud-movies',
    source_path: '/电影',
    local_path: target,
    include_metadata: false,
    enabled: true,
  });
  service.sync('movies');
  await waitForSync(service, 'movies');
  assert.equal(await fsp.readFile(path.join(target, '阿凡达', '阿凡达.strm'), 'utf8').then((value) => value.includes('/strm/avatar-1')), true);

  tree = { 'cloud-movies': [] };
  service.sync('movies');
  await waitForSync(service, 'movies');
  await assert.rejects(fsp.access(path.join(target, '阿凡达', '阿凡达.strm')));
  await assert.rejects(fsp.access(path.join(target, '阿凡达')));
  assert.equal((await fsp.readdir(target)).includes('.guangya-virtual-library.json'), true);
});
