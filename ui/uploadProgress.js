const terminalStates = new Set(['done', 'error']);

export function nextUploadProgress(previous, payload, updatedAt = Date.now()) {
  const current = previous || { percent: 0, state: 'queued', stage: '排队等待', bytesPerSecond: 0 };

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
    state: nextState,
    stage: payload?.stage || (nextState === 'error' ? payload?.error : '') || defaultStage,
    updatedAt,
  };
}

export function formatUploadSpeed(bytesPerSecond) {
  const value = Number(bytesPerSecond);
  return `${((Number.isFinite(value) && value > 0 ? value : 0) / (1024 * 1024)).toFixed(2)} MB/s`;
}
