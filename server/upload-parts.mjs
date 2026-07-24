const MIB = 1024 * 1024;

export const OSS_MULTIPART_TARGET_PARTS = 9_000;
export const OSS_LARGE_FILE_PART_SIZE = 16 * MIB;

export function uploadPartSize(size) {
  if (!Number.isSafeInteger(size) || size < 0) {
    throw new RangeError('文件大小必须是非负安全整数');
  }

  let tierSize;
  if (size <= 100 * MIB) tierSize = MIB;
  else if (size <= 1024 * MIB) tierSize = 2 * MIB;
  else if (size <= 10 * 1024 * MIB) tierSize = 4 * MIB;
  else tierSize = OSS_LARGE_FILE_PART_SIZE;

  const minimumSize = Math.ceil(size / OSS_MULTIPART_TARGET_PARTS);
  const alignedMinimumSize = Math.ceil(minimumSize / MIB) * MIB;
  return Math.max(tierSize, alignedMinimumSize);
}
