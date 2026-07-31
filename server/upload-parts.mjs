const MIB = 1024 * 1024;

export const OSS_MULTIPART_TARGET_PARTS = 9_000;
export const OSS_LARGE_FILE_PART_SIZE = 16 * MIB;
export const OSS_MAX_IN_FLIGHT_PARTS = 16;
export const UPLOAD_CHECKPOINT_SAVE_INTERVAL_MS = 2_000;
export const UPLOAD_CHECKPOINT_SAVE_BYTES = 64 * MIB;
export const UPLOAD_SPEED_WINDOW_MS = 5_000;
export const UPLOAD_SPEED_MIN_SAMPLE_MS = 250;
const FIXED_PART_SIZES = new Map([
  ['4m', 4 * MIB],
  ['8m', 8 * MIB],
  ['16m', 16 * MIB],
]);

export function uploadPartSize(size, mode = 'auto') {
  if (!Number.isSafeInteger(size) || size < 0) {
    throw new RangeError('文件大小必须是非负安全整数');
  }

  const normalizedMode = String(mode).toLowerCase();
  let tierSize = FIXED_PART_SIZES.get(normalizedMode);
  if (normalizedMode === 'auto') {
    if (size <= 100 * MIB) tierSize = 4 * MIB;
    else if (size <= 1024 * MIB) tierSize = 8 * MIB;
    else tierSize = OSS_LARGE_FILE_PART_SIZE;
  } else if (!tierSize) {
    throw new RangeError('分片设置必须是 auto、4m、8m 或 16m');
  }

  const minimumSize = Math.ceil(size / OSS_MULTIPART_TARGET_PARTS);
  const alignedMinimumSize = Math.ceil(minimumSize / MIB) * MIB;
  return Math.max(tierSize, alignedMinimumSize);
}

export function multipartCheckpointHasAllParts(checkpoint, fallbackFileSize, fallbackPartSize) {
  const fileSize = Number(checkpoint?.fileSize ?? fallbackFileSize);
  const partSize = Number(checkpoint?.partSize ?? fallbackPartSize);
  const doneParts = checkpoint?.doneParts;
  return Number.isFinite(fileSize) && fileSize > 0
    && Number.isFinite(partSize) && partSize > 0
    && Array.isArray(doneParts)
    && doneParts.length >= Math.ceil(fileSize / partSize);
}

export function createUploadCheckpointSaver(save, {
  now = Date.now,
  initialUploadedBytes = 0,
  intervalMs = UPLOAD_CHECKPOINT_SAVE_INTERVAL_MS,
  bytesThreshold = UPLOAD_CHECKPOINT_SAVE_BYTES,
} = {}) {
  if (typeof save !== 'function') throw new TypeError('save 必须是函数');
  if (!Number.isFinite(intervalMs) || intervalMs < 0) throw new RangeError('断点保存间隔必须是非负数');
  if (!Number.isFinite(bytesThreshold) || bytesThreshold < 0) throw new RangeError('断点保存字节阈值必须是非负数');

  let pending = null;
  let lastSavedAt = Number(now());
  let lastSavedBytes = Math.max(0, Math.round(Number(initialUploadedBytes) || 0));

  function flush() {
    if (!pending) return false;
    const staged = pending;
    save(staged);
    pending = null;
    lastSavedAt = Number(now());
    lastSavedBytes = Math.max(lastSavedBytes, staged.uploadedBytes);
    return true;
  }

  function stage(value, { force = false } = {}) {
    const uploadedBytes = Math.max(lastSavedBytes, Math.round(Number(value?.uploadedBytes) || 0));
    pending = { ...value, uploadedBytes };
    const elapsed = Number(now()) - lastSavedAt;
    if (force || elapsed >= intervalMs || uploadedBytes - lastSavedBytes >= bytesThreshold) return flush();
    return false;
  }

  return { stage, flush };
}

export function createUploadSpeedTracker({
  now = Date.now,
  initialUploadedBytes = 0,
  windowMs = UPLOAD_SPEED_WINDOW_MS,
  minimumSampleMs = UPLOAD_SPEED_MIN_SAMPLE_MS,
  minimumGrowthSamples = 1,
} = {}) {
  if (!Number.isFinite(windowMs) || windowMs <= 0) throw new RangeError('速度窗口必须是正数');
  if (!Number.isFinite(minimumSampleMs) || minimumSampleMs < 0 || minimumSampleMs > windowMs) throw new RangeError('速度最小采样时长必须位于 0 到窗口时长之间');
  if (!Number.isInteger(minimumGrowthSamples) || minimumGrowthSamples < 1) throw new RangeError('速度最小增长样本数必须是正整数');

  const initialBytes = Math.max(0, Math.round(Number(initialUploadedBytes) || 0));
  const initialTime = Number(now());
  let firstSample = { time: initialTime, bytes: initialBytes };
  let lastBytes = initialBytes;
  let displayedSpeed = 0;
  let hasGrowth = false;
  let growthSamples = 0;
  const samples = [firstSample];

  function update(uploadedBytes) {
    const time = Math.max(samples.at(-1).time, Number(now()));
    const bytes = Math.max(lastBytes, Math.round(Number(uploadedBytes) || 0));
    if (!hasGrowth && bytes === firstSample.bytes) {
      firstSample = { time, bytes };
      samples.splice(0, samples.length, firstSample);
      return { bytesPerSecond: 0, averageBytesPerSecond: 0 };
    }
    const grew = bytes > lastBytes;
    if (grew) {
      hasGrowth = true;
      growthSamples += 1;
    }
    const sample = { time, bytes };
    samples.push(sample);
    const cutoff = time - windowMs;
    while (samples.length > 2 && samples[1].time <= cutoff) samples.shift();

    const windowStart = samples[0];
    const windowDurationMs = Math.max(0, time - windowStart.time);
    const windowSeconds = windowDurationMs / 1_000;
    const windowSpeed = windowSeconds > 0 ? Math.max(0, bytes - windowStart.bytes) / windowSeconds : 0;
    if (grew
      && growthSamples >= minimumGrowthSamples
      && windowSpeed > 0
      && windowDurationMs >= minimumSampleMs) {
      displayedSpeed = windowSpeed;
    }

    const averageSeconds = Math.max(0, time - firstSample.time) / 1_000;
    const averageSpeed = averageSeconds > 0
      ? Math.max(0, bytes - firstSample.bytes) / averageSeconds
      : 0;
    lastBytes = bytes;
    return {
      bytesPerSecond: Math.max(0, displayedSpeed),
      averageBytesPerSecond: Math.max(0, averageSpeed),
    };
  }

  return { update };
}

export function createOssPartConcurrencyLimiter(limit = OSS_MAX_IN_FLIGHT_PARTS) {
  if (!Number.isInteger(limit) || limit < 1) throw new RangeError('OSS 总分片并发上限必须是正整数');
  let available = limit;
  const waiters = [];

  function drain() {
    while (available > 0 && waiters.length) {
      const waiter = waiters.shift();
      const parallel = Math.min(waiter.requested, available);
      available -= parallel;
      let released = false;
      waiter.resolve({
        parallel,
        release() {
          if (released) return;
          released = true;
          available += parallel;
          drain();
        },
      });
    }
  }

  return {
    acquire(requested) {
      if (!Number.isInteger(requested) || requested < 1) return Promise.reject(new RangeError('OSS 分片并发数必须是正整数'));
      return new Promise((resolve) => {
        waiters.push({ requested: Math.min(requested, limit), resolve });
        drain();
      });
    },
  };
}
