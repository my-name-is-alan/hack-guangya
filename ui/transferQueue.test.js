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

test('download queue does not start queued work while paused and resumes on pump', async () => {
  let paused = true;
  const started = [];
  const queue = createConcurrencyQueue(() => 2, () => paused);

  queue.enqueue(async () => { started.push('first'); });
  queue.enqueue(async () => { started.push('second'); });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(started, []);
  assert.equal(queue.pending, 2);

  paused = false;
  queue.pump();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(started, ['first', 'second']);
  assert.equal(queue.active, 0);
  assert.equal(queue.pending, 0);
});

test('a queued download can be cancelled before it starts', async () => {
  const gate = deferred();
  const started = [];
  const queue = createConcurrencyQueue(() => 1);
  queue.enqueue('active', async () => {
    started.push('active');
    await gate.promise;
  });
  queue.enqueue('cancel-me', async () => { started.push('cancel-me'); });

  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(queue.cancel('cancel-me'), true);
  assert.equal(queue.cancel('cancel-me'), false);
  gate.resolve();
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(started, ['active']);
  assert.equal(queue.pending, 0);
});
