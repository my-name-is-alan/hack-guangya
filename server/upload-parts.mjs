const MIB = 1024 * 1024;

export const OSS_MULTIPART_TARGET_PARTS = 9_000;
export const OSS_LARGE_FILE_PART_SIZE = 16 * MIB;
const FIXED_PART_SIZES = new Map([
  ['4m', 4 * MIB],
  ['8m', 8 * MIB],
  ['16m', 16 * MIB],
]);

export function uploadPartSize(size, mode = 'auto') {
  if (!Number.isSafeInteger(size) || size < 0) {
    throw new RangeError('文件大小必须是非负安全整数');
  }

  const normalizedMode = String(mode).toLowerCase();
  let tierSize = FIXED_PART_SIZES.get(normalizedMode);
  if (normalizedMode === 'auto') {
    if (size <= 100 * MIB) tierSize = MIB;
    else if (size <= 1024 * MIB) tierSize = 2 * MIB;
    else if (size <= 10 * 1024 * MIB) tierSize = 4 * MIB;
    else tierSize = OSS_LARGE_FILE_PART_SIZE;
  } else if (!tierSize) {
    throw new RangeError('分片设置必须是 auto、4m、8m 或 16m');
  }

  const minimumSize = Math.ceil(size / OSS_MULTIPART_TARGET_PARTS);
  const alignedMinimumSize = Math.ceil(minimumSize / MIB) * MIB;
  return Math.max(tierSize, alignedMinimumSize);
}
