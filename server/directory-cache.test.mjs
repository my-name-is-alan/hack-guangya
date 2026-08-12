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
  const invalidatedDirectories = [];
  const cache = createDirectoryCache({
    freshMs: 10,
    staleMs: 20,
    now: () => now,
    onDirectoryInvalidated: (fileId) => invalidatedDirectories.push(fileId),
  });
  await cache.get('root', async () => [{
    fileId: 'folder',
    fileName: '资料',
    resType: 2,
    utime: 1,
  }]);
  await cache.get('folder', async () => [{ fileId: 'old-child', fileName: '旧内容' }]);

  // 内容变化（utime）只失效子目录快照，不通知"路径→ID"观察者：
  // 名字没变，映射仍然有效；上传期间目录内容持续变化，若也通知观察者
  // 会造成上传路径解析的 generation 重试风暴。
  now = 21;
  await cache.get('root', async () => [{
    fileId: 'folder',
    fileName: '资料',
    resType: 2,
    utime: 2,
  }], { force: true });
  assert.equal(cache.stats().entries, 1);
  assert.deepEqual(invalidatedDirectories, []);

  // 改名才会通知观察者（映射真的失效了）。
  await cache.get('folder', async () => [{ fileId: 'new-child', fileName: '新内容' }]);
  now = 42;
  await cache.get('root', async () => [{
    fileId: 'folder',
    fileName: '资料改名',
    resType: 2,
    utime: 2,
  }], { force: true });
  assert.equal(cache.stats().entries, 1);
  assert.deepEqual(invalidatedDirectories, ['folder']);

  let release;
  let calls = 0;
  const pending = cache.get('root', async () => {
    calls += 1;
    if (calls === 1) {
      await new Promise((resolve) => { release = resolve; });
      return [{ fileId: 'stale', fileName: '旧请求' }];
    }
    return [{ fileId: 'fresh', fileName: '失效后重读' }];
  }, { force: true });
  await new Promise((resolve) => setImmediate(resolve));
  cache.invalidate('root');
  release();
  assert.equal((await pending)[0].fileId, 'fresh');
  assert.equal(calls, 2);
  assert.equal(cache.stats().entries, 1);
  assert.equal((await cache.get('root', async () => []))[0].fileId, 'fresh');
});

test('两代请求交错完成时最旧响应不能覆盖最新目录', async () => {
  const cache = createDirectoryCache();
  let calls = 0;
  let releaseOld;
  let releaseMiddle;
  const loader = async () => {
    calls += 1;
    if (calls === 1) {
      await new Promise((resolve) => { releaseOld = resolve; });
      return [{ fileId: 'stale-old', fileName: '第一代旧结果' }];
    }
    if (calls === 2) {
      await new Promise((resolve) => { releaseMiddle = resolve; });
      return [{ fileId: 'stale-middle', fileName: '第二代旧结果' }];
    }
    return [{ fileId: 'fresh', fileName: '第三代最新结果' }];
  };

  const oldRead = cache.get('root', loader, { force: true });
  await new Promise((resolve) => setImmediate(resolve));
  cache.invalidate('root');

  const middleRead = cache.get('root', loader, { force: true });
  await new Promise((resolve) => setImmediate(resolve));
  cache.invalidate('root');

  releaseMiddle();
  assert.equal((await middleRead)[0].fileId, 'fresh');
  releaseOld();
  assert.equal((await oldRead)[0].fileId, 'fresh');
  assert.equal((await cache.get('root', async () => []))[0].fileId, 'fresh');
  assert.ok(calls >= 3);
});

test('普通访问使用 SWR，强一致写路径会等待刷新结果', async () => {
  let now = 0;
  let calls = 0;
  const cache = createDirectoryCache({ freshMs: 100, staleMs: 1_000, now: () => now });
  const loader = async () => [{ fileId: `version-${++calls}` }];
  assert.equal((await cache.get('root', loader))[0].fileId, 'version-1');

  now = 101;
  assert.equal((await cache.get('root', loader))[0].fileId, 'version-1');
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal((await cache.get('root', loader))[0].fileId, 'version-2');

  now = 202;
  assert.equal(
    (await cache.get('root', loader, { foreground: true }))[0].fileId,
    'version-3',
  );
  assert.equal(calls, 3);

  await assert.rejects(
    () => cache.get('root', async () => { throw new Error('云端强读失败'); }, { force: true }),
    /云端强读失败/,
  );
});

test('后台刷新只更新近期访问目录且不会让冷目录永久活跃', async () => {
  let now = 0;
  let calls = 0;
  const cache = createDirectoryCache({
    freshMs: 100,
    activeMs: 500,
    refreshLimit: 4,
    now: () => now,
  });
  const loader = async () => [{ fileId: `version-${++calls}` }];
  await cache.get('root', loader);
  now = 101;
  assert.deepEqual(await cache.refreshActive(), { attempted: 1, refreshed: 1 });
  assert.equal(calls, 2);

  now = 601;
  assert.deepEqual(await cache.refreshActive(), { attempted: 0, refreshed: 0 });
  assert.equal(calls, 2);
  assert.equal(cache.stats().active, 0);
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
