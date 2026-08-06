import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createProxiedFetch,
  normalizeNetworkPreferences,
  normalizeProxyUrl,
  testNetworkTarget,
} from './network-preferences.mjs';

test('network proxy preferences normalize HTTP and SOCKS5 URLs', () => {
  assert.equal(normalizeProxyUrl('127.0.0.1:7890'), 'http://127.0.0.1:7890');
  assert.equal(normalizeProxyUrl('socks://127.0.0.1'), 'socks://127.0.0.1:1080');
  assert.equal(normalizeProxyUrl('socks5h://127.0.0.1'), 'socks5://127.0.0.1:1080');
  assert.throws(() => normalizeProxyUrl('ftp://proxy.local:21'), /仅支持/);
  assert.throws(() => normalizeProxyUrl('http://proxy.local:8080/?token=x'), /查询参数/);
  assert.deepEqual(normalizeNetworkPreferences({ github: 'http://g:80', tmdb_proxy: '' }, { tg_proxy: 'socks5://t:1080' }), {
    github_proxy: 'http://g', tmdb_proxy: '', tg_proxy: 'socks5://t:1080',
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

test('network test reports reachability and never exposes proxy credentials', async () => {
  const result = await testNetworkTarget('github', {
    proxyUrl: 'http://user:secret@127.0.0.1:7890',
    fetchImpl: async () => ({ ok: false, status: 403 }),
  });
  assert.equal(result.reachable, true);
  assert.equal(result.success, true);
  assert.doesNotMatch(result.proxy, /secret|user/);
});
