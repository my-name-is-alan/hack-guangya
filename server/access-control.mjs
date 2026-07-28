import crypto from 'node:crypto';
import net from 'node:net';

const ACCESS_COOKIE = 'guangya_access';
const SESSION_TTL_SECONDS = 12 * 60 * 60;
const SCRYPT_OPTIONS = { N: 16_384, r: 8, p: 1, maxmem: 32 * 1024 * 1024 };
const MIN_CODE_LENGTH = 8;
const MAX_CODE_LENGTH = 256;
const DEFAULT_RATE_LIMIT = Object.freeze({
  windowMs: 60_000,
  perIpFailures: 8,
  globalFailures: 64,
  maxConcurrentKdf: 4,
});

function digest(value) {
  return crypto.createHash('sha256').update(String(value)).digest('hex');
}

function equalText(left, right) {
  return crypto.timingSafeEqual(
    crypto.createHash('sha256').update(String(left)).digest(),
    crypto.createHash('sha256').update(String(right)).digest(),
  );
}

function hashCodeSync(code, salt) {
  return crypto.scryptSync(String(code), Buffer.from(salt, 'hex'), 32, SCRYPT_OPTIONS).toString('hex');
}

function hashCodeAsync(code, salt) {
  return new Promise((resolve, reject) => {
    crypto.scrypt(String(code), Buffer.from(salt, 'hex'), 32, SCRYPT_OPTIONS, (error, derivedKey) => {
      if (error) reject(error);
      else resolve(derivedKey);
    });
  });
}

function normalizeNewCode(code) {
  const normalized = String(code ?? '');
  if (normalized.length < MIN_CODE_LENGTH || normalized.length > MAX_CODE_LENGTH) {
    throw new Error(`访问码长度必须为 ${MIN_CODE_LENGTH} 到 ${MAX_CODE_LENGTH} 个字符`);
  }
  return normalized;
}

function normalizeCandidateCode(code) {
  const normalized = String(code ?? '');
  return normalized.length >= MIN_CODE_LENGTH && normalized.length <= MAX_CODE_LENGTH ? normalized : null;
}

function positiveInteger(value, fallback) {
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed > 0 ? parsed : fallback;
}

function parseCookies(header) {
  const values = new Map();
  for (const part of String(header || '').split(';')) {
    const separator = part.indexOf('=');
    if (separator < 1) continue;
    const name = part.slice(0, separator).trim();
    const value = part.slice(separator + 1).trim();
    if (name) values.set(name, value);
  }
  return values;
}

function splitForwardedElements(value) {
  const elements = [];
  let current = '';
  let quoted = false;
  let escaped = false;
  for (const character of String(value || '')) {
    if (escaped) {
      current += character;
      escaped = false;
    } else if (character === '\\' && quoted) {
      current += character;
      escaped = true;
    } else if (character === '"') {
      current += character;
      quoted = !quoted;
    } else if (character === ',' && !quoted) {
      elements.push(current);
      current = '';
    } else {
      current += character;
    }
  }
  if (current) elements.push(current);
  return elements;
}

function forwardedParameter(request, name) {
  for (const element of splitForwardedElements(request.headers.forwarded)) {
    for (const part of element.split(';')) {
      const separator = part.indexOf('=');
      if (separator < 1 || part.slice(0, separator).trim().toLowerCase() !== name) continue;
      const raw = part.slice(separator + 1).trim();
      if (raw.startsWith('"') && raw.endsWith('"')) return raw.slice(1, -1).replace(/\\(.)/g, '$1');
      return raw;
    }
  }
  return '';
}

function normalizeForwardedIp(value) {
  let candidate = String(value || '').trim();
  if (!candidate || candidate.toLowerCase() === 'unknown' || candidate.startsWith('_')) return '';
  if (candidate.startsWith('[')) {
    const closing = candidate.indexOf(']');
    if (closing < 0) return '';
    candidate = candidate.slice(1, closing);
  } else if (net.isIP(candidate) === 0) {
    const ipv4WithPort = candidate.match(/^([^:]+):\d+$/);
    if (ipv4WithPort) candidate = ipv4WithPort[1];
  }
  return net.isIP(candidate) ? candidate : '';
}

function forwardedClientIp(request) {
  const standardized = normalizeForwardedIp(forwardedParameter(request, 'for'));
  if (standardized) return standardized;
  for (const entry of String(request.headers['x-forwarded-for'] || '').split(',')) {
    const parsed = normalizeForwardedIp(entry);
    if (parsed) return parsed;
  }
  return '';
}

export function requestProtocol(request, trustedProxy = false) {
  if (trustedProxy) {
    const forwarded = String(forwardedParameter(request, 'proto')
      || String(request.headers['x-forwarded-proto'] || '').split(',')[0]).trim().toLowerCase();
    if (forwarded === 'https' || forwarded === 'http') return forwarded;
  }
  return request.socket.encrypted ? 'https' : 'http';
}

function sessionCookie(request, value, maxAge = SESSION_TTL_SECONDS, trustedProxy = false) {
  return [
    `${ACCESS_COOKIE}=${value}`,
    'Path=/',
    'HttpOnly',
    'SameSite=Strict',
    `Max-Age=${maxAge}`,
    requestProtocol(request, trustedProxy) === 'https' ? 'Secure' : '',
  ].filter(Boolean).join('; ');
}

function gateDocument(nonce) {
  return `<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>访问光鸭云盘</title>
  <style nonce="${nonce}">
    :root { color-scheme: light; font-family: Inter, "PingFang SC", "Microsoft YaHei", sans-serif; background: #f5f7fb; color: #172033; }
    * { box-sizing: border-box; }
    body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: radial-gradient(circle at 50% 0%, #fff 0, #f5f7fb 45%, #edf1f7 100%); }
    main { width: min(92vw, 400px); padding: 36px; border: 1px solid #e5e9f0; border-radius: 20px; background: rgba(255,255,255,.96); box-shadow: 0 24px 70px rgba(35,48,75,.12); }
    .mark { width: 48px; height: 48px; display: grid; place-items: center; border-radius: 14px; background: #1677ff; color: #fff; font-size: 24px; }
    h1 { margin: 24px 0 8px; font-size: 24px; letter-spacing: -.02em; }
    p { margin: 0 0 24px; color: #667085; line-height: 1.6; }
    label { display: block; margin-bottom: 8px; font-size: 14px; font-weight: 600; }
    input { width: 100%; height: 44px; padding: 0 13px; border: 1px solid #d5dae3; border-radius: 10px; font: inherit; outline: none; }
    input:focus { border-color: #1677ff; box-shadow: 0 0 0 3px rgba(22,119,255,.14); }
    button { width: 100%; height: 44px; margin-top: 14px; border: 0; border-radius: 10px; background: #1677ff; color: #fff; font: inherit; font-weight: 600; cursor: pointer; }
    button:disabled { cursor: wait; opacity: .65; }
    #error { min-height: 22px; margin: 10px 0 0; color: #d92d20; font-size: 13px; }
  </style>
</head>
<body>
  <main>
    <div class="mark" aria-hidden="true">☁</div>
    <h1>访问光鸭云盘</h1>
    <p>请输入管理员设置的访问码后继续。</p>
    <form id="access-form">
      <label for="access-code">访问码</label>
      <input id="access-code" name="code" type="password" autocomplete="current-password" required autofocus>
      <button type="submit">进入系统</button>
      <div id="error" role="alert" aria-live="polite"></div>
    </form>
  </main>
  <script nonce="${nonce}">
    const form = document.querySelector('#access-form');
    const input = document.querySelector('#access-code');
    const button = form.querySelector('button');
    const error = document.querySelector('#error');
    form.addEventListener('submit', async (event) => {
      event.preventDefault();
      button.disabled = true;
      error.textContent = '';
      try {
        const response = await fetch('/api/access/unlock', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ code: input.value }),
        });
        const payload = await response.json().catch(() => ({}));
        if (!response.ok) throw new Error(payload.error || '访问码错误');
        location.reload();
      } catch (reason) {
        error.textContent = reason.message || '暂时无法验证访问码';
        input.select();
      } finally {
        button.disabled = false;
      }
    });
  </script>
</body>
</html>`;
}

export function createAccessControl({
  database,
  initialCode = '',
  username = 'admin',
  tableName = 'access_control',
  realm = 'Guangya Sync',
  trustedProxy = false,
  persistUsername = false,
  rateLimit = {},
  now = () => Date.now(),
}) {
  if (!/^[a-z][a-z0-9_]*$/.test(tableName)) throw new Error('访问控制表名无效');
  const normalizeUsername = (value) => {
    const normalized = String(value || '').trim();
    if (!normalized || normalized.includes(':') || /[\u0000-\u001f\u007f]/.test(normalized)) {
      throw new Error('访问用户名无效');
    }
    return normalized;
  };
  let currentUsername = normalizeUsername(username);
  database.exec(`
    CREATE TABLE IF NOT EXISTS ${tableName} (
      id INTEGER PRIMARY KEY CHECK (id = 1),
      code_salt TEXT NOT NULL,
      code_hash TEXT NOT NULL,
      username TEXT,
      updated_at INTEGER NOT NULL
    );
  `);
  if (!database.prepare(`PRAGMA table_info(${tableName})`).all().some((column) => column.name === 'username')) {
    database.exec(`ALTER TABLE ${tableName} ADD COLUMN username TEXT`);
  }

  const selectRecord = database.prepare(`SELECT code_salt, code_hash, username, updated_at FROM ${tableName} WHERE id = 1`);
  const saveRecord = database.prepare(`
    INSERT INTO ${tableName} (id, code_salt, code_hash, username, updated_at)
    VALUES (1, ?, ?, ?, ?)
    ON CONFLICT(id) DO UPDATE SET
      code_salt = excluded.code_salt,
      code_hash = excluded.code_hash,
      username = CASE WHEN ? THEN excluded.username ELSE ${tableName}.username END,
      updated_at = excluded.updated_at
  `);
  let record = selectRecord.get() || null;
  if (persistUsername && record?.username) currentUsername = normalizeUsername(record.username);
  if (persistUsername && record && !record.username) {
    database.prepare(`UPDATE ${tableName} SET username = ? WHERE id = 1`).run(currentUsername);
    record = { ...record, username: currentUsername };
  }
  const sessions = new Map();
  const rateConfig = {
    windowMs: positiveInteger(rateLimit.windowMs, DEFAULT_RATE_LIMIT.windowMs),
    perIpFailures: positiveInteger(rateLimit.perIpFailures, DEFAULT_RATE_LIMIT.perIpFailures),
    globalFailures: positiveInteger(rateLimit.globalFailures, DEFAULT_RATE_LIMIT.globalFailures),
    maxConcurrentKdf: positiveInteger(rateLimit.maxConcurrentKdf, DEFAULT_RATE_LIMIT.maxConcurrentKdf),
  };
  const attemptsByIp = new Map();
  let globalFailures = [];
  let globalPending = 0;
  let activeKdf = 0;

  function persistCode(code, nextUsername = currentUsername) {
    const normalized = normalizeNewCode(code);
    const salt = crypto.randomBytes(16).toString('hex');
    const codeHash = hashCodeSync(normalized, salt);
    const storedUsername = persistUsername ? normalizeUsername(nextUsername) : null;
    saveRecord.run(salt, codeHash, storedUsername, Math.floor(now() / 1000), persistUsername ? 1 : 0);
    record = { code_salt: salt, code_hash: codeHash, username: storedUsername };
  }

  const normalizedInitialCode = String(initialCode ?? '');
  if (normalizedInitialCode) normalizeNewCode(normalizedInitialCode);
  if (!record && normalizedInitialCode) persistCode(normalizedInitialCode);

  function required() {
    return Boolean(record);
  }

  async function verifyCode(code) {
    if (!record) return true;
    const normalized = normalizeCandidateCode(code);
    if (!normalized) return false;
    try {
      const candidate = await hashCodeAsync(normalized, record.code_salt);
      const expected = Buffer.from(record.code_hash, 'hex');
      return candidate.length === expected.length && crypto.timingSafeEqual(candidate, expected);
    } catch {
      return false;
    }
  }

  function hasSession(request) {
    const token = parseCookies(request.headers.cookie).get(ACCESS_COOKIE);
    if (!token) return false;
    const tokenHash = digest(token);
    const expiresAt = sessions.get(tokenHash);
    if (!expiresAt) return false;
    if (expiresAt <= now()) {
      sessions.delete(tokenHash);
      return false;
    }
    return true;
  }

  function clientIp(request) {
    if (trustedProxy) {
      const forwarded = forwardedClientIp(request);
      if (forwarded) return forwarded;
    }
    return String(request.socket?.remoteAddress || 'unknown');
  }

  function pruneFailures(currentTime) {
    const cutoff = currentTime - rateConfig.windowMs;
    globalFailures = globalFailures.filter((timestamp) => timestamp > cutoff);
    for (const [ip, attempt] of attemptsByIp) {
      attempt.failures = attempt.failures.filter((timestamp) => timestamp > cutoff);
      if (!attempt.failures.length && attempt.pending === 0) attemptsByIp.delete(ip);
    }
  }

  function retryAfterSeconds(failures, currentTime) {
    if (!failures.length) return 1;
    return Math.max(1, Math.ceil((failures[0] + rateConfig.windowMs - currentTime) / 1000));
  }

  function rateLimitedResult(seconds) {
    return {
      ok: false,
      status: 429,
      retryAfterSeconds: Math.max(1, Number(seconds) || 1),
      method: null,
    };
  }

  function unauthorizedResult() {
    return { ok: false, status: 401, retryAfterSeconds: null, method: null };
  }

  function authorizedResult(method) {
    return { ok: true, status: 200, retryAfterSeconds: null, method };
  }

  function beginAttempt(request) {
    const currentTime = now();
    pruneFailures(currentTime);
    const ip = clientIp(request);
    const attempt = attemptsByIp.get(ip) || { failures: [], pending: 0 };
    attemptsByIp.set(ip, attempt);
    const perIpLimited = attempt.failures.length + attempt.pending >= rateConfig.perIpFailures;
    const globallyLimited = globalFailures.length + globalPending >= rateConfig.globalFailures;
    if (perIpLimited || globallyLimited) {
      if (!attempt.failures.length && attempt.pending === 0) attemptsByIp.delete(ip);
      const retryAfter = Math.max(
        perIpLimited ? retryAfterSeconds(attempt.failures, currentTime) : 1,
        globallyLimited ? retryAfterSeconds(globalFailures, currentTime) : 1,
      );
      return { allowed: false, result: rateLimitedResult(retryAfter) };
    }
    attempt.pending += 1;
    globalPending += 1;
    return { allowed: true, ip, attempt, finished: false };
  }

  function finishAttempt(ticket, succeeded, countFailure = true) {
    if (!ticket.allowed || ticket.finished) return;
    ticket.finished = true;
    ticket.attempt.pending = Math.max(0, ticket.attempt.pending - 1);
    globalPending = Math.max(0, globalPending - 1);
    if (succeeded) {
      ticket.attempt.failures = [];
    } else if (countFailure) {
      const currentTime = now();
      ticket.attempt.failures.push(currentTime);
      globalFailures.push(currentTime);
    }
    if (!ticket.attempt.failures.length && ticket.attempt.pending === 0) attemptsByIp.delete(ticket.ip);
  }

  async function checkAttempt(request, code, usernameMatches = true) {
    const ticket = beginAttempt(request);
    if (!ticket.allowed) return ticket.result;
    const normalized = normalizeCandidateCode(code);
    if (!usernameMatches || !normalized) {
      finishAttempt(ticket, false);
      return unauthorizedResult();
    }
    if (activeKdf >= rateConfig.maxConcurrentKdf) {
      finishAttempt(ticket, false, false);
      return rateLimitedResult(1);
    }
    activeKdf += 1;
    let matches = false;
    try {
      matches = await verifyCode(normalized);
    } finally {
      activeKdf = Math.max(0, activeKdf - 1);
    }
    finishAttempt(ticket, matches);
    return matches ? authorizedResult('access_code') : unauthorizedResult();
  }

  async function authenticateBasic(request) {
    const authorization = String(request.headers.authorization || '');
    const matched = authorization.match(/^Basic\s+(.+)$/i);
    if (!matched) return unauthorizedResult();
    let suppliedUsername = '';
    let suppliedCode = '';
    try {
      const decoded = Buffer.from(matched[1], 'base64').toString('utf8');
      const separator = decoded.indexOf(':');
      if (separator >= 0) {
        suppliedUsername = decoded.slice(0, separator);
        suppliedCode = decoded.slice(separator + 1);
      }
    } catch {}
    const usernameMatches = equalText(suppliedUsername, currentUsername);
    const result = await checkAttempt(request, suppliedCode, usernameMatches);
    return result.ok ? { ...result, method: 'basic' } : result;
  }

  async function authenticate(request) {
    if (!required()) return authorizedResult('none');
    if (hasSession(request)) return authorizedResult('session');
    return authenticateBasic(request);
  }

  function status(request) {
    const needsCode = required();
    return {
      required: needsCode,
      authenticated: !needsCode || hasSession(request),
      mode: needsCode ? 'access_code' : 'loopback',
      username: currentUsername,
    };
  }

  async function unlock(request, code) {
    if (!required()) return { ...authorizedResult('none'), payload: status(request), cookie: null };
    const result = await checkAttempt(request, code);
    if (!result.ok) {
      const retryPayload = result.status === 429 ? { retry_after: result.retryAfterSeconds } : {};
      return {
        ...result,
        payload: { error: result.status === 429 ? '访问尝试过于频繁，请稍后重试' : '访问码错误', ...retryPayload },
        cookie: null,
      };
    }
    const token = crypto.randomBytes(32).toString('base64url');
    sessions.set(digest(token), now() + SESSION_TTL_SECONDS * 1000);
    return {
      ...authorizedResult('access_code'),
      payload: { required: true, authenticated: true, mode: 'access_code', username: currentUsername },
      cookie: sessionCookie(request, token, SESSION_TTL_SECONDS, trustedProxy),
    };
  }

  function updateCode(request, code) {
    persistCode(code);
    sessions.clear();
    return sessionCookie(request, '', 0, trustedProxy);
  }

  function updateCredentials(request, nextUsername, code) {
    const normalizedUsername = normalizeUsername(nextUsername);
    persistCode(code, normalizedUsername);
    currentUsername = normalizedUsername;
    sessions.clear();
    return sessionCookie(request, '', 0, trustedProxy);
  }

  function reject(response, result = unauthorizedResult()) {
    const statusCode = result.status === 429 ? 429 : 401;
    const headers = {
      'content-type': 'application/json; charset=utf-8',
      'cache-control': 'no-store',
    };
    if (statusCode === 401) headers['www-authenticate'] = `Basic realm="${String(realm).replaceAll('"', '')}", charset="UTF-8"`;
    if (statusCode === 429) headers['retry-after'] = String(result.retryAfterSeconds);
    response.writeHead(statusCode, headers);
    response.end(JSON.stringify(statusCode === 429
      ? { error: '访问尝试过于频繁，请稍后重试', retry_after: result.retryAfterSeconds }
      : { error: '需要管理员身份验证' }));
  }

  function serveGate(response) {
    const nonce = crypto.randomBytes(16).toString('base64');
    const body = gateDocument(nonce);
    response.writeHead(200, {
      'content-type': 'text/html; charset=utf-8',
      'content-length': Buffer.byteLength(body),
      'cache-control': 'no-store',
      'content-security-policy': `default-src 'none'; style-src 'nonce-${nonce}'; script-src 'nonce-${nonce}'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'self'`,
      'referrer-policy': 'no-referrer',
      'x-content-type-options': 'nosniff',
    });
    response.end(body);
  }

  return {
    authenticate,
    reject,
    required,
    serveGate,
    status,
    unlock,
    updateCode,
    updateCredentials,
    username: () => currentUsername,
    verifyCode,
  };
}
