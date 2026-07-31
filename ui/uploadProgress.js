const terminalStates = new Set(['done', 'error']);

export function nextUploadProgress(previous, payload, updatedAt = Date.now()) {
  const current = previous || {
    percent: 0,
    state: 'queued',
    stage: '排队等待',
    bytesPerSecond: 0,
    uploadedBytes: 0,
    totalBytes: 0,
  };

  // A delayed progress event from the backend must never turn a completed or
  // failed item back into an active upload. A new explicit file event can
  // still start another upload for the same path.
  if (payload?.type === 'progress' && terminalStates.has(current.state)) return current;

  const nextState = payload?.type === 'progress'
    ? (['preparing', 'processing'].includes(current.state) ? current.state : 'uploading')
    : (payload?.state || current.state);
  let percent = Number.isFinite(Number(payload?.percent))
    ? Math.max(0, Math.min(100, Math.round(Number(payload.percent))))
    : current.percent;
  if (['queued', 'waiting-login', 'waiting-file', 'preparing'].includes(nextState)) percent = 0;
  if (['processing', 'done'].includes(nextState)) percent = 100;
  let bytesPerSecond = Number.isFinite(Number(payload?.bytes_per_second))
    ? Math.max(0, Number(payload.bytes_per_second))
    : Number(current.bytesPerSecond || 0);
  if (nextState !== 'uploading') bytesPerSecond = 0;
  const totalBytes = Number.isFinite(Number(payload?.total_bytes))
    ? Math.max(0, Number(payload.total_bytes))
    : Math.max(0, Number(current.totalBytes || 0));
  let uploadedBytes = Number.isFinite(Number(payload?.uploaded_bytes))
    ? Math.max(0, Number(payload.uploaded_bytes))
    : Math.max(0, Number(current.uploadedBytes || 0));
  if (totalBytes > 0 && payload?.uploaded_bytes == null && payload?.percent != null) {
    uploadedBytes = Math.round((percent / 100) * totalBytes);
  }
  if (['queued', 'waiting-login', 'waiting-file', 'preparing'].includes(nextState)) {
    uploadedBytes = payload?.uploaded_bytes == null ? 0 : uploadedBytes;
  }
  if (['processing', 'done'].includes(nextState) && totalBytes > 0) uploadedBytes = totalBytes;
  if (totalBytes > 0) uploadedBytes = Math.min(uploadedBytes, totalBytes);

  const defaultStage = {
    queued: '排队等待',
    'waiting-login': '等待登录',
    'waiting-file': '另外的程序正在使用该文件，释放后将自动上传',
    preparing: '正在准备',
    uploading: '正在上传',
    processing: '已上传，正在等待云端入库',
    done: '上传完成',
    error: '上传失败',
  }[nextState] || current.stage;

  return {
    percent,
    bytesPerSecond,
    uploadedBytes,
    totalBytes,
    state: nextState,
    stage: payload?.stage || (nextState === 'error' ? payload?.error : '') || defaultStage,
    updatedAt,
  };
}

export function formatUploadSpeed(bytesPerSecond) {
  const value = Number(bytesPerSecond);
  return `${((Number.isFinite(value) && value > 0 ? value : 0) / (1024 * 1024)).toFixed(2)} MiB/s`;
}

export function uploadProgressStatus(state) {
  if (state === 'error') return 'exception';
  if (state === 'done') return 'success';
  return 'normal';
}

export function orderUploadProgress(uploads) {
  return [...uploads].sort((left, right) => {
    const leftStartedAt = Number(left?.startedAt ?? left?.updatedAt ?? 0);
    const rightStartedAt = Number(right?.startedAt ?? right?.updatedAt ?? 0);
    return rightStartedAt - leftStartedAt;
  });
}
