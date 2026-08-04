import crypto from 'node:crypto';

export const GUANGYA_DEVELOPER_API_BASE = 'https://dapi.guangyapan.com';

const RETRYABLE_CODES = new Set([18010, 18013]);

function requiredText(value, label, maximum = 256) {
  const normalized = String(value || '').trim();
  if (!normalized) throw new Error(`${label}不能为空`);
  if (normalized.length > maximum) throw new Error(`${label}不能超过 ${maximum} 个字符`);
  if (/[^\x21-\x7e]/.test(normalized)) throw new Error(`${label}只能包含可见 ASCII 字符`);
  return normalized;
}

export function normalizeDeveloperCredentials(clientId, clientSecret) {
  return {
    clientId: requiredText(clientId, '开发者 client_id'),
    clientSecret: requiredText(clientSecret, '开发者 client_secret'),
  };
}

export function buildDeveloperSignature({ clientId, clientSecret, nonce, timestamp }) {
  const credentials = normalizeDeveloperCredentials(clientId, clientSecret);
  const normalizedNonce = requiredText(nonce, 'nonce', 32);
  if (normalizedNonce.length < 16) throw new Error('nonce 必须为 16 到 32 个字符');
  const normalizedTimestamp = String(timestamp);
  if (!/^\d{10,}$/.test(normalizedTimestamp)) throw new Error('timestamp 必须是 Unix 秒级时间戳');
  const source = `client_id=${credentials.clientId}&client_secret=${credentials.clientSecret}&nonce=${normalizedNonce}&timestamp=${normalizedTimestamp}`;
  const md5Bytes = crypto.createHash('md5').update(source, 'utf8').digest();
  return crypto.createHash('sha512').update(md5Bytes).digest('hex');
}

export function buildDeveloperHeaders({ clientId, clientSecret, nonce = crypto.randomBytes(16).toString('hex'), timestamp = Math.floor(Date.now() / 1000) }) {
  const credentials = normalizeDeveloperCredentials(clientId, clientSecret);
  const normalizedNonce = String(nonce);
  const normalizedTimestamp = String(timestamp);
  return {
    'content-type': 'application/json',
    client_id: credentials.clientId,
    nonce: normalizedNonce,
    timestamp: normalizedTimestamp,
    sign: buildDeveloperSignature({ ...credentials, nonce: normalizedNonce, timestamp: normalizedTimestamp }),
  };
}

export class DeveloperApiError extends Error {
  constructor(message, { code = null, httpStatus = 0, endpoint = '', retryable = false, cause } = {}) {
    super(message, cause ? { cause } : undefined);
    this.name = 'DeveloperApiError';
    this.apiCode = code == null || code === '' || !Number.isFinite(Number(code)) ? null : Number(code);
    this.httpStatus = Number(httpStatus) || 0;
    this.statusCode = this.httpStatus === 429 ? 429 : this.httpStatus >= 500 ? 502 : this.httpStatus >= 400 ? this.httpStatus : 400;
    this.endpoint = endpoint;
    this.retryable = Boolean(retryable);
  }
}

function developerErrorMessage(code, fallback = '') {
  const messages = {
    18001: '接收 TOKEN 不存在或已删除',
    18002: '接收 TOKEN 已绑定其他开发者账号',
    18003: '发送账号与接收账号相同，不能互传',
    18006: '所选文件不属于当前开发者账号',
    18007: '小号云盘空间不足',
    18008: '小号授权的目标目录已不存在',
    18009: '任务不存在，或不属于当前开发者凭据',
    18010: '操作过于频繁，请稍后重试',
    18011: '文件尚未通过预审，暂时不能秒传',
    18012: '一次最多互传 20 项',
    18013: '开发者服务繁忙，请稍后重试',
    18014: '这些文件已经传给该小号，不能重复传输',
    18020: '开发者凭据无效或已删除',
    18021: '开发者签名校验失败',
    18022: '开发者签名已过期，请校准系统时间',
    18023: '开发者请求 nonce 已被使用',
    18025: '当前开发者凭据没有此接口权限',
    18026: '当前开发者账号已被限制使用接口',
  };
  return messages[Number(code)] || String(fallback || `开发者接口失败（业务码 ${code}）`);
}

export function createGuangyaDeveloperClient({
  clientId,
  clientSecret,
  baseUrl = GUANGYA_DEVELOPER_API_BASE,
  fetchImpl = globalThis.fetch,
  timeoutMs = 30_000,
} = {}) {
  const credentials = normalizeDeveloperCredentials(clientId, clientSecret);
  const parsedBase = new URL(baseUrl);
  if (parsedBase.protocol !== 'https:' && parsedBase.hostname !== '127.0.0.1' && parsedBase.hostname !== 'localhost') {
    throw new Error('开发者接口必须使用 HTTPS');
  }
  parsedBase.pathname = parsedBase.pathname.replace(/\/+$/, '');
  parsedBase.search = '';
  parsedBase.hash = '';
  if (typeof fetchImpl !== 'function') throw new Error('当前运行环境不支持 fetch');

  return {
    async post(endpoint, body = {}) {
      const pathname = String(endpoint || '').startsWith('/') ? String(endpoint) : `/${endpoint}`;
      let response;
      try {
        response = await fetchImpl(`${parsedBase.toString().replace(/\/$/, '')}${pathname}`, {
          method: 'POST',
          headers: buildDeveloperHeaders(credentials),
          body: JSON.stringify(body || {}),
          signal: AbortSignal.timeout(timeoutMs),
        });
      } catch (cause) {
        const timedOut = ['AbortError', 'TimeoutError'].includes(cause?.name);
        throw new DeveloperApiError(
          timedOut ? `开发者接口 ${pathname} 请求超时` : `无法连接开发者接口 ${pathname}`,
          { httpStatus: timedOut ? 504 : 502, endpoint: pathname, retryable: true, cause },
        );
      }

      const raw = await response.text();
      let payload;
      try {
        payload = raw.trim() ? JSON.parse(raw.replace(/^\uFEFF/, '')) : {};
      } catch (cause) {
        throw new DeveloperApiError(`开发者接口 ${pathname} 返回了非 JSON 响应`, {
          httpStatus: response.status,
          endpoint: pathname,
          retryable: response.status >= 500,
          cause,
        });
      }
      const code = Number(payload?.code ?? 0);
      if (!response.ok || !Number.isFinite(code) || code !== 0) {
        throw new DeveloperApiError(developerErrorMessage(code, payload?.msg || payload?.message), {
          code,
          httpStatus: response.status,
          endpoint: pathname,
          retryable: response.status === 429 || response.status >= 500 || RETRYABLE_CODES.has(code),
        });
      }
      return payload;
    },
  };
}
