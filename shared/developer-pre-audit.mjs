export const DEVELOPER_PRE_AUDIT_BATCH_SIZE = 20;

export function chunkDeveloperPreAuditFileIds(fileIds, batchSize = DEVELOPER_PRE_AUDIT_BATCH_SIZE) {
  const normalizedSize = Math.max(1, Math.trunc(Number(batchSize) || DEVELOPER_PRE_AUDIT_BATCH_SIZE));
  const normalized = Array.from(new Set(Array.from(fileIds || [], (value) => String(value || '').trim()).filter(Boolean)));
  const chunks = [];
  for (let index = 0; index < normalized.length; index += normalizedSize) {
    chunks.push(normalized.slice(index, index + normalizedSize));
  }
  return chunks;
}

export function createDeveloperPreAuditBatch(taskId, fileCount, overrides = {}) {
  const count = Math.max(0, Math.trunc(Number(fileCount) || 0));
  return {
    task_id: String(taskId || '').trim(),
    file_count: count,
    passed_count: Math.max(0, Math.trunc(Number(overrides.passed_count) || 0)),
    rejected_count: Math.max(0, Math.trunc(Number(overrides.rejected_count) || 0)),
    done: Boolean(overrides.done),
    failed: Boolean(overrides.failed),
  };
}

export function encodeDeveloperPreAuditPlan(batches) {
  return JSON.stringify({
    version: 2,
    batches: Array.from(batches || [], (batch) => createDeveloperPreAuditBatch(
      batch?.task_id,
      batch?.file_count,
      batch,
    )),
  });
}

export function decodeDeveloperPreAuditPlan(value, fallbackFileCount = 0) {
  const raw = String(value || '').trim();
  if (!raw) return { version: 2, batches: [] };
  try {
    const parsed = JSON.parse(raw);
    if (Number(parsed?.version) === 2 && Array.isArray(parsed?.batches)) {
      return {
        version: 2,
        batches: parsed.batches.map((batch) => createDeveloperPreAuditBatch(
          batch?.task_id,
          batch?.file_count,
          batch,
        )),
      };
    }
  } catch {}
  return {
    version: 1,
    batches: [createDeveloperPreAuditBatch(raw, fallbackFileCount)],
  };
}

export function summarizeDeveloperPreAuditPlan(plan) {
  const batches = Array.from(plan?.batches || []);
  return batches.reduce((summary, batch) => {
    const passed = Math.max(0, Number(batch?.passed_count) || 0);
    const rejected = Math.max(0, Number(batch?.rejected_count) || 0);
    const total = Math.max(0, Number(batch?.file_count) || 0, passed + rejected);
    summary.total_count += total;
    summary.passed_count += passed;
    summary.rejected_count += rejected;
    summary.failed_batches += batch?.failed ? 1 : 0;
    summary.done = summary.done && Boolean(batch?.done);
    return summary;
  }, {
    total_count: 0,
    passed_count: 0,
    rejected_count: 0,
    pending_count: 0,
    failed_batches: 0,
    done: batches.length > 0,
  });
}

export function finalizeDeveloperPreAuditSummary(plan) {
  const summary = summarizeDeveloperPreAuditPlan(plan);
  summary.pending_count = Math.max(0, summary.total_count - summary.passed_count - summary.rejected_count);
  return summary;
}
