import { fetch as undiciFetch, ProxyAgent, Socks5ProxyAgent } from 'undici';

const TARGETS = Object.freeze({
  github: 'https://api.github.com/zen',
  tg: 'https://api.telegram.org',
});

export const NETWORK_TARGETS = Object.freeze(['github', 'tmdb', 'tg', 'hdhive']);

function text(value) {
  return String(value ?? '').trim();
}

/**
 * Normalize a user supplied proxy URL without ever logging its credentials.
 * Empty values intentionally mean "use the direct connection".
 */
export function normalizeProxyUrl(value, label = '代理') {
  const raw = text(value);
  if (!raw) return '';
  if (raw.length > 512) throw new Error(`${label}地址不能超过 512 个字符`);
  let parsed;
  try {
    parsed = new URL(raw.includes('://') ? raw : `http://${raw}`);
  } catch {
    throw new Error(`${label}地址格式不正确`);
  }
  let protocol = parsed.protocol.toLowerCase();
  const allowed = new Set(['http:', 'https:', 'socks:', 'socks5:', 'socks5h:']);
  if (!allowed.has(protocol)) throw new Error(`${label}仅支持 HTTP、HTTPS 或 SOCKS5`);
  if (!parsed.hostname) throw new Error(`${label}必须包含主机地址`);
  if (parsed.search || parsed.hash) throw new Error(`${label}不能包含查询参数或片段`);
  if (parsed.port && (!/^\d+$/.test(parsed.port) || Number(parsed.port) < 1 || Number(parsed.port) > 65535)) {
    throw new Error(`${label}端口必须在 1 到 65535 之间`);
  }
  // undici's Socks5ProxyAgent intentionally accepts socks:// and socks5://
  // only. socks5h has the same remote-DNS semantics for this agent, so
  // normalize the alias before constructing the dispatcher.
  if (protocol === 'socks5h:') {
    parsed.protocol = 'socks5:';
    protocol = 'socks5:';
  }
  if ((protocol === 'socks:' || protocol === 'socks5:') && !parsed.port) parsed.port = '1080';
  if ((protocol === 'http:' || protocol === 'https:') && !parsed.port) parsed.port = protocol === 'https:' ? '443' : '80';
  return parsed.toString().replace(/\/$/, '');
}

export function normalizeNetworkPreferences(input = {}, current = {}) {
  const hasValue = (source, key) => Object.prototype.hasOwnProperty.call(source, key)
    && source[key] !== undefined && source[key] !== null;
  const firstValue = (source, keys) => {
    const key = keys.find((candidate) => hasValue(source, candidate));
    return key == null ? undefined : source[key];
  };
  const globalKeys = ['proxy_url', 'proxy', 'global_proxy', 'network_proxy'];
  const legacyKeys = ['github_proxy', 'tmdb_proxy', 'tg_proxy', 'github', 'tmdb', 'telegram_proxy', 'telegram'];
  let raw;
  if (globalKeys.some((key) => hasValue(input, key))) {
    raw = firstValue(input, globalKeys);
  } else if (legacyKeys.some((key) => hasValue(input, key))) {
    // Versions before the unified setting stored one value per upstream. On
    // upgrade, keep the first configured value as the global default.
    raw = legacyKeys.map((key) => firstValue(input, [key])).find((value) => text(value)) || '';
  } else {
    raw = firstValue(current, globalKeys);
    if (!text(raw)) raw = legacyKeys.map((key) => firstValue(current, [key])).find((value) => text(value)) || '';
  }
  return { proxy_url: normalizeProxyUrl(raw, '全局代理') };
}

const dispatcherCache = new Map();
function dispatcherFor(proxyUrl) {
  const normalized = normalizeProxyUrl(proxyUrl);
  if (!normalized) return undefined;
  if (dispatcherCache.has(normalized)) return dispatcherCache.get(normalized);
  const parsed = new URL(normalized);
  const dispatcher = parsed.protocol.startsWith('socks')
    ? new Socks5ProxyAgent(normalized)
    : new ProxyAgent(normalized);
  dispatcherCache.set(normalized, dispatcher);
  return dispatcher;
}

/** Return a fetch compatible function using the requested proxy. */
export function createProxiedFetch(proxyUrl = '', baseFetch = undiciFetch) {
  const normalized = normalizeProxyUrl(proxyUrl);
  const dispatcher = dispatcherFor(normalized);
  return (url, options = {}) => {
    const next = { ...options };
    if (dispatcher && !next.dispatcher) next.dispatcher = dispatcher;
    return baseFetch(url, next);
  };
}

function safeProxyLabel(proxyUrl) {
  if (!proxyUrl) return '直连';
  try {
    const parsed = new URL(proxyUrl);
    return `${parsed.protocol}//${parsed.hostname}${parsed.port ? `:${parsed.port}` : ''}`;
  } catch {
    return '已配置代理';
  }
}

function safeErrorMessage(error, proxyUrl) {
  let value = text(error?.message || error);
  const candidates = [text(proxyUrl)];
  try {
    const parsed = new URL(proxyUrl);
    if (parsed.username) candidates.push(parsed.username);
    if (parsed.password) candidates.push(parsed.password);
  } catch {}
  for (const candidate of candidates.filter(Boolean)) value = value.replaceAll(candidate, '[proxy-secret]');
  return value.slice(0, 240);
}

/**
 * Probe one of the supported upstreams. A non-2xx response still proves the
 * network path is reachable, so the result distinguishes reachability from
 * an application-level authorization failure.
 */
export async function testNetworkTarget(target, {
  proxyUrl = '',
  tmdbApiBase = 'https://api.themoviedb.org/3',
  tmdbApiKey = '',
  hdhiveBaseUrl = '',
  fetchImpl = undiciFetch,
} = {}) {
  const normalizedTarget = text(target).toLowerCase();
  if (!NETWORK_TARGETS.includes(normalizedTarget)) throw new Error('不支持的网络测试目标');
  let endpoint = TARGETS[normalizedTarget];
  const headers = { accept: 'application/json' };
  if (normalizedTarget === 'tmdb') {
    const base = text(tmdbApiBase).replace(/\/+$/, '') || 'https://api.themoviedb.org/3';
    endpoint = `${base}/configuration`;
    const key = text(tmdbApiKey);
    if (key.startsWith('eyJ') || key.length > 80) headers.authorization = `Bearer ${key}`;
    else if (key) endpoint += `?api_key=${encodeURIComponent(key)}`;
  } else if (normalizedTarget === 'hdhive') {
    endpoint = text(hdhiveBaseUrl);
    if (!endpoint) {
      return {
        target: normalizedTarget,
        success: false,
        reachable: false,
        configured: false,
        status: 0,
        latency_ms: 0,
        proxy: safeProxyLabel(proxyUrl),
        message: '尚未配置 HDHive 地址',
      };
    }
  }
  const proxied = createProxiedFetch(proxyUrl, fetchImpl);
  const started = Date.now();
  try {
    const response = await proxied(endpoint, { method: 'GET', headers, signal: AbortSignal.timeout(15_000) });
    const reachable = true;
    const ok = response.ok || (normalizedTarget !== 'tmdb' && reachable);
    return {
      target: normalizedTarget,
      success: ok,
      reachable,
      status: response.status,
      latency_ms: Date.now() - started,
      proxy: safeProxyLabel(proxyUrl),
      message: ok ? '网络可达' : reachable ? `网络可达，上游返回 HTTP ${response.status}` : `上游返回 HTTP ${response.status}`,
    };
  } catch (error) {
    return {
      target: normalizedTarget,
      success: false,
      reachable: false,
      status: 0,
      latency_ms: Date.now() - started,
      proxy: safeProxyLabel(proxyUrl),
      message: `连接失败：${safeErrorMessage(error, proxyUrl)}`,
    };
  }
}

export async function testNetworkTargets(targets = NETWORK_TARGETS, options = {}) {
  const requested = Array.isArray(targets) ? targets : NETWORK_TARGETS;
  const normalized = [...new Set(requested.map((target) => text(target).toLowerCase()))]
    .filter((target) => NETWORK_TARGETS.includes(target));
  return Promise.all(normalized.map((target) => testNetworkTarget(target, options)));
}

export function networkPreferencesPublic(preferences = {}) {
  const proxyUrl = text(preferences.proxy_url ?? preferences.proxy);
  return {
    proxy_url: proxyUrl,
    configured: Boolean(proxyUrl),
  };
}
