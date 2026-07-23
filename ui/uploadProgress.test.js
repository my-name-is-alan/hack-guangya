import test from 'node:test';
import assert from 'node:assert/strict';
import { formatUploadSpeed, nextUploadProgress } from './uploadProgress.js';

test('a delayed progress event cannot regress a completed upload', () => {
  const done = { percent: 100, state: 'done', stage: '上传完成', updatedAt: 10 };
  assert.equal(nextUploadProgress(done, { type: 'progress', percent: 80, stage: '正在上传' }, 20), done);
});

test('cloud processing is shown as uploaded instead of uploading', () => {
  const result = nextUploadProgress(
    { percent: 100, state: 'processing', stage: '等待云端入库', updatedAt: 10 },
    { type: 'progress', percent: 100, stage: '已上传，正在等待云端入库' },
    20,
  );
  assert.equal(result.state, 'processing');
  assert.equal(result.percent, 100);
});

test('an explicit queued event can restart the same file path', () => {
  const result = nextUploadProgress(
    { percent: 100, state: 'done', stage: '上传完成', updatedAt: 10 },
    { type: 'file', state: 'queued' },
    20,
  );
  assert.equal(result.state, 'queued');
  assert.equal(result.percent, 0);
});

test('upload speed is retained while uploading and uses MB/s instead of Mbps', () => {
  const result = nextUploadProgress(
    { percent: 10, state: 'uploading', stage: '正在上传', bytesPerSecond: 0, updatedAt: 10 },
    { type: 'progress', percent: 20, bytes_per_second: 10 * 1024 * 1024, stage: '正在上传' },
    20,
  );
  assert.equal(result.bytesPerSecond, 10 * 1024 * 1024);
  assert.equal(formatUploadSpeed(result.bytesPerSecond), '10.00 MB/s');
  assert.equal(nextUploadProgress(result, { type: 'file', state: 'done' }, 30).bytesPerSecond, 0);
});

test('a busy file remains pending without becoming an upload error', () => {
  const next = nextUploadProgress(
    { percent: 0, state: 'preparing', stage: '正在准备', bytesPerSecond: 0 },
    { type: 'file', state: 'waiting-file', stage: '另外的程序正在使用该文件，释放后将自动上传' },
    123,
  );
  assert.equal(next.state, 'waiting-file');
  assert.equal(next.percent, 0);
  assert.match(next.stage, /另外的程序正在使用该文件/);
});
