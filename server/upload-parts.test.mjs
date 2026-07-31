import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createOssPartConcurrencyLimiter,
  createUploadCheckpointSaver,
  createUploadSpeedTracker,
  OSS_LARGE_FILE_PART_SIZE,
  OSS_MAX_IN_FLIGHT_PARTS,
  OSS_MULTIPART_TARGET_PARTS,
  UPLOAD_CHECKPOINT_SAVE_BYTES,
  multipartCheckpointHasAllParts,
  uploadPartSize,
} from './upload-parts.mjs';

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;
const partCount = (size, partSize) => Math.ceil(size / partSize);

test('OSS 分片使用安全档位并为大文件动态扩容', () => {
  assert.equal(uploadPartSize(100 * MIB), 4 * MIB);
  assert.equal(uploadPartSize(100 * MIB + 1), 8 * MIB);
  assert.equal(uploadPartSize(GIB), 8 * MIB);
  assert.equal(uploadPartSize(GIB + 1), OSS_LARGE_FILE_PART_SIZE);
  assert.equal(uploadPartSize(10 * GIB), OSS_LARGE_FILE_PART_SIZE);

  const failedFileSize = 96_220_456_048;
  const selectedPartSize = uploadPartSize(failedFileSize);
  assert.equal(selectedPartSize, OSS_LARGE_FILE_PART_SIZE);
  assert.equal(partCount(failedFileSize, selectedPartSize), 5_736);
  assert.ok(
    partCount(failedFileSize, selectedPartSize) <= OSS_MULTIPART_TARGET_PARTS,
  );
});

test('OSS 分片越过安全片数边界后按 MiB 向上扩容', () => {
  const tierBoundary = OSS_LARGE_FILE_PART_SIZE * OSS_MULTIPART_TARGET_PARTS;
  assert.equal(uploadPartSize(tierBoundary), OSS_LARGE_FILE_PART_SIZE);
  assert.equal(uploadPartSize(tierBoundary + 1), OSS_LARGE_FILE_PART_SIZE + MIB);
  assert.ok(
    partCount(tierBoundary + 1, uploadPartSize(tierBoundary + 1))
      <= OSS_MULTIPART_TARGET_PARTS,
  );
});

test('OSS 固定分片档位仍遵守安全片数上限', () => {
  assert.equal(uploadPartSize(100 * MIB, '4m'), 4 * MIB);
  assert.equal(uploadPartSize(GIB, '8m'), 8 * MIB);
  assert.equal(uploadPartSize(10 * GIB, '16m'), 16 * MIB);

  const oversized = 16 * MIB * OSS_MULTIPART_TARGET_PARTS + 1;
  assert.equal(uploadPartSize(oversized, '4m'), 17 * MIB);
  assert.ok(partCount(oversized, uploadPartSize(oversized, '4m')) <= OSS_MULTIPART_TARGET_PARTS);
});

test('OSS 分片拒绝无效文件大小', () => {
  assert.throws(() => uploadPartSize(-1), RangeError);
  assert.throws(() => uploadPartSize(Number.MAX_SAFE_INTEGER + 1), RangeError);
  assert.throws(() => uploadPartSize(MIB, '32m'), RangeError);
});

test('OSS 断点只在所有分片确认后标记可执行完成请求', () => {
  const checkpoint = { fileSize: 20 * MIB, partSize: 8 * MIB, doneParts: [{}, {}] };
  assert.equal(multipartCheckpointHasAllParts(checkpoint), false);
  checkpoint.doneParts.push({});
  assert.equal(multipartCheckpointHasAllParts(checkpoint), true);
  assert.equal(multipartCheckpointHasAllParts({ doneParts: [{}, {}] }, 16 * MIB, 8 * MIB), true);
  assert.equal(multipartCheckpointHasAllParts(null, 16 * MIB, 8 * MIB), false);
});

test('OSS 断点保存按 2 秒或 64MiB 节流并支持强制落盘', () => {
  let now = 1_000;
  const saved = [];
  const saver = createUploadCheckpointSaver((value) => saved.push(value), {
    now: () => now,
    initialUploadedBytes: 8 * MIB,
  });

  assert.equal(saver.stage({ checkpoint: 'part-1', uploadedBytes: 12 * MIB }), false);
  now += 1_999;
  assert.equal(saver.stage({ checkpoint: 'part-2', uploadedBytes: 20 * MIB }), false);
  now += 1;
  assert.equal(saver.stage({ checkpoint: 'part-3', uploadedBytes: 24 * MIB }), true);
  assert.deepEqual(saved, [{ checkpoint: 'part-3', uploadedBytes: 24 * MIB }]);

  assert.equal(saver.stage({ checkpoint: 'part-4', uploadedBytes: 24 * MIB + UPLOAD_CHECKPOINT_SAVE_BYTES }), true);
  assert.equal(saved.at(-1).checkpoint, 'part-4');

  assert.equal(saver.stage({ checkpoint: 'part-5', uploadedBytes: 96 * MIB }), false);
  assert.equal(saver.stage({ checkpoint: 'all-parts', uploadedBytes: 100 * MIB }, { force: true }), true);
  assert.equal(saved.at(-1).checkpoint, 'all-parts');
  assert.equal(saver.stage({ checkpoint: 'error-path', uploadedBytes: 104 * MIB }), false);
  assert.equal(saver.flush(), true);
  assert.equal(saved.at(-1).checkpoint, 'error-path');
  assert.equal(saver.flush(), false);
});

test('OSS 速度使用 5 秒滚动窗口且保留累计平均用于诊断', () => {
  let now = 0;
  const tracker = createUploadSpeedTracker({ now: () => now });
  assert.deepEqual(tracker.update(0), { bytesPerSecond: 0, averageBytesPerSecond: 0 });

  now = 1_000;
  let speed = tracker.update(10 * MIB);
  assert.equal(speed.bytesPerSecond, 10 * MIB);
  assert.equal(speed.averageBytesPerSecond, 10 * MIB);

  now = 6_000;
  speed = tracker.update(60 * MIB);
  assert.ok(speed.bytesPerSecond >= 9.9 * MIB && speed.bytesPerSecond <= 10.1 * MIB);
  assert.equal(speed.averageBytesPerSecond, 10 * MIB);

  now = 7_000;
  speed = tracker.update(80 * MIB);
  assert.ok(speed.bytesPerSecond > 10 * MIB, '近期提速应高于累计平均');
  assert.ok(speed.averageBytesPerSecond > 11 * MIB && speed.averageBytesPerSecond < 12 * MIB);
});

test('OSS 速度合并毫秒级并发回调并在最小观测窗口前保持为零', () => {
  let now = 10_000;
  const tracker = createUploadSpeedTracker({
    now: () => now,
    initialUploadedBytes: 100 * MIB,
  });

  now += 1;
  assert.equal(tracker.update(108 * MIB).bytesPerSecond, 0);
  assert.equal(tracker.update(116 * MIB).bytesPerSecond, 0, '同毫秒完成的分片不应产生极高瞬时值');
  now += 248;
  assert.equal(tracker.update(124 * MIB).bytesPerSecond, 0);
  now += 1;
  const stable = tracker.update(132 * MIB);
  assert.ok(stable.bytesPerSecond > 0);
  assert.ok(Number.isFinite(stable.bytesPerSecond));
});

test('OSS 速度在同毫秒四分片批次中累加到总吞吐', () => {
  let now = 0;
  const tracker = createUploadSpeedTracker({
    now: () => now,
    minimumGrowthSamples: 4,
  });
  tracker.update(0);

  now = 1_000;
  let speed;
  for (let lane = 1; lane <= 3; lane += 1) {
    speed = tracker.update(lane * 3 * MIB);
    assert.equal(speed.bytesPerSecond, 0, '首批真实并发分片未全部完成前应保持测速中');
  }
  speed = tracker.update(12 * MIB);
  assert.equal(speed.bytesPerSecond, 12 * MIB, '首批最后一个回调应显示四路总速度');

  now = 2_000;
  for (let lane = 5; lane <= 8; lane += 1) speed = tracker.update(lane * 3 * MIB);
  assert.equal(speed.bytesPerSecond, 12 * MIB, '第二批最后一个回调不应冻结在单路速度');
  assert.equal(speed.averageBytesPerSecond, 12 * MIB);
});

test('OSS 速度按小文件实际分片数立即完成首批采样', () => {
  let now = 0;
  const tracker = createUploadSpeedTracker({
    now: () => now,
    minimumGrowthSamples: 1,
  });
  now = 1_000;
  const speed = tracker.update(2 * MIB);
  assert.equal(speed.bytesPerSecond, 2 * MIB);
  assert.equal(speed.averageBytesPerSecond, 2 * MIB);
  assert.throws(
    () => createUploadSpeedTracker({ minimumGrowthSamples: 0 }),
    RangeError,
  );
});

test('OSS 分片总并发限制器不超过 16 且释放后唤醒等待上传', async () => {
  const limiter = createOssPartConcurrencyLimiter();
  const first = await limiter.acquire(8);
  const second = await limiter.acquire(8);
  assert.equal(first.parallel + second.parallel, OSS_MAX_IN_FLIGHT_PARTS);

  let thirdResolved = false;
  const thirdPromise = limiter.acquire(8).then((lease) => {
    thirdResolved = true;
    return lease;
  });
  await Promise.resolve();
  assert.equal(thirdResolved, false);

  first.release();
  const third = await thirdPromise;
  assert.equal(third.parallel, 8);
  second.release();
  third.release();
  assert.throws(() => createOssPartConcurrencyLimiter(0), RangeError);
  await assert.rejects(limiter.acquire(0), RangeError);
});
