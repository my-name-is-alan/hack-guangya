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
  createVirtualLibraryService,
  strmContent,
  strmFileName,
  virtualFileKind,
  virtualMediaPath,
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

test('media files produce pure-path STRM content without HTTP', () => {
  assert.equal(virtualFileKind('Movie.2026.2160p.mkv'), 'strm');
  assert.equal(virtualFileKind('album.FLAC'), 'strm');
  assert.equal(virtualFileKind('movie.nfo'), 'metadata');
  assert.equal(virtualFileKind('notes.txt'), '');
  assert.equal(strmFileName('Movie.2026.2160p.mkv'), 'Movie.2026.2160p.strm');
  const virtualPath = virtualMediaPath('/电影', ['子目录'], '电影.mkv');
  assert.equal(strmContent(virtualPath), '/电影/子目录/电影.mkv\n');
  assert.doesNotMatch(strmContent(virtualPath), /^https?:/);
});

test('sync writes pure virtual paths, obeys metadata setting, and preserves unmanaged files', async (t) => {
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
    proxyPort: 18096,
    embyUpstream: 'http://127.0.0.1:8096',
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
  assert.equal(await fsp.readFile(path.join(target, 'Movie.2026.strm'), 'utf8'), '/电影/Movie.2026.mkv\n');
  await assert.rejects(fsp.access(path.join(target, 'Movie.2026.nfo')));
  assert.deepEqual(downloads, []);

  service.upsert({ ...mapping, include_metadata: true });
  service.sync(mapping.id);
  status = await waitForSync(service, mapping.id);
  assert.equal(status.metadata_files, 2);
  assert.match(await fsp.readFile(path.join(target, 'Movie.2026.nfo'), 'utf8'), /metadata:https:\/\/download\.invalid\/nfo-1/);
  await fsp.writeFile(path.join(target, 'kept-by-user.txt'), 'keep');

  children = [{ fileId: 'poster-1', fileName: 'poster.jpg', resType: 1, fileSize: 8, utime: 13 }];
  service.sync(mapping.id);
  await waitForSync(service, mapping.id);
  await assert.rejects(fsp.access(path.join(target, 'Movie.2026.strm')));
  await assert.rejects(fsp.access(path.join(target, 'Movie.2026.nfo')));
  assert.equal(await fsp.readFile(path.join(target, 'kept-by-user.txt'), 'utf8'), 'keep');
});

test('18096-style proxy redirects matched playback and forwards everything else to Emby 8096-style upstream', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-emby-proxy-'));
  const database = new DatabaseSync(':memory:');
  const observed = [];
  const openSockets = new Set();
  const upstreamServer = http.createServer((request, response) => {
    observed.push({ method: request.method, url: request.url, token: request.headers['x-emby-token'] || '' });
    const playback = request.url.match(/^\/Items\/([^/?]+)\/PlaybackInfo/);
    if (playback) {
      const mediaPath = playback[1] === 'mapped-item' ? '/电影/Movie.2026.mkv' : '/本地/Other.mkv';
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
    proxyPort: 18096,
    embyUpstream: `http://127.0.0.1:${upstreamPort}`,
    cloud: {
      listChildren: async () => [{ fileId: 'movie-1', fileName: 'Movie.2026.mkv', resType: 1, fileSize: 100, utime: 1 }],
      getDownloadUrl: async (id) => `https://cdn.invalid/${id}?signed=1`,
    },
  });
  const target = path.join(root, 'movies');
  service.upsert({ id: 'movies', name: '电影', source_dir_id: 'cloud-movies', source_path: '/电影', local_path: target, enabled: true });
  service.sync('movies');
  await waitForSync(service, 'movies');
  const proxyServer = http.createServer(async (request, response) => {
    try { await service.handleProxy(request, response, new URL(request.url, 'http://localhost')); }
    catch (error) { response.writeHead(502); response.end(error.message); }
  });
  proxyServer.on('upgrade', (request, socket, head) => service.proxyUpgrade(request, socket, head));
  proxyServer.on('connection', (socket) => {
    openSockets.add(socket);
    socket.once('close', () => openSockets.delete(socket));
  });
  const proxyPort = await listen(proxyServer);
  t.after(async () => {
    service.close();
    database.close();
    for (const socket of openSockets) socket.destroy();
    openSockets.clear();
    proxyServer.closeAllConnections();
    upstreamServer.closeAllConnections();
    await Promise.all([
      new Promise((resolve) => proxyServer.close(resolve)),
      new Promise((resolve) => upstreamServer.close(resolve)),
    ]);
    await fsp.rm(root, { recursive: true, force: true });
  });

  const directOriginal = await fetch(`http://127.0.0.1:${upstreamPort}/Videos/mapped-item/stream.mkv`, { redirect: 'manual' });
  assert.equal(directOriginal.status, 200, 'direct Emby 8096-style access must remain untouched');

  const matched = await fetch(`http://127.0.0.1:${proxyPort}/Videos/mapped-item/stream.mkv?MediaSourceId=mapped-item-source`, {
    headers: { 'x-emby-token': 'user-token' },
    redirect: 'manual',
  });
  assert.equal(matched.status, 302);
  assert.equal(matched.headers.get('location'), 'https://cdn.invalid/movie-1?signed=1');
  assert.ok(observed.some((item) => item.url.startsWith('/Items/mapped-item/PlaybackInfo') && item.token === 'user-token'));

  const unmatched = await fetch(`http://127.0.0.1:${proxyPort}/Videos/local-item/stream.mkv`, { redirect: 'manual' });
  assert.equal(unmatched.status, 200);
  assert.equal(await unmatched.text(), 'upstream:GET:/Videos/local-item/stream.mkv');

  const ordinaryApi = await fetch(`http://127.0.0.1:${proxyPort}/System/Info`);
  assert.equal(ordinaryApi.status, 200);
  assert.equal(await ordinaryApi.text(), 'upstream:GET:/System/Info');

  const hls = await fetch(`http://127.0.0.1:${proxyPort}/Videos/mapped-item/master.m3u8`);
  assert.equal(hls.status, 200);
  assert.equal(await hls.text(), 'upstream:GET:/Videos/mapped-item/master.m3u8');

  const websocket = await websocketHandshake(proxyPort);
  assert.match(websocket, /^HTTP\/1\.1 101 Switching Protocols/);
});

test('Web API persists virtual-library mappings and Emby upstream settings', async (t) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-virtual-api-'));
  const instance = await startTestServer(root);
  t.after(async () => {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  });
  const api = `http://127.0.0.1:${instance.port}`;
  const ready = await waitUntil(async () => {
    const response = await fetch(`${api}/api/virtual-library`);
    const payload = await response.json();
    return payload.proxy_running ? payload : null;
  });
  assert.match(ready.proxy_endpoint, /^http:\/\/127\.0\.0\.1:\d+\/$/);
  assert.equal(ready.virtual_root, instance.virtualLibraryRoot);

  const target = path.join(instance.virtualLibraryRoot, 'movies');
  const createdResponse = await fetch(`${api}/api/virtual-library/mappings`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ mapping: { id: 'movies', name: '电影', source_dir_id: 'cloud-movies', source_path: '/电影', local_path: target, include_metadata: false, enabled: true } }),
  });
  assert.equal(createdResponse.status, 200);
  const created = await createdResponse.json();
  assert.equal(created.mappings[0].local_path, target);

  const settingsResponse = await fetch(`${api}/api/virtual-library/settings`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ refresh_minutes: 30, emby_upstream: 'http://127.0.0.1:18097' }),
  });
  assert.equal(settingsResponse.status, 200);
  const settings = await settingsResponse.json();
  assert.equal(settings.refresh_minutes, 30);
  assert.equal(settings.emby_upstream, 'http://127.0.0.1:18097');

  const outsideResponse = await fetch(`${api}/api/virtual-library/mappings`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ mapping: { id: 'outside', source_dir_id: 'cloud-outside', source_path: '/外部', local_path: path.join(root, 'outside') } }),
  });
  assert.equal(outsideResponse.status, 400);
});
