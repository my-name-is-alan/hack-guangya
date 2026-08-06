import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {
  createNativeMountManager,
  nativeMountArguments,
  normalizeNativeMountOptions,
  prepareNativeMountTarget,
} from './native-mount.mjs';

test('原生挂载参数限制权限、并行数与缓存上限', () => {
  const options = normalizeNativeMountOptions({
    target: '/mnt/cloud',
    access_mode: 'read_write',
    vfs_cache_mode: 'full',
    transfers: 6,
    read_streams: 3,
    cache_size_gb: 32,
  }, 'linux');
  assert.equal(options.transfers, 6);
  assert.throws(() => normalizeNativeMountOptions({ ...options, access_mode: 'owner' }, 'linux'), /只读或读写/);
  assert.throws(() => normalizeNativeMountOptions({ ...options, transfers: 17 }, 'linux'), /1 到 16/);
  assert.throws(() => normalizeNativeMountOptions({ ...options, read_streams: 0 }, 'linux'), /1 到 16/);
  assert.throws(() => normalizeNativeMountOptions({ ...options, cache_size_gb: 2048 }, 'linux'), /1 到 1024/);
});

test('原生挂载命令映射只读、上传并行、读取并行和 VFS 缓存', () => {
  const options = normalizeNativeMountOptions({
    target: '/mnt/cloud',
    access_mode: 'read_only',
    vfs_cache_mode: 'writes',
    transfers: 7,
    read_streams: 5,
    cache_size_gb: 48,
  }, 'linux');
  const args = nativeMountArguments(options, '/tmp/cache', '/tmp/mount.log', 'linux');
  assert.ok(args.includes('--read-only'));
  assert.deepEqual(args.slice(args.indexOf('--transfers'), args.indexOf('--transfers') + 2), ['--transfers', '7']);
  assert.deepEqual(args.slice(args.indexOf('--vfs-read-chunk-streams'), args.indexOf('--vfs-read-chunk-streams') + 2), ['--vfs-read-chunk-streams', '5']);
  assert.deepEqual(args.slice(args.indexOf('--vfs-cache-max-size'), args.indexOf('--vfs-cache-max-size') + 2), ['--vfs-cache-max-size', '48G']);
  assert.deepEqual(args.slice(args.indexOf('--dir-cache-time'), args.indexOf('--dir-cache-time') + 2), ['--dir-cache-time', '2s']);
  assert.deepEqual(args.slice(args.indexOf('--vfs-cache-poll-interval'), args.indexOf('--vfs-cache-poll-interval') + 2), ['--vfs-cache-poll-interval', '5s']);
  assert.deepEqual(args.slice(args.indexOf('--vfs-cache-max-age'), args.indexOf('--vfs-cache-max-age') + 2), ['--vfs-cache-max-age', '24h']);
  assert.deepEqual(args.slice(args.indexOf('--vfs-read-chunk-size'), args.indexOf('--vfs-read-chunk-size') + 2), ['--vfs-read-chunk-size', '4M']);
  assert.deepEqual(args.slice(args.indexOf('--config'), args.indexOf('--config') + 2), ['--config', '/dev/null']);
});

test('服务端原生挂载默认可安全关闭并报告缺少的运行依赖', async () => {
  const manager = createNativeMountManager({
    dataDir: process.cwd(),
    initialOptions: {
      rclone_path: 'definitely-missing-rclone-binary',
      target: '/mnt/cloud',
    },
    enabled: false,
    platform: 'linux',
  });
  const info = manager.info();
  assert.equal(info.enabled, false);
  assert.equal(info.running, false);
  assert.equal(info.available, false);
  assert.match(info.prerequisite, /未启用/);
  await assert.rejects(
    () => manager.start({ endpoint: 'http://127.0.0.1:19090/dav/', username: 'user', password: 'password' }),
    /未启用/,
  );
});

test('Windows 目录挂载只移除空挂载叶子且拒绝非空目录', async () => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'guangya-native-mount-'));
  const target = path.join(root, 'mount');
  try {
    await fs.mkdir(target);
    prepareNativeMountTarget(target, 'win32');
    await assert.rejects(() => fs.stat(target), { code: 'ENOENT' });

    await fs.mkdir(target);
    await fs.writeFile(path.join(target, 'keep.txt'), 'keep');
    assert.throws(
      () => prepareNativeMountTarget(target, 'win32'),
      /必须为空/,
    );
    assert.equal(await fs.readFile(path.join(target, 'keep.txt'), 'utf8'), 'keep');
  } finally {
    await fs.rm(root, { recursive: true, force: true });
  }
});
