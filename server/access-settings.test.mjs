import assert from 'node:assert/strict';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { createAccessControl } from './access-control.mjs';
import { startTestServer, stopTestServer } from './test-helpers.mjs';

function basic(username, code) {
  return `Basic ${Buffer.from(`${username}:${code}`).toString('base64')}`;
}

function cookieFrom(response) {
  return String(response.headers.get('set-cookie') || '').split(';')[0];
}

function accessRequest(remoteAddress, headers = {}) {
  return { headers, socket: { remoteAddress, encrypted: false } };
}

function responseRecorder() {
  return {
    body: '',
    headers: {},
    statusCode: 0,
    writeHead(statusCode, headers) {
      this.statusCode = statusCode;
      this.headers = headers;
    },
    end(body) {
      this.body = String(body || '');
    },
  };
}

test('访问控制拒绝长度不足或超长的初始访问码', () => {
  for (const code of ['1234567', 'x'.repeat(257)]) {
    const database = new DatabaseSync(':memory:');
    try {
      assert.throws(
        () => createAccessControl({ database, initialCode: code }),
        /访问码长度必须为 8 到 256 个字符/,
      );
    } finally {
      database.close();
    }
  }
});

test('访问控制让 unlock 与 Basic 共用每 IP 和全局失败限额', async () => {
  const database = new DatabaseSync(':memory:');
  try {
    const accessControl = createAccessControl({
      database,
      initialCode: 'correct access code',
      username: 'operator',
      rateLimit: { windowMs: 60_000, perIpFailures: 2, globalFailures: 3, maxConcurrentKdf: 2 },
    });
    const firstIp = accessRequest('192.0.2.10');
    assert.equal((await accessControl.unlock(firstIp, 'wrong code')).status, 401);
    assert.equal((await accessControl.authenticate(accessRequest('192.0.2.10', {
      authorization: basic('operator', 'wrong code'),
    }))).status, 401);

    const perIpLimited = await accessControl.unlock(firstIp, 'correct access code');
    assert.equal(perIpLimited.status, 429);
    assert.ok(perIpLimited.retryAfterSeconds >= 1);
    assert.equal(perIpLimited.payload.retry_after, perIpLimited.retryAfterSeconds);

    assert.equal((await accessControl.unlock(accessRequest('192.0.2.11'), 'wrong code')).status, 401);
    const globallyLimited = await accessControl.authenticate(accessRequest('192.0.2.12', {
      authorization: basic('operator', 'correct access code'),
    }));
    assert.equal(globallyLimited.status, 429);
    assert.ok(globallyLimited.retryAfterSeconds >= 1);

    const response = responseRecorder();
    accessControl.reject(response, globallyLimited);
    assert.equal(response.statusCode, 429);
    assert.equal(response.headers['retry-after'], String(globallyLimited.retryAfterSeconds));
    assert.equal(JSON.parse(response.body).retry_after, globallyLimited.retryAfterSeconds);
  } finally {
    database.close();
  }
});

test('访问控制保留正确 unlock、Cookie 会话与旧 Basic 登录', async () => {
  const database = new DatabaseSync(':memory:');
  try {
    const accessControl = createAccessControl({
      database,
      initialCode: 'correct access code',
      username: 'operator',
      rateLimit: { windowMs: 60_000, perIpFailures: 2, globalFailures: 20, maxConcurrentKdf: 2 },
    });
    const loginRequest = accessRequest('192.0.2.20');
    assert.equal((await accessControl.unlock(loginRequest, 'wrong code')).status, 401);
    const unlocked = await accessControl.unlock(loginRequest, 'correct access code');
    assert.equal(unlocked.status, 200);
    assert.match(unlocked.cookie, /HttpOnly/i);
    const cookie = unlocked.cookie.split(';')[0];

    assert.equal((await accessControl.unlock(loginRequest, 'wrong code')).status, 401);
    assert.equal((await accessControl.unlock(loginRequest, 'wrong code')).status, 401);
    assert.equal((await accessControl.unlock(loginRequest, 'correct access code')).status, 429);

    const session = await accessControl.authenticate(accessRequest('192.0.2.20', {
      authorization: basic('operator', 'wrong code'),
      cookie,
    }));
    assert.deepEqual(session, { ok: true, status: 200, retryAfterSeconds: null, method: 'session' });
    assert.equal(accessControl.status(accessRequest('192.0.2.20', { cookie })).authenticated, true);

    const legacyBasic = await accessControl.authenticate(accessRequest('192.0.2.21', {
      authorization: basic('operator', 'correct access code'),
    }));
    assert.deepEqual(legacyBasic, { ok: true, status: 200, retryAfterSeconds: null, method: 'basic' });
    assert.equal(await accessControl.verifyCode('x'.repeat(257)), false);
  } finally {
    database.close();
  }
});

test('独立访问控制表支持修改用户名且不会复用管理员凭据', async () => {
  const database = new DatabaseSync(':memory:');
  try {
    const admin = createAccessControl({
      database,
      initialCode: 'administrator password',
      username: 'admin',
    });
    const webdav = createAccessControl({
      database,
      initialCode: 'initial webdav password',
      username: 'mount-user',
      tableName: 'webdav_access_control',
      realm: 'Guangya WebDAV',
    });

    assert.equal((await webdav.authenticate(accessRequest('192.0.2.30', {
      authorization: basic('admin', 'administrator password'),
    }))).status, 401);
    assert.equal((await webdav.authenticate(accessRequest('192.0.2.31', {
      authorization: basic('mount-user', 'initial webdav password'),
    }))).status, 200);

    webdav.updateCredentials(accessRequest('192.0.2.32'), 'storage-user', 'replacement webdav password');
    assert.equal(webdav.status(accessRequest('192.0.2.33')).username, 'storage-user');
    assert.equal((await webdav.authenticate(accessRequest('192.0.2.34', {
      authorization: basic('mount-user', 'initial webdav password'),
    }))).status, 401);
    assert.equal((await webdav.authenticate(accessRequest('192.0.2.35', {
      authorization: basic('storage-user', 'replacement webdav password'),
    }))).status, 200);
    assert.equal(await admin.verifyCode('administrator password'), true);

    const rejected = responseRecorder();
    webdav.reject(rejected);
    assert.match(rejected.headers['www-authenticate'], /Guangya WebDAV/);
  } finally {
    database.close();
  }
});

test('WebDAV 使用独立本机端口和独立持久化账号密码', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-webdav-access-test-'));
  let instance;
  try {
    instance = await startTestServer(root);
    const mainBase = `http://127.0.0.1:${instance.port}`;
    const davBase = `http://127.0.0.1:${instance.webdavPort}/dav/`;

    assert.equal((await fetch(`${mainBase}/dav/`)).status, 404);
    assert.equal((await fetch(davBase, { method: 'OPTIONS' })).status, 503);

    const saved = await fetch(`${mainBase}/api/mount/credentials`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        username: 'storage-user',
        password: 'correct horse battery staple',
      }),
    });
    assert.equal(saved.status, 200, await saved.clone().text());
    const mount = await saved.json();
    assert.equal(mount.configured, true);
    assert.equal(mount.local_only, true);
    assert.equal(mount.endpoint, davBase);
    assert.equal(mount.password, '');

    assert.equal((await fetch(davBase, {
      method: 'OPTIONS',
      headers: { authorization: basic('admin', 'correct horse battery staple') },
    })).status, 401);
    assert.equal((await fetch(davBase, {
      method: 'OPTIONS',
      headers: { authorization: basic('storage-user', 'correct horse battery staple') },
    })).status, 204);
    const nativeOptions = await fetch(`${mainBase}/api/mount/native/options`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        options: {
          target: process.platform === 'win32' ? 'Y:' : '/mnt/test-guangya',
          access_mode: 'read_only',
          vfs_cache_mode: 'writes',
          transfers: 6,
          read_streams: 3,
          cache_size_gb: 24,
          rclone_path: 'missing-rclone-for-api-test',
        },
      }),
    });
    assert.equal(nativeOptions.status, 200, await nativeOptions.clone().text());
    assert.equal((await nativeOptions.json()).access_mode, 'read_only');
    const wrongNativePassword = await fetch(`${mainBase}/api/mount/native/start`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ password: 'wrong webdav password' }),
    });
    assert.equal(wrongNativePassword.status, 400);
    assert.match((await wrongNativePassword.json()).error, /密码错误/);

    await stopTestServer(instance);
    instance = await startTestServer(root);
    const restartedDav = `http://127.0.0.1:${instance.webdavPort}/dav/`;
    assert.equal((await fetch(restartedDav, {
      method: 'OPTIONS',
      headers: { authorization: basic('storage-user', 'correct horse battery staple') },
    })).status, 204);
    const restartedMount = await fetch(`http://127.0.0.1:${instance.port}/api/mount`).then((response) => response.json());
    assert.equal(restartedMount.username, 'storage-user');
    assert.equal(restartedMount.password, '');
    const restartedNative = await fetch(`http://127.0.0.1:${instance.port}/api/mount/native`).then((response) => response.json());
    assert.equal(restartedNative.access_mode, 'read_only');
    assert.equal(restartedNative.transfers, 6);
  } finally {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('访问码支持静态门禁、会话 Cookie、旧 Basic、改码轮换与持久化', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-access-code-test-'));
  const initialCode = 'correct horse battery staple';
  const replacementCode = 'replacement access code';
  let instance;
  try {
    instance = await startTestServer(root, {
      GUANGYA_ADMIN_USERNAME: 'operator',
      GUANGYA_ADMIN_PASSWORD: initialCode,
    });
    const base = `http://127.0.0.1:${instance.port}`;

    const initialStatus = await fetch(`${base}/api/access/status`).then((response) => response.json());
    assert.deepEqual(initialStatus, { required: true, authenticated: false, mode: 'access_code', username: 'operator' });

    const gate = await fetch(`${base}/`, { headers: { accept: 'text/html' } });
    assert.equal(gate.status, 200);
    assert.match(gate.headers.get('content-security-policy') || '', /frame-ancestors 'none'/);
    assert.match(await gate.text(), /id="access-form"/);
    assert.equal((await fetch(`${base}/api/state`)).status, 401);

    const wrong = await fetch(`${base}/api/access/unlock`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ code: 'wrong access code' }),
    });
    assert.equal(wrong.status, 401);

    const unlocked = await fetch(`${base}/api/access/unlock`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ code: initialCode }),
    });
    assert.equal(unlocked.status, 200, await unlocked.clone().text());
    const setCookie = unlocked.headers.get('set-cookie') || '';
    assert.match(setCookie, /HttpOnly/i);
    assert.match(setCookie, /SameSite=Strict/i);
    const sessionCookie = cookieFrom(unlocked);
    assert.equal((await fetch(`${base}/api/state`, { headers: { cookie: sessionCookie } })).status, 200);
    assert.equal((await fetch(`${base}/api/state`, { headers: { authorization: basic('operator', initialCode) } })).status, 200);

    const missingCurrent = await fetch(`${base}/api/access/code`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie: sessionCookie },
      body: JSON.stringify({ code: replacementCode }),
    });
    assert.equal(missingCurrent.status, 400);

    const changed = await fetch(`${base}/api/access/code`, {
      method: 'POST',
      headers: { 'content-type': 'application/json', cookie: sessionCookie },
      body: JSON.stringify({ current_code: initialCode, code: replacementCode }),
    });
    assert.equal(changed.status, 200, await changed.clone().text());
    assert.match(changed.headers.get('set-cookie') || '', /Max-Age=0/i);
    assert.equal((await fetch(`${base}/api/state`, { headers: { cookie: sessionCookie } })).status, 401);
    assert.equal((await fetch(`${base}/api/state`, { headers: { authorization: basic('operator', initialCode) } })).status, 401);
    assert.equal((await fetch(`${base}/api/state`, { headers: { authorization: basic('operator', replacementCode) } })).status, 200);

    await stopTestServer(instance);
    instance = await startTestServer(root, {
      GUANGYA_ADMIN_USERNAME: 'operator',
      GUANGYA_ADMIN_PASSWORD: initialCode,
    });
    const restartedBase = `http://127.0.0.1:${instance.port}`;
    assert.equal((await fetch(`${restartedBase}/api/access/unlock`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: initialCode }),
    })).status, 401);
    assert.equal((await fetch(`${restartedBase}/api/access/unlock`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ code: replacementCode }),
    })).status, 200);

    const database = new DatabaseSync(path.join(instance.dataDir, 'state.sqlite3'));
    const record = database.prepare('SELECT code_salt, code_hash FROM access_control WHERE id = 1').get();
    database.close();
    assert.match(record.code_salt, /^[a-f0-9]{32}$/);
    assert.match(record.code_hash, /^[a-f0-9]{64}$/);
    assert.notEqual(record.code_hash, replacementCode);
  } finally {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('传输、HDHive 与缓存设置持久化，清理缓存不触碰其他状态', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-settings-cache-test-'));
  let instance;
  try {
    instance = await startTestServer(root);
    const base = `http://127.0.0.1:${instance.port}`;
    const defaults = await fetch(`${base}/api/settings`).then((response) => response.json());
    assert.deepEqual(defaults.transfer, { upload_concurrency: 2, download_concurrency: 2, multipart: 'auto', multipart_part_size: 'auto' });
    assert.deepEqual(defaults.cache, { enabled: true, max_entries: 10_000 });
    assert.equal(defaults.hdhive.enabled, true);

    const updatedResponse = await fetch(`${base}/api/settings/transfer`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ upload_concurrency: 4, download_concurrency: 5, multipart: '8m' }),
    });
    assert.equal(updatedResponse.status, 200, await updatedResponse.clone().text());
    assert.deepEqual((await updatedResponse.json()).transfer, { upload_concurrency: 4, download_concurrency: 5, multipart: '8m', multipart_part_size: '8m' });
    assert.equal((await fetch(`${base}/api/settings/transfer`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ upload_concurrency: 9 }),
    })).status, 400);

    const disabled = await fetch(`${base}/api/hdhive/config`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ enabled: false }),
    });
    assert.equal(disabled.status, 200, await disabled.clone().text());
    assert.equal((await disabled.json()).enabled, false);

    const database = new DatabaseSync(path.join(instance.dataDir, 'state.sqlite3'));
    database.exec('PRAGMA busy_timeout = 1000');
    database.prepare('INSERT INTO file_fingerprints (file_path, size, modified_ms, gcid, updated_at) VALUES (?, ?, ?, ?, ?)')
      .run(path.join(root, 'cached.bin'), 100, '1234', 'A'.repeat(40), 1);
    database.prepare("INSERT INTO app_state (key, value, updated_at) VALUES ('cache_test_sentinel', 'keep', 1)").run();

    const before = await fetch(`${base}/api/cache`).then((response) => response.json());
    assert.equal(before.file_fingerprints.entries, 1);
    assert.ok(before.file_fingerprints.size_bytes > 0);
    assert.equal(before.remote_cache.entries, 0);
    assert.deepEqual(before.policy, { enabled: true, max_entries: 10_000 });
    const cleared = await fetch(`${base}/api/cache/clear`, { method: 'POST' }).then((response) => response.json());
    assert.equal(cleared.file_fingerprints.entries, 0);
    assert.equal(database.prepare('SELECT COUNT(*) AS count FROM file_fingerprints').get().count, 0);
    assert.equal(database.prepare("SELECT value FROM app_state WHERE key = 'cache_test_sentinel'").get().value, 'keep');

    const insertFingerprint = database.prepare('INSERT INTO file_fingerprints (file_path, size, modified_ms, gcid, updated_at) VALUES (?, ?, ?, ?, ?)');
    for (let index = 0; index < 105; index += 1) {
      insertFingerprint.run(path.join(root, `cached-${index}.bin`), index + 1, String(index), 'B'.repeat(40), index);
    }
    const boundedResponse = await fetch(`${base}/api/settings/cache`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ max_entries: 100 }),
    });
    assert.equal(boundedResponse.status, 200, await boundedResponse.clone().text());
    assert.deepEqual(await boundedResponse.json(), { enabled: true, max_entries: 100 });
    assert.equal(database.prepare('SELECT COUNT(*) AS count FROM file_fingerprints').get().count, 100);
    for (const max_entries of [99, 100_001, 100.5]) {
      assert.equal((await fetch(`${base}/api/settings/cache`, {
        method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ max_entries }),
      })).status, 400);
    }
    assert.equal((await fetch(`${base}/api/settings/cache`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ enabled: 'false' }),
    })).status, 400);
    const disabledCache = await fetch(`${base}/api/settings/cache`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ enabled: false }),
    });
    assert.equal(disabledCache.status, 200, await disabledCache.clone().text());
    assert.deepEqual(await disabledCache.json(), { enabled: false, max_entries: 100 });
    assert.equal(database.prepare('SELECT COUNT(*) AS count FROM file_fingerprints').get().count, 0);
    assert.deepEqual((await fetch(`${base}/api/cache`).then((response) => response.json())).policy, { enabled: false, max_entries: 100 });
    database.close();

    await stopTestServer(instance);
    instance = await startTestServer(root);
    const restartedBase = `http://127.0.0.1:${instance.port}`;
    const restored = await fetch(`${restartedBase}/api/settings`).then((response) => response.json());
    assert.deepEqual(restored.transfer, { upload_concurrency: 4, download_concurrency: 5, multipart: '8m', multipart_part_size: '8m' });
    assert.deepEqual(restored.cache, { enabled: false, max_entries: 100 });
    assert.deepEqual(await fetch(`${restartedBase}/api/settings/cache`).then((response) => response.json()), { enabled: false, max_entries: 100 });
    assert.equal(restored.hdhive.enabled, false);
    assert.equal((await fetch(`${restartedBase}/api/hdhive/config`).then((response) => response.json())).enabled, false);
  } finally {
    await stopTestServer(instance);
    await fsp.rm(root, { recursive: true, force: true });
  }
});
