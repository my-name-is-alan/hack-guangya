export const GCID_EXPORT_FILE_CONCURRENCY = 20;
export const GCID_EXPORT_RANGE_CONCURRENCY = 3;
export const GCID_EXPORT_RANGE_ATTEMPTS = 3;
export const GCID_EXPORT_FULL_ATTEMPTS = 3;
export const GCID_EXPORT_REQUEST_TIMEOUT_MS = 30_000;
export const GCID_EXPORT_READ_IDLE_TIMEOUT_MS = 45_000;

export class GcidExportRangeError extends Error {
  constructor(message, { retryable = true } = {}) {
    super(message);
    this.name = 'GcidExportRangeError';
    this.retryable = retryable;
  }
}

export function retryableGcidExportRangeStatus(status) {
  const code = Number(status || 0);
  return code === 401
    || code === 403
    || code === 408
    || code === 425
    || code === 429
    || code >= 500;
}

export async function retryGcidExportRange(
  operation,
  {
    attempts = GCID_EXPORT_RANGE_ATTEMPTS,
    baseDelayMs = 250,
    sleep = (delay) => new Promise((resolve) => setTimeout(resolve, delay)),
  } = {},
) {
  const totalAttempts = Math.max(1, Math.floor(Number(attempts) || 1));
  let lastError = new GcidExportRangeError('云端分段读取失败');
  for (let attempt = 0; attempt < totalAttempts; attempt += 1) {
    try {
      return await operation(attempt);
    }
    catch (error) {
      lastError = error instanceof Error ? error : new GcidExportRangeError(String(error || '云端分段读取失败'));
      if (lastError.retryable === false || attempt + 1 >= totalAttempts) break;
      await sleep(Math.max(0, baseDelayMs) * (2 ** attempt));
    }
  }
  throw lastError;
}

export const retryGcidExportFull = retryGcidExportRange;

export async function withGcidExportAttemptProgress(operation, onDelta) {
  let processed = 0n;
  try {
    return await operation((nextProcessed) => {
      const next = BigInt(nextProcessed);
      if (next < processed) throw new GcidExportRangeError('完整校验进度发生倒退', { retryable: false });
      const delta = next - processed;
      processed = next;
      if (delta) onDelta(delta);
    });
  }
  catch (error) {
    if (processed) onDelta(-processed);
    throw error;
  }
}

export async function* withGcidExportReadTimeout(
  stream,
  {
    timeoutMs = GCID_EXPORT_READ_IDLE_TIMEOUT_MS,
    abort = () => {},
    label = '完整校验读取',
  } = {},
) {
  const iterator = stream[Symbol.asyncIterator]();
  try {
    while (true) {
      let timer;
      const timeout = new Promise((_, reject) => {
        timer = setTimeout(() => {
          const error = new GcidExportRangeError(`${label}连续 ${timeoutMs}ms 无数据`);
          abort(error);
          reject(error);
        }, timeoutMs);
      });
      let item;
      try {
        item = await Promise.race([iterator.next(), timeout]);
      }
      finally {
        clearTimeout(timer);
      }
      if (item.done) break;
      yield item.value;
    }
  }
  finally {
    if (typeof iterator.return === 'function') await iterator.return();
  }
}

export async function readGcidExportRangeBody(
  stream,
  expectedBytes,
  {
    timeoutMs = GCID_EXPORT_READ_IDLE_TIMEOUT_MS,
    abort = () => {},
  } = {},
) {
  const expected = Number(expectedBytes);
  if (!Number.isSafeInteger(expected) || expected < 0) {
    throw new GcidExportRangeError('分段读取的预期字节数无效', { retryable: false });
  }
  if (!stream || typeof stream[Symbol.asyncIterator] !== 'function') {
    const error = new GcidExportRangeError('分段读取没有返回响应数据');
    abort(error);
    throw error;
  }

  const result = Buffer.allocUnsafe(expected);
  let received = 0;
  for await (const value of withGcidExportReadTimeout(stream, {
    timeoutMs,
    abort,
    label: '分段读取',
  })) {
    const bytes = Buffer.isBuffer(value) ? value : Buffer.from(value || []);
    if (bytes.length > expected - received) {
      const error = new GcidExportRangeError('分段读取返回的字节数超出请求范围');
      abort(error);
      throw error;
    }
    bytes.copy(result, received);
    received += bytes.length;
  }
  if (received !== expected) {
    throw new GcidExportRangeError('分段读取返回的字节数不完整');
  }
  return result;
}
