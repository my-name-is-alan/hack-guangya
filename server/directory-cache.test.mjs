import assert from 'node:assert/strict';
import test from 'node:test';
import { createDirectoryCache } from './directory-cache.mjs';

test('目录缓存复用新鲜结果并将过期目录合并为一次后台刷新', async () => {
  let now = 0;
  let calls = 0;
  let release;
  const cache = createDirectoryCache({ freshMs: 100, staleMs: 1_000, now: () => now });
  const first = await cache.get('root', async () => {
    calls += 1;
    return [{ fileId: 'old', fileName: '旧文件' }];
  });
  assert.equal(first[0].fileId, 'old');
  assert.equal((await cache.get('root', async () => { calls += 1; return []; }))[0].fileId, 'old');
  assert.equal(calls, 1);

  now = 101;
  const loader = async () => {
    calls += 1;
    await new Promise((resolve) => { release = resolve; });
    return [{ fileId: 'new', fileName: '新文件' }];
  };
  const staleResults = await Promise.all(
    Array.from({ length: 100 }, () => cache.get('root', loader)),
  );
  assert.ok(staleResults.every((records) => records[0].fileId === 'old'));
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(calls, 2);
  assert.equal(cache.stats().inflight, 1);
  release();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal((await cache.get('root', loader))[0].fileId, 'new');
});

test('目录变更会失效子目录缓存，写操作失效可阻止旧请求回填', async () => {
  let now = 0;
  const cache = createDirectoryCache({ freshMs: 10, staleMs: 20, now: () => now });
  await cache.get('root', async () => [{
    fileId: 'folder',
    fileName: '资料',
    resType: 2,
    updateTime: 1,
  }]);
  await cache.get('folder', async () => [{ fileId: 'old-child', fileName: '旧内容' }]);

  now = 21;
  await cache.get('root', async () => [{
    fileId: 'folder',
    fileName: '资料',
    resType: 2,
    updateTime: 2,
  }], { force: true });
  assert.equal(cache.stats().entries, 1);

  let release;
  const pending = cache.get('root', async () => {
    await new Promise((resolve) => { release = resolve; });
    return [{ fileId: 'stale', fileName: '旧请求' }];
  }, { force: true });
  await new Promise((resolve) => setImmediate(resolve));
  cache.invalidate('root');
  release();
  await pending;
  assert.equal(cache.stats().entries, 0);
});

test('登录身份变化会清空目录缓存并阻止跨账号复用', async () => {
  const cache = createDirectoryCache();
  cache.setScope('account-a');
  await cache.get('root', async () => [{ fileId: 'private-a' }]);
  assert.equal(cache.stats().entries, 1);
  cache.setScope('account-b');
  assert.equal(cache.stats().entries, 0);
  assert.equal((await cache.get('root', async () => [{ fileId: 'private-b' }]))[0].fileId, 'private-b');
});

test('强制刷新失败时不会让写操作降级使用过期目录', async () => {
  const cache = createDirectoryCache();
  await cache.get('root', async () => [{ fileId: 'stale-target', fileName: '旧目标' }]);
  await assert.rejects(
    () => cache.get('root', async () => { throw new Error('云端目录刷新失败'); }, { force: true }),
    /刷新失败/,
  );
});
