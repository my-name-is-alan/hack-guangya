import test from 'node:test';
import assert from 'node:assert/strict';
import {
  formatUploadSpeed,
  nextUploadProgress,
  orderUploadProgress,
  uploadProgressStatus,
} from './uploadProgress.js';
import { readRustBackendSourceSync } from './rustBackendSource.js';

const rustSource = readRustBackendSourceSync();

test('a delayed progress event cannot regress a completed upload', () => {
  const done = { percent: 100, state: 'done', stage: '上传完成', updatedAt: 10 };
  assert.equal(nextUploadProgress(done, { type: 'progress', percent: 80, stage: '正在上传' }, 20), done);
});

test('a delayed progress event cannot revive a cancelled upload', () => {
  const cancelled = { percent: 42, state: 'cancelled', stage: '已取消', updatedAt: 10 };
  assert.equal(nextUploadProgress(cancelled, { type: 'progress', percent: 60, stage: '正在上传' }, 20), cancelled);
  assert.equal(uploadProgressStatus('cancelled'), 'exception');
});

test('a delayed progress event cannot resume a paused upload', () => {
  const paused = {
    percent: 42,
    state: 'paused',
    stage: '已暂停，可从当前断点继续',
    bytesPerSecond: 0,
    uploadedBytes: 42,
    totalBytes: 100,
    updatedAt: 10,
  };
  assert.equal(nextUploadProgress(paused, {
    type: 'progress',
    percent: 60,
    uploaded_bytes: 60,
    bytes_per_second: 1024,
    stage: '正在上传',
  }, 20), paused);

  const resumed = nextUploadProgress(paused, {
    type: 'file',
    state: 'queued',
    uploaded_bytes: 0,
    total_bytes: 100,
    stage: '已恢复，等待上传通道',
  }, 30);
  assert.equal(resumed.state, 'queued');
});

test('a new queued event can restart a cancelled upload', () => {
  const result = nextUploadProgress(
    { percent: 42, state: 'cancelled', stage: '已取消', updatedAt: 10 },
    { type: 'file', state: 'queued' },
    20,
  );
  assert.equal(result.state, 'queued');
  assert.equal(result.percent, 0);
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

test('organizer metadata uploads emit the terminal transfer event after cloud confirmation', () => {
  const start = rustSource.indexOf('pub(crate) async fn organizer_upload_bytes');
  const end = rustSource.indexOf('fn archive_candidate', start);
  assert.ok(start >= 0 && end > start);
  const organizerUpload = rustSource.slice(start, end);
  assert.match(organizerUpload, /upload_item\([\s\S]*finalize_successful_upload\(/);
  assert.match(organizerUpload, /finalize_successful_upload\([\s\S]*remove_file\(&file_path\)/);
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

test('upload progress keeps source size and transferred byte counts', () => {
  const totalBytes = 40 * 1024 * 1024;
  const result = nextUploadProgress(
    { percent: 0, state: 'preparing', stage: '正在准备', bytesPerSecond: 0, uploadedBytes: 0, totalBytes },
    {
      type: 'progress',
      percent: 25,
      uploaded_bytes: 10 * 1024 * 1024,
      total_bytes: totalBytes,
      stage: '正在上传',
    },
    20,
  );
  assert.equal(result.uploadedBytes, 10 * 1024 * 1024);
  assert.equal(result.totalBytes, totalBytes);
  assert.equal(nextUploadProgress(result, { type: 'file', state: 'done' }, 30).uploadedBytes, totalBytes);
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

test('active uploads use a static progress bar', () => {
  assert.equal(uploadProgressStatus('uploading'), 'normal');
  assert.equal(uploadProgressStatus('processing'), 'normal');
  assert.equal(uploadProgressStatus('done'), 'success');
  assert.equal(uploadProgressStatus('error'), 'exception');
});

test('progress updates do not reorder uploads that are already visible', () => {
  const ordered = orderUploadProgress([
    { filePath: 'older.mp4', startedAt: 10, updatedAt: 40 },
    { filePath: 'newer.mp4', startedAt: 20, updatedAt: 30 },
  ]);
  assert.deepEqual(ordered.map((upload) => upload.filePath), ['newer.mp4', 'older.mp4']);
});
