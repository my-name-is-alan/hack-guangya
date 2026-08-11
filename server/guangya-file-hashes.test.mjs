import assert from 'node:assert/strict';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {
  calculateGuangyaFileHashes,
  calculateGuangyaStreamHashes,
  cidByteRanges,
  gcidChunkSize,
} from './guangya-file-hashes.mjs';

test('Guangya GCID and CID match the current web uploader algorithm', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-hashes-'));
  const filePath = path.join(root, 'fixture.bin');
  const content = Buffer.alloc(600_000);
  for (let index = 0; index < content.length; index += 1) content[index] = index % 251;
  await fsp.writeFile(filePath, content);
  try {
    const hashes = await calculateGuangyaFileHashes(filePath, content.length);
    assert.deepEqual(hashes, {
      gcid: '3FC0617C331816DA4EE9C19C6F532F2D6D4FD6CC',
      cid: 'ECDDF55803ED503C4DF219A5C9C847860A438CB8',
    });
  } finally {
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('streamed Guangya hashes are independent of download chunk boundaries', async () => {
  const content = Buffer.alloc(900_123);
  for (let index = 0; index < content.length; index += 1) content[index] = (index * 17) % 251;
  async function* chunks() {
    let offset = 0;
    for (const length of [1, 31_337, 262_143, 7, 400_001, 99_999, content.length]) {
      if (offset >= content.length) break;
      const end = Math.min(content.length, offset + length);
      yield content.subarray(offset, end);
      offset = end;
    }
    if (offset < content.length) yield content.subarray(offset);
  }
  const streamed = await calculateGuangyaStreamHashes(chunks(), content.length);

  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-stream-hashes-'));
  const filePath = path.join(root, 'fixture.bin');
  await fsp.writeFile(filePath, content);
  try {
    assert.deepEqual(streamed, await calculateGuangyaFileHashes(filePath, content.length));
  } finally {
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('Guangya hash boundaries follow the released browser worker', () => {
  assert.equal(gcidChunkSize(0x08000000), 256 * 1024);
  assert.equal(gcidChunkSize(0x08000001), 512 * 1024);
  assert.deepEqual(cidByteRanges(60 * 1024), [
    { start: 0, end: 20 * 1024 },
    { start: 20 * 1024, end: 40 * 1024 },
    { start: 40 * 1024, end: 60 * 1024 },
  ]);
});
