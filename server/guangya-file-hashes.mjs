import crypto from 'node:crypto';
import fsp from 'node:fs/promises';

export function gcidChunkSize(fileSize) {
  if (fileSize <= 0x08000000) return 256 * 1024;
  if (fileSize <= 0x10000000) return 512 * 1024;
  if (fileSize <= 0x20000000) return 1024 * 1024;
  return 2 * 1024 * 1024;
}

export function cidByteRanges(fileSize) {
  if (fileSize < 60 * 1024) return [{ start: 0, end: fileSize }];
  const middle = Math.floor(fileSize / 3);
  return [
    { start: 0, end: 20 * 1024 },
    { start: middle, end: middle + 20 * 1024 },
    { start: fileSize - 20 * 1024, end: fileSize },
  ];
}

export function createGuangyaHashAccumulator(fileSize) {
  if (!Number.isSafeInteger(fileSize) || fileSize < 0) throw new Error('秒传文件大小无效');
  const chunkSize = gcidChunkSize(fileSize);
  const gcidChunk = Buffer.allocUnsafe(chunkSize);
  const gcidHash = crypto.createHash('sha1');
  const cidHash = crypto.createHash('sha1');
  const cidRanges = cidByteRanges(fileSize);
  const expectedCidBytes = cidRanges.reduce((total, range) => total + range.end - range.start, 0);
  let gcidChunkBytes = 0;
  let cidBytes = 0;
  let position = 0;
  let finished = false;

  const flushGcidChunk = () => {
    if (!gcidChunkBytes) return;
    gcidHash.update(crypto.createHash('sha1').update(gcidChunk.subarray(0, gcidChunkBytes)).digest());
    gcidChunkBytes = 0;
  };

  return {
    update(value) {
      if (finished) throw new Error('秒传指纹已经计算完成');
      const bytes = Buffer.isBuffer(value) ? value : Buffer.from(value);
      if (position + bytes.length > fileSize) throw new Error('下载内容超过云端文件声明大小');
      const chunkStart = position;
      const chunkEnd = chunkStart + bytes.length;
      for (const range of cidRanges) {
        const overlapStart = Math.max(chunkStart, range.start);
        const overlapEnd = Math.min(chunkEnd, range.end);
        if (overlapStart >= overlapEnd) continue;
        cidHash.update(bytes.subarray(overlapStart - chunkStart, overlapEnd - chunkStart));
        cidBytes += overlapEnd - overlapStart;
      }
      let offset = 0;
      while (offset < bytes.length) {
        const copied = Math.min(chunkSize - gcidChunkBytes, bytes.length - offset);
        bytes.copy(gcidChunk, gcidChunkBytes, offset, offset + copied);
        gcidChunkBytes += copied;
        offset += copied;
        if (gcidChunkBytes === chunkSize) flushGcidChunk();
      }
      position = chunkEnd;
      return position;
    },
    finish() {
      if (finished) throw new Error('秒传指纹已经计算完成');
      finished = true;
      if (position !== fileSize || cidBytes !== expectedCidBytes) throw new Error('下载内容与云端文件声明大小不一致');
      flushGcidChunk();
      return {
        gcid: gcidHash.digest('hex').toUpperCase(),
        cid: cidHash.digest('hex').toUpperCase(),
      };
    },
    get bytesProcessed() { return position; },
  };
}

export async function calculateGuangyaStreamHashes(stream, fileSize, onProgress = () => {}) {
  const accumulator = createGuangyaHashAccumulator(fileSize);
  for await (const chunk of stream) {
    const processed = accumulator.update(chunk);
    onProgress(processed, fileSize);
  }
  return accumulator.finish();
}

export async function calculateGuangyaFileHashes(filePath, fileSize, onProgress = () => {}) {
  const handle = await fsp.open(filePath, 'r');
  const chunkSize = gcidChunkSize(fileSize);
  const buffer = Buffer.allocUnsafe(chunkSize);
  const gcidHash = crypto.createHash('sha1');
  const cidHash = crypto.createHash('sha1');
  const cidRanges = cidByteRanges(fileSize);
  const expectedCidBytes = cidRanges.reduce((total, range) => total + range.end - range.start, 0);
  let cidBytes = 0;
  let position = 0;
  try {
    while (position < fileSize) {
      const length = Math.min(chunkSize, fileSize - position);
      let bytesRead = 0;
      while (bytesRead < length) {
        const result = await handle.read(buffer, bytesRead, length - bytesRead, position + bytesRead);
        if (!result.bytesRead) break;
        bytesRead += result.bytesRead;
      }
      if (bytesRead !== length) break;
      const chunk = buffer.subarray(0, length);
      gcidHash.update(crypto.createHash('sha1').update(chunk).digest());
      const chunkEnd = position + bytesRead;
      for (const range of cidRanges) {
        const overlapStart = Math.max(position, range.start);
        const overlapEnd = Math.min(chunkEnd, range.end);
        if (overlapStart >= overlapEnd) continue;
        cidHash.update(chunk.subarray(overlapStart - position, overlapEnd - position));
        cidBytes += overlapEnd - overlapStart;
      }
      position = chunkEnd;
      onProgress(fileSize ? Math.floor(position * 100 / fileSize) : 100);
    }
  } finally {
    await handle.close();
  }
  if (position !== fileSize || cidBytes !== expectedCidBytes) {
    throw new Error('计算秒传指纹时文件大小发生变化');
  }
  return {
    gcid: gcidHash.digest('hex').toUpperCase(),
    cid: cidHash.digest('hex').toUpperCase(),
  };
}
