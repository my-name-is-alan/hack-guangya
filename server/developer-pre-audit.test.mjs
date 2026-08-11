import assert from 'node:assert/strict';
import test from 'node:test';
import {
  chunkDeveloperPreAuditFileIds,
  createDeveloperPreAuditBatch,
  decodeDeveloperPreAuditPlan,
  encodeDeveloperPreAuditPlan,
  finalizeDeveloperPreAuditSummary,
} from '../shared/developer-pre-audit.mjs';

test('developer pre-audit splits leaf ids into API-safe batches', () => {
  const chunks = chunkDeveloperPreAuditFileIds(Array.from({ length: 45 }, (_, index) => `file-${index + 1}`));
  assert.deepEqual(chunks.map((chunk) => chunk.length), [20, 20, 5]);
});

test('developer pre-audit plan preserves partial success and failed batches', () => {
  const encoded = encodeDeveloperPreAuditPlan([
    createDeveloperPreAuditBatch('task-a', 20, { passed_count: 17, rejected_count: 3, done: true }),
    createDeveloperPreAuditBatch('', 5, { rejected_count: 5, done: true, failed: true }),
  ]);
  const plan = decodeDeveloperPreAuditPlan(encoded);
  assert.deepEqual(finalizeDeveloperPreAuditSummary(plan), {
    total_count: 25,
    passed_count: 17,
    rejected_count: 8,
    pending_count: 0,
    failed_batches: 1,
    done: true,
  });
});

test('legacy single pre-audit task remains resumable', () => {
  const plan = decodeDeveloperPreAuditPlan('legacy-task-id', 7);
  assert.equal(plan.version, 1);
  assert.equal(plan.batches[0].task_id, 'legacy-task-id');
  assert.equal(plan.batches[0].file_count, 7);
  assert.equal(plan.batches[0].done, false);
});
