import assert from 'node:assert/strict';
import test from 'node:test';
import { Readable, Writable } from 'node:stream';
import {
  createWebDavFileReader,
  normalizeWebDavRedirectMode,
  webDavContentType,
  webDavRedirectAllowed,
} from './webdav-read.mjs';

function fakeResponse() {
  const chunks = [];
  const state = { statusCode: 0, headers: {}, ended: false };
  const writable = new Writable({
    write(chunk, _encoding, callback) { chunks.push(Buffer.from(chunk)); callback(); },
  });
  writable.writeHead = (statusCode, headers = {}) => { state.statusCode = statusCode; state.headers = headers; return writable; };
  const originalEnd = writable.end.bind(writable);
  writable.end = (payload) => {
    if (payload) chunks.push(Buffer.from(payload));
    state.ended = true;
    return originalEnd();
  };
  return { response: writable, state, body: () => Buffer.concat(chunks).toString() };
}

function fakeDownloadUrls(url = 'https://cdn.example.com/file?Expires=9999999999') {
  const calls = [];
  return {
    calls,
    async get(fileId, options = {}) { calls.push({ fileId, force: options.force === true }); return url; },
    invalidate() {},
  };
}

const entry = {
  id: 'file-1',
  name: '电影.mkv',
  size: 123,
  modifiedAt: Date.parse('2026-08-01T08:00:00.000Z'),
  etag: '"gy-file-1-1-123"',
};

test('重定向模式解析与 UA 自动回退判定', () => {
  assert.equal(normalizeWebDavRedirectMode(undefined), 'auto');
  assert.equal(normalizeWebDavRedirectMode(''), 'auto');
  assert.equal(normalizeWebDavRedirectMode('1'), 'auto');
  assert.equal(normalizeWebDavRedirectMode('always'), 'always');
  assert.equal(normalizeWebDavRedirectMode('0'), 'off');
  assert.equal(normalizeWebDavRedirectMode('OFF'), 'off');
  assert.throws(() => normalizeWebDavRedirectMode('sometimes'), /off、auto 或 always/);

  assert.equal(webDavRedirectAllowed('auto', 'rclone/v1.74.4'), true);
  assert.equal(webDavRedirectAllowed('auto', 'Microsoft-WebDAV-MiniRedir/10.0.22631'), false);
  assert.equal(webDavRedirectAllowed('auto', 'WebDAVFS/3.0.0 (03008000) Darwin/23.0.0'), false);
  assert.equal(webDavRedirectAllowed('auto', 'davfs2/1.6.1 neon/0.32.5'), false);
  assert.equal(webDavRedirectAllowed('always', 'Microsoft-WebDAV-MiniRedir/10.0.22631'), true);
  assert.equal(webDavRedirectAllowed('off', 'rclone/v1.74.4'), false);
});

test('HEAD 直接用目录条目元数据回应，不请求云端', async () => {
  const urls = fakeDownloadUrls();
  const reader = createWebDavFileReader({
    downloadUrls: urls,
    fetchImpl: () => { throw new Error('HEAD 不应请求云端'); },
  });
  const { response, state } = fakeResponse();
  await reader({ request: { headers: {} }, response, entry, headOnly: true });
  assert.equal(state.statusCode, 200);
  assert.equal(state.headers['content-length'], '123');
  assert.equal(state.headers['content-type'], 'video/x-matroska');
  assert.equal(state.headers['accept-ranges'], 'bytes');
  assert.equal(state.headers.etag, entry.etag);
  assert.equal(urls.calls.length, 0);
});

test('GET 默认 302 重定向到直链，数据不经过服务器', async () => {
  const urls = fakeDownloadUrls('https://cdn.example.com/signed');
  const reader = createWebDavFileReader({
    downloadUrls: urls,
    fetchImpl: () => { throw new Error('重定向模式不应中转数据'); },
  });
  const { response, state } = fakeResponse();
  await reader({ request: { headers: { 'user-agent': 'rclone/v1.74.4' } }, response, entry, headOnly: false });
  assert.equal(state.statusCode, 302);
  assert.equal(state.headers.location, 'https://cdn.example.com/signed');
  assert.equal(state.headers['cache-control'], 'no-store');
  assert.deepEqual(urls.calls, [{ fileId: 'file-1', force: false }]);
});

test('auto 模式对 Windows WebClient 自动回退为中转', async () => {
  const urls = fakeDownloadUrls();
  const reader = createWebDavFileReader({
    downloadUrls: urls,
    fetchImpl: async () => ({
      ok: true,
      status: 200,
      headers: new Headers({ 'content-length': '4', 'content-type': 'application/octet-stream' }),
      body: Readable.from([Buffer.from('data')]),
    }),
  });
  const { response, state, body } = fakeResponse();
  await reader({
    request: { headers: { 'user-agent': 'Microsoft-WebDAV-MiniRedir/10.0.22631' } },
    response,
    entry,
    headOnly: false,
  });
  assert.equal(state.statusCode, 200);
  assert.equal(body(), 'data');
  assert.equal(state.headers['content-length'], '4');
});

test('中转路径缓存直链 403 时强制刷新一次并重试', async () => {
  const urls = fakeDownloadUrls();
  let attempts = 0;
  const reader = createWebDavFileReader({
    downloadUrls: urls,
    redirectMode: 'off',
    fetchImpl: async () => {
      attempts += 1;
      if (attempts === 1) {
        return { ok: false, status: 403, headers: new Headers(), body: { cancel: async () => {} } };
      }
      return {
        ok: false,
        status: 206,
        headers: new Headers({ 'content-range': 'bytes 0-3/123', 'content-length': '4' }),
        body: Readable.from([Buffer.from('data')]),
      };
    },
  });
  const { response, state, body } = fakeResponse();
  await reader({ request: { headers: { range: 'bytes=0-3' } }, response, entry, headOnly: false });
  assert.equal(attempts, 2);
  assert.deepEqual(urls.calls, [
    { fileId: 'file-1', force: false },
    { fileId: 'file-1', force: true },
  ]);
  assert.equal(state.statusCode, 206);
  assert.equal(state.headers['content-range'], 'bytes 0-3/123');
  assert.equal(body(), 'data');
});

test('条件请求命中 If-None-Match 返回 304 且不触发直链', async () => {
  const urls = fakeDownloadUrls();
  const reader = createWebDavFileReader({ downloadUrls: urls });
  const { response, state } = fakeResponse();
  await reader({
    request: { headers: { 'if-none-match': entry.etag, 'user-agent': 'rclone/v1.74.4' } },
    response,
    entry,
    headOnly: false,
  });
  assert.equal(state.statusCode, 304);
  assert.equal(urls.calls.length, 0);
});

test('常见媒体扩展名有正确的 Content-Type', () => {
  assert.equal(webDavContentType('movie.MKV'), 'video/x-matroska');
  assert.equal(webDavContentType('show.strm'), 'application/octet-stream');
  assert.equal(webDavContentType('poster.jpg'), 'image/jpeg');
});
