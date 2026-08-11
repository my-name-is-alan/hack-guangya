const ACTIVE_STATUSES = new Set(['queued', 'direct', 'auditing', 'copying', 'running']);

export function normalizeDeveloperTransferJob(value) {
  const id = String(value?.id ?? value?.job_id ?? value?.jobId ?? '').trim();
  if (!id) return null;
  const fileIds = Array.isArray(value?.file_ids) ? value.file_ids : Array.isArray(value?.fileIds) ? value.fileIds : [];
  const fileNames = Array.isArray(value?.file_names) ? value.file_names : Array.isArray(value?.fileNames) ? value.fileNames : [];
  return {
    id,
    target_id: String(value?.target_id ?? value?.targetId ?? '').trim(),
    target_name: String(value?.target_name ?? value?.targetName ?? '未命名小号').trim() || '未命名小号',
    file_ids: fileIds.map(String),
    file_names: fileNames.map(String).filter(Boolean),
    status: String(value?.status ?? 'queued').toLowerCase(),
    phase: String(value?.phase ?? '').toLowerCase(),
    total_count: Math.max(0, Number(value?.total_count ?? value?.totalCount ?? 0) || 0),
    passed_count: Math.max(0, Number(value?.passed_count ?? value?.passedCount ?? 0) || 0),
    rejected_count: Math.max(0, Number(value?.rejected_count ?? value?.rejectedCount ?? 0) || 0),
    pending_count: Math.max(0, Number(value?.pending_count ?? value?.pendingCount ?? 0) || 0),
    success_count: Math.max(0, Number(value?.success_count ?? value?.successCount ?? 0) || 0),
    skipped_count: Math.max(0, Number(value?.skipped_count ?? value?.skippedCount ?? 0) || 0),
    work_total_count: Math.max(0, Number(value?.work_total_count ?? value?.workTotalCount ?? 0) || 0),
    processed_count: Math.max(0, Number(value?.processed_count ?? value?.processedCount ?? 0) || 0),
    current_path: String(value?.current_path ?? value?.currentPath ?? ''),
    message: value?.message == null ? null : String(value.message),
    error_code: value?.error_code == null && value?.errorCode == null ? null : Number(value?.error_code ?? value?.errorCode),
    created_at: Number(value?.created_at ?? value?.createdAt ?? 0) || 0,
    updated_at: Number(value?.updated_at ?? value?.updatedAt ?? value?.created_at ?? value?.createdAt ?? 0) || 0,
  };
}

export function developerTransferIsActive(job) {
  return Boolean(job?.id && (ACTIVE_STATUSES.has(job.status) || job.phase === 'restoring'));
}

export function developerTransferPercent(job) {
  if (!job) return 0;
  if (job.status === 'success' && job.phase !== 'restoring') return 100;
  const total = Math.max(0, Number(job.work_total_count || job.total_count || 0));
  const processed = Math.max(0, Number(job.processed_count || 0));
  if (!total) return 0;
  return Math.max(0, Math.min(100, Math.floor(processed * 100 / total)));
}

export function developerTransferStageLabel(job) {
  const phase = String(job?.phase || '').toLowerCase();
  if (phase === 'obfuscating') return '处理源文件名';
  if (phase === 'pre_upload') return '文件预审';
  if (phase === 'upload') return job?.status === 'copying' ? '提交秒传' : '秒传到小号';
  if (phase === 'restoring') return '恢复源文件名';
  if (phase === 'completed') return '秒传完成';
  if (phase === 'failed') return '秒传失败';
  return '检查直传条件';
}

export function developerTransferProgressLabel(job) {
  const total = Math.max(0, Number(job?.work_total_count || job?.total_count || 0));
  const processed = Math.max(0, Math.min(total || Number.MAX_SAFE_INTEGER, Number(job?.processed_count || 0)));
  return total ? `${processed} / ${total}` : '处理中';
}
