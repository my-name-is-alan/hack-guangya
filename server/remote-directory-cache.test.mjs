import assert from 'node:assert/strict';
import test from 'node:test';
import {
  invalidateRemoteDirectoryIds,
  reconcileRemoteDirectoryCache,
} from './remote-directory-cache.mjs';

test('远程路径缓存按目录 ID 精确失效并连带清除后代路径', () => {
  const cache = new Map([
    ['', ''],
    ['::媒体', 'media-id'],
    ['::媒体/电影', 'movie-id'],
    ['::保留', 'keep-id'],
    ['base-id::备份', 'backup-id'],
  ]);
  assert.equal(invalidateRemoteDirectoryIds(cache, ['media-id']), 2);
  assert.deepEqual([...cache], [
    ['', ''],
    ['::保留', 'keep-id'],
    ['base-id::备份', 'backup-id'],
  ]);
});

test('完整目录读取会淘汰已删除或改名路径，不完整分页不会误删', () => {
  const cache = new Map([
    ['', ''],
    ['::媒体', 'media-id'],
    ['::媒体/电影', 'movie-id'],
    ['::保留', 'keep-id'],
  ]);
  const currentRoot = [{ fileId: 'keep-id', fileName: '保留', resType: 2 }];
  const confirmed = [];
  assert.equal(reconcileRemoteDirectoryCache(cache, '', currentRoot, {
    onConfirmed: (key) => confirmed.push(key),
  }), 0);
  assert.equal(cache.has('::媒体'), true);
  assert.deepEqual(confirmed, ['::保留']);

  assert.equal(reconcileRemoteDirectoryCache(cache, '', currentRoot, { complete: true }), 2);
  assert.deepEqual([...cache], [
    ['', ''],
    ['::保留', 'keep-id'],
  ]);

  cache.set('::保留/子目录', 'child-id');
  assert.equal(reconcileRemoteDirectoryCache(cache, 'keep-id', [{
    fileId: 'child-id',
    fileName: '新名称',
    resType: 2,
  }], { complete: true }), 1);
  assert.equal(cache.has('::保留/子目录'), false);
});
