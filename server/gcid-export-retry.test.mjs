import assert from 'node:assert/strict';
import test from 'node:test';
import {
  GCID_EXPORT_FILE_CONCURRENCY,
  GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY,
  GCID_EXPORT_SCAN_CONCURRENCY,
  GCID_EXPORT_SCAN_ATTEMPTS,
  GcidExportRangeError,
  createGcidExportRangeGate,
  readGcidExportRangeBody,
  retryGcidExportRange,
  retryGcidExportScan,
} from './gcid-export-retry.mjs';

test('GCID export hashes up to twenty files concurrently', () => {
  assert.equal(GCID_EXPORT_FILE_CONCURRENCY, 20);
  assert.equal(GCID_EXPORT_SCAN_CONCURRENCY, 24);
});

test('GCID export retries transient scan requests five times but not business failures', async () => {
  let transientCalls = 0;
  const recovered = await retryGcidExportScan(async () => {
    transientCalls += 1;
    if (transientCalls < 3) {
      const error = new Error('network disconnected');
      error.retryable = true;
      throw error;
    }
    return 'ok';
  }, { baseDelayMs: 0 });
  assert.equal(recovered, 'ok');
  assert.equal(transientCalls, 3);
  assert.equal(GCID_EXPORT_SCAN_ATTEMPTS, 5);

  let businessCalls = 0;
  await assert.rejects(retryGcidExportScan(async () => {
    businessCalls += 1;
    const error = new Error('permission denied');
    error.retryable = false;
    throw error;
  }, { baseDelayMs: 0 }), /permission denied/);
  assert.equal(businessCalls, 1);
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

test('GCID export globally caps simultaneous CDN range reads', async () => {
  assert.equal(GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY, 24);
  const acquire = createGcidExportRangeGate(3);
  let active = 0;
  let peak = 0;
  await Promise.all(Array.from({ length: 12 }, async () => {
    const release = await acquire();
    active += 1;
    peak = Math.max(peak, active);
    await new Promise((resolve) => setTimeout(resolve, 1));
    active -= 1;
    release();
  }));
  assert.equal(peak, 3);
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
