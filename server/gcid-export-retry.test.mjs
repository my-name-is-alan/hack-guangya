import assert from 'node:assert/strict';
import test from 'node:test';
import {
  GCID_EXPORT_FILE_CONCURRENCY,
  GCID_EXPORT_FULL_ATTEMPTS,
  GcidExportRangeError,
  readGcidExportRangeBody,
  retryGcidExportFull,
  retryGcidExportRange,
  withGcidExportAttemptProgress,
  withGcidExportReadTimeout,
} from './gcid-export-retry.mjs';

test('GCID export hashes up to twenty files concurrently', () => {
  assert.equal(GCID_EXPORT_FILE_CONCURRENCY, 20);
});

test('GCID export retries only the range that failed', async () => {
  const calls = [0, 0, 0];
  const values = await Promise.all(calls.map((_, index) => retryGcidExportRange(async () => {
    calls[index] += 1;
    if (index === 1 && calls[index] === 1) throw new GcidExportRangeError('temporary range failure');
    return index;
  }, { baseDelayMs: 0 })));

  assert.deepEqual(values, [0, 1, 2]);
  assert.deepEqual(calls, [1, 2, 1]);
});

test('GCID export does not retry a server that explicitly rejects ranges', async () => {
  let calls = 0;
  await assert.rejects(
    retryGcidExportRange(async () => {
      calls += 1;
      throw new GcidExportRangeError('range unsupported', { retryable: false });
    }, { baseDelayMs: 0 }),
    /range unsupported/,
  );
  assert.equal(calls, 1);
});

test('full GCID verification retries three times and rolls failed progress back', async () => {
  let calls = 0;
  let progress = 0n;
  const value = await retryGcidExportFull(async () => withGcidExportAttemptProgress(async (report) => {
    calls += 1;
    report(12);
    if (calls < GCID_EXPORT_FULL_ATTEMPTS) throw new GcidExportRangeError('stream disconnected');
    report(20);
    return 'complete';
  }, (delta) => { progress += delta; }), { baseDelayMs: 0 });

  assert.equal(value, 'complete');
  assert.equal(calls, 3);
  assert.equal(progress, 20n);
});

test('full GCID verification aborts an idle stream so it can be retried', async () => {
  let aborted = false;
  const stalled = (async function* generate() {
    await new Promise((resolve) => setTimeout(resolve, 25));
    yield Buffer.from('late');
  }());

  await assert.rejects(async () => {
    for await (const _chunk of withGcidExportReadTimeout(stalled, {
      timeoutMs: 1,
      abort: () => { aborted = true; },
    })) {}
  }, /完整校验读取连续 1ms 无数据/);
  assert.equal(aborted, true);
});

test('range body reader accepts an exact response split across short chunks', async () => {
  const stream = (async function* generate() {
    yield Buffer.from('a');
    yield Buffer.from('bc');
  }());
  assert.deepEqual(await readGcidExportRangeBody(stream, 3), Buffer.from('abc'));
});

test('range body reader rejects short and oversized responses', async () => {
  await assert.rejects(
    readGcidExportRangeBody((async function* generate() { yield Buffer.from('ab'); }()), 3),
    /字节数不完整/,
  );

  let aborted = false;
  let released = false;
  const oversized = (async function* generate() {
    try {
      yield Buffer.from('abcd');
      yield Buffer.from('tail must not be read');
    }
    finally {
      released = true;
    }
  }());
  await assert.rejects(
    readGcidExportRangeBody(oversized, 3, { abort: () => { aborted = true; } }),
    /超出请求范围/,
  );
  assert.equal(aborted, true);
  assert.equal(released, true);
});

test('range body idle timeout resets after every received chunk', async () => {
  const stream = (async function* generate() {
    for (const byte of ['a', 'b', 'c']) {
      await new Promise((resolve) => setTimeout(resolve, 20));
      yield Buffer.from(byte);
    }
  }());
  const startedAt = Date.now();
  assert.deepEqual(
    await readGcidExportRangeBody(stream, 3, { timeoutMs: 50 }),
    Buffer.from('abc'),
  );
  assert.ok(Date.now() - startedAt >= 50, 'total read should exceed one idle-timeout window');
});
