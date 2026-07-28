import assert from 'node:assert/strict';
import test from 'node:test';
import {
  OSS_LARGE_FILE_PART_SIZE,
  OSS_MULTIPART_TARGET_PARTS,
  uploadPartSize,
} from './upload-parts.mjs';

const MIB = 1024 * 1024;
const GIB = 1024 * MIB;
const partCount = (size, partSize) => Math.ceil(size / partSize);

test('OSS 分片使用安全档位并为大文件动态扩容', () => {
  assert.equal(uploadPartSize(100 * MIB), MIB);
  assert.equal(uploadPartSize(100 * MIB + 1), 2 * MIB);
  assert.equal(uploadPartSize(GIB), 2 * MIB);
  assert.equal(uploadPartSize(GIB + 1), 4 * MIB);
  assert.equal(uploadPartSize(10 * GIB), 4 * MIB);
  assert.equal(uploadPartSize(10 * GIB + 1), OSS_LARGE_FILE_PART_SIZE);

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
