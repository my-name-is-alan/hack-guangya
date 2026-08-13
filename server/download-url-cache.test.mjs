import assert from 'node:assert/strict';
import test from 'node:test';
import { createDownloadUrlCache, parseSignedUrlExpiryMs } from './download-url-cache.mjs';

test('解析签名 URL 的过期参数', () => {
  assert.equal(parseSignedUrlExpiryMs('https://cdn.example.com/f?Expires=1770000000&Signature=x'), 1_770_000_000_000);
  assert.equal(parseSignedUrlExpiryMs('https://cdn.example.com/f?Expires=1770000000000'), 1_770_000_000_000);
  const v4 = parseSignedUrlExpiryMs('https://cdn.example.com/f?x-oss-date=20260813T000000Z&x-oss-expires=21600');
  assert.equal(v4, Date.UTC(2026, 7, 13) + 21_600_000);
  const amz = parseSignedUrlExpiryMs('https://cdn.example.com/f?X-Amz-Date=20260813T000000Z&X-Amz-Expires=3600');
  assert.equal(amz, Date.UTC(2026, 7, 13) + 3_600_000);
  assert.equal(parseSignedUrlExpiryMs('https://cdn.example.com/f?token=abc'), null);
  assert.equal(parseSignedUrlExpiryMs('not a url'), null);
});

test('直链缓存命中期内不重复请求，过期后自动刷新', async () => {
  let clock = 1_000_000;
  let calls = 0;
  const cache = createDownloadUrlCache({
    fetchUrl: async () => { calls += 1; return `https://cdn.example.com/f?v=${calls}`; },
    fallbackTtlMs: 60_000,
    now: () => clock,
  });
  assert.equal(await cache.get('file-1'), 'https://cdn.example.com/f?v=1');
  assert.equal(await cache.get('file-1'), 'https://cdn.example.com/f?v=1');
  assert.equal(calls, 1);
  clock += 61_000;
  assert.equal(await cache.get('file-1'), 'https://cdn.example.com/f?v=2');
  assert.equal(calls, 2);
});

test('缓存有效期尊重 URL 签名过期时间并留安全余量，且封顶最大 TTL', async () => {
  let clock = 0;
  const cache = createDownloadUrlCache({
    fetchUrl: async () => `https://cdn.example.com/f?Expires=${Math.floor((clock + 600_000) / 1000)}`,
    safetyMarginMs: 300_000,
    maxTtlMs: 3_600_000,
    now: () => clock,
  });
  await cache.get('file-1');
  clock = 299_000;
  assert.equal(cache.peek('file-1'), 'https://cdn.example.com/f?Expires=600');
  clock = 301_000;
  assert.equal(cache.peek('file-1'), '');

  clock = 0;
  const longCache = createDownloadUrlCache({
    fetchUrl: async () => `https://cdn.example.com/f?Expires=${Math.floor((clock + 21_600_000) / 1000)}`,
    maxTtlMs: 3_600_000,
    now: () => clock,
  });
  await longCache.get('file-2');
  clock = 3_599_000;
  assert.notEqual(longCache.peek('file-2'), '');
  clock = 3_601_000;
  assert.equal(longCache.peek('file-2'), '');
});

test('并发请求同一文件只发起一次上游调用，失败不缓存', async () => {
  let calls = 0;
  let shouldFail = true;
  const cache = createDownloadUrlCache({
    fetchUrl: async () => {
      calls += 1;
      if (shouldFail) throw new Error('上游失败');
      return 'https://cdn.example.com/ok';
    },
  });
  const results = await Promise.allSettled([cache.get('file-1'), cache.get('file-1')]);
  assert.equal(calls, 1);
  assert.ok(results.every((result) => result.status === 'rejected'));
  shouldFail = false;
  assert.equal(await cache.get('file-1'), 'https://cdn.example.com/ok');
  assert.equal(calls, 2);
});

test('超过容量按最久未使用淘汰，invalidate 强制刷新', async () => {
  let calls = 0;
  const cache = createDownloadUrlCache({
    fetchUrl: async (fileId) => { calls += 1; return `https://cdn.example.com/${fileId}?v=${calls}`; },
    maxEntries: 2,
  });
  await cache.get('a');
  await cache.get('b');
  assert.notEqual(cache.peek('a'), '');
  await cache.get('c');
  assert.equal(cache.size, 2);
  assert.equal(cache.peek('b'), '');
  assert.notEqual(cache.peek('a'), '');

  cache.invalidate('a');
  const before = calls;
  await cache.get('a');
  assert.equal(calls, before + 1);

  await cache.get('c', { force: true });
  assert.equal(calls, before + 2);
});
