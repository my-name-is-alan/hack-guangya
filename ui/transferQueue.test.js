import test from 'node:test';
import assert from 'node:assert/strict';
import {
  createConcurrencyQueue,
  normalizeTransferConcurrency,
} from './transferQueue.js';

function deferred() {
  let resolve;
  const promise = new Promise((done) => { resolve = done; });
  return { promise, resolve };
}

test('transfer concurrency is constrained to the supported range', () => {
  assert.equal(normalizeTransferConcurrency(1), 1);
  assert.equal(normalizeTransferConcurrency(8), 8);
  assert.equal(normalizeTransferConcurrency(0), 2);
  assert.equal(normalizeTransferConcurrency(9), 2);
});

test('download queue never starts more jobs than the configured concurrency', async () => {
  const gates = [deferred(), deferred(), deferred()];
  const started = [];
  const queue = createConcurrencyQueue(() => 2);
  gates.forEach((gate, index) => queue.enqueue(async () => {
    started.push(index);
    await gate.promise;
  }));

  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(started, [0, 1]);
  assert.equal(queue.active, 2);
  assert.equal(queue.pending, 1);

  gates[0].resolve();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(started, [0, 1, 2]);

  gates[1].resolve();
  gates[2].resolve();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(queue.active, 0);
  assert.equal(queue.pending, 0);
});
