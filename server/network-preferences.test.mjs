import http from 'node:http';
import assert from 'node:assert/strict';
import test from 'node:test';
import { fetch as undiciFetch } from 'undici';
import {
  createProxiedFetch,
  normalizeNetworkPreferences,
  normalizeProxyUrl,
  networkPreferencesPublic,
  testNetworkTarget,
  testNetworkTargets,
} from './network-preferences.mjs';

test('network proxy preferences normalize HTTP and SOCKS5 URLs', () => {
  assert.equal(normalizeProxyUrl('127.0.0.1:7890'), 'http://127.0.0.1:7890');
  assert.equal(normalizeProxyUrl('socks://127.0.0.1'), 'socks://127.0.0.1:1080');
  assert.equal(normalizeProxyUrl('socks5h://127.0.0.1'), 'socks5://127.0.0.1:1080');
  assert.throws(() => normalizeProxyUrl('ftp://proxy.local:21'), /仅支持/);
  assert.throws(() => normalizeProxyUrl('http://proxy.local:8080/?token=x'), /查询参数/);
  assert.deepEqual(normalizeNetworkPreferences({ github: 'http://g:80', tmdb_proxy: '' }, { tg_proxy: 'socks5://t:1080' }), {
    proxy_url: 'http://g',
  });
  assert.deepEqual(normalizeNetworkPreferences({}, { github_proxy: 'http://legacy:80' }), {
    proxy_url: 'http://legacy',
  });
  assert.deepEqual(networkPreferencesPublic({ proxy_url: 'http://g' }), {
    proxy_url: 'http://g', configured: true,
  });
});

test('proxied fetch injects an undici dispatcher without changing request semantics', async () => {
  let seen;
  const fetchImpl = async (url, options) => { seen = { url, options }; return { ok: true, status: 204 }; };
  const response = await createProxiedFetch('http://127.0.0.1:7890', fetchImpl)('https://example.test', { method: 'GET' });
  assert.equal(response.status, 204);
  assert.equal(seen.url, 'https://example.test');
  assert.equal(seen.options.method, 'GET');
  assert.ok(seen.options.dispatcher);
});

test('the configured proxy works with the runtime undici fetch implementation', async (t) => {
  let targetHits = 0;
  const target = http.createServer((request, response) => {
    targetHits += 1;
    response.writeHead(200, { 'content-type': 'application/json' });
    response.end(JSON.stringify({ path: request.url }));
  });
  const proxy = http.createServer((request, response) => {
    const upstreamUrl = new URL(request.url);
    const upstream = http.request({
      hostname: upstreamUrl.hostname,
      port: upstreamUrl.port,
      path: `${upstreamUrl.pathname}${upstreamUrl.search}`,
      method: request.method,
      headers: request.headers,
    }, (upstreamResponse) => {
      response.writeHead(upstreamResponse.statusCode || 502, upstreamResponse.headers);
      upstreamResponse.pipe(response);
    });
    upstream.on('error', (error) => { response.writeHead(502); response.end(error.message); });
    request.pipe(upstream);
  });
  await Promise.all([
    new Promise((resolve) => target.listen(0, '127.0.0.1', resolve)),
    new Promise((resolve) => proxy.listen(0, '127.0.0.1', resolve)),
  ]);
  t.after(async () => {
    await Promise.all([
      new Promise((resolve) => target.close(resolve)),
      new Promise((resolve) => proxy.close(resolve)),
    ]);
  });
  const targetUrl = `http://127.0.0.1:${target.address().port}/proxy-check`;
  const proxyUrl = `http://127.0.0.1:${proxy.address().port}`;
  const response = await createProxiedFetch(proxyUrl, undiciFetch)(targetUrl);
  assert.equal(response.status, 200);
  assert.deepEqual(await response.json(), { path: '/proxy-check' });
  assert.equal(targetHits, 1);
});

test('network test reports reachability and never exposes proxy credentials', async () => {
  const result = await testNetworkTarget('github', {
    proxyUrl: 'http://user:secret@127.0.0.1:7890',
    fetchImpl: async () => ({ ok: false, status: 403 }),
  });
  assert.equal(result.reachable, true);
  assert.equal(result.success, true);
  assert.doesNotMatch(result.proxy, /secret|user/);
});

test('all network probes run in parallel and include the configured HDHive endpoint', async () => {
  const seen = [];
  const results = await testNetworkTargets(undefined, {
    proxyUrl: 'http://127.0.0.1:7890',
    hdhiveBaseUrl: 'https://hdhive.example.test',
    fetchImpl: async (url) => {
      seen.push(String(url));
      return { ok: true, status: 200 };
    },
  });
  assert.deepEqual(results.map((item) => item.target), ['github', 'tmdb', 'tg', 'hdhive']);
  assert.ok(seen.some((url) => url.startsWith('https://hdhive.example.test')));
  assert.ok(results.every((item) => item.proxy === 'http://127.0.0.1:7890'));
});
