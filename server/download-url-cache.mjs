const DEFAULT_MAX_ENTRIES = 2048;
const DEFAULT_FALLBACK_TTL_MS = 30 * 60_000;
const DEFAULT_MAX_TTL_MS = 60 * 60_000;
const DEFAULT_SAFETY_MARGIN_MS = 5 * 60_000;
const MIN_TTL_MS = 30_000;

function parseCompactDate(value) {
  // 20260813T023000Z (OSS V4 / AWS SigV4 style)
  const match = String(value || '').match(/^(\d{4})(\d{2})(\d{2})T(\d{2})(\d{2})(\d{2})Z$/);
  if (!match) return null;
  const parsed = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]), Number(match[4]), Number(match[5]), Number(match[6]));
  return Number.isFinite(parsed) ? parsed : null;
}

function epochToMs(value) {
  const number = Number(value);
  if (!Number.isFinite(number) || number <= 0) return null;
  return number < 10_000_000_000 ? number * 1000 : number;
}

export function parseSignedUrlExpiryMs(url) {
  let parsed;
  try { parsed = new URL(String(url || '')); } catch { return null; }
  const params = new Map();
  for (const [name, value] of parsed.searchParams) params.set(name.toLowerCase(), value);
  const candidates = [];
  // Aliyun OSS V1 / CloudFront style absolute unix expiry.
  for (const name of ['expires', 'x-oss-expires']) {
    const value = params.get(name);
    if (value !== undefined && /^\d+$/.test(value)) {
      // x-oss-expires can also be a relative duration (V4); treat huge values as epoch.
      if (name === 'expires' || Number(value) > 10_000_000) {
        const epoch = epochToMs(value);
        if (epoch) candidates.push(epoch);
      }
    }
  }
  // OSS V4 / AWS SigV4 style: signing date + relative duration in seconds.
  const pairs = [
    ['x-oss-date', 'x-oss-expires'],
    ['x-amz-date', 'x-amz-expires'],
  ];
  for (const [dateName, durationName] of pairs) {
    const date = parseCompactDate(params.get(dateName));
    const duration = Number(params.get(durationName));
    if (date && Number.isFinite(duration) && duration > 0 && duration <= 30 * 86_400) {
      candidates.push(date + duration * 1000);
    }
  }
  if (!candidates.length) return null;
  return Math.min(...candidates);
}

export function createDownloadUrlCache({
  fetchUrl,
  maxEntries = DEFAULT_MAX_ENTRIES,
  fallbackTtlMs = DEFAULT_FALLBACK_TTL_MS,
  maxTtlMs = DEFAULT_MAX_TTL_MS,
  safetyMarginMs = DEFAULT_SAFETY_MARGIN_MS,
  now = Date.now,
} = {}) {
  if (typeof fetchUrl !== 'function') throw new TypeError('download url cache 需要 fetchUrl 函数');
  const entries = new Map();
  const inflight = new Map();

  function expiryFor(url) {
    const current = now();
    const signed = parseSignedUrlExpiryMs(url);
    const candidate = signed ? signed - safetyMarginMs : current + fallbackTtlMs;
    return Math.max(current + MIN_TTL_MS, Math.min(candidate, current + maxTtlMs));
  }

  function store(fileId, url) {
    entries.delete(fileId);
    entries.set(fileId, { url, expiresAt: expiryFor(url) });
    while (entries.size > maxEntries) {
      const oldest = entries.keys().next().value;
      entries.delete(oldest);
    }
  }

  function peek(fileId) {
    const key = String(fileId || '');
    const entry = entries.get(key);
    if (!entry) return '';
    if (entry.expiresAt <= now()) {
      entries.delete(key);
      return '';
    }
    // Refresh LRU position on hit.
    entries.delete(key);
    entries.set(key, entry);
    return entry.url;
  }

  async function get(fileId, { force = false } = {}) {
    const key = String(fileId || '');
    if (!key) throw new Error('缺少文件 ID');
    if (force) entries.delete(key);
    const cached = peek(key);
    if (cached) return cached;
    const pending = inflight.get(key);
    if (pending) return pending;
    const promise = (async () => {
      const url = String(await fetchUrl(key) || '');
      if (!url) throw new Error('云端没有返回文件下载地址');
      store(key, url);
      return url;
    })().finally(() => { inflight.delete(key); });
    inflight.set(key, promise);
    return promise;
  }

  function invalidate(fileId) {
    entries.delete(String(fileId || ''));
  }

  function clear() {
    entries.clear();
  }

  return { get, peek, invalidate, clear, get size() { return entries.size; } };
}
