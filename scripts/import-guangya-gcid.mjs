import crypto from 'node:crypto';
import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import process from 'node:process';
import { DatabaseSync } from 'node:sqlite';
import { setTimeout as delay } from 'node:timers/promises';
import { pathToFileURL } from 'node:url';

const API_BASE = 'https://api.guangyapan.com';
const ACCOUNT_BASE = 'https://account.guangyapan.com';
const OAUTH_CLIENT_ID = 'aMe-8VSlkrbQXpUR';
const DEFAULT_APP_DATA = path.join(
  process.env.APPDATA || path.join(os.homedir(), 'AppData', 'Roaming'),
  'com.hackguangya.folder-sync',
);
const DEFAULT_AUTH_DB = path.join(DEFAULT_APP_DATA, 'state.sqlite3');
const TRANSIENT_TASK_CODES = new Set([145, 146, 155, 163]);

class ApiError extends Error {
  constructor(message, { apiCode = null, httpStatus = null, retryable = false } = {}) {
    super(message);
    this.name = 'ApiError';
    this.apiCode = apiCode;
    this.httpStatus = httpStatus;
    this.retryable = retryable;
  }
}

export function normalizeRelativePath(value) {
  const raw = String(value || '').replaceAll('\\', '/');
  if (!raw || raw.startsWith('/') || /^[a-zA-Z]:\//.test(raw)) {
    throw new Error(`不是合法的相对路径：${value}`);
  }
  const parts = raw.split('/');
  if (parts.some((part) => !part || part === '.' || part === '..')) {
    throw new Error(`路径包含空目录或越界片段：${value}`);
  }
  return parts.join('/');
}

export function validateExport(payload) {
  if (!payload || typeof payload !== 'object' || Array.isArray(payload)) {
    throw new Error('导入文件顶层必须是 JSON 对象');
  }
  if (payload.source !== 'guangya' || payload.hashType !== 'gcid' || payload.usesGcidInExport !== true) {
    throw new Error('只支持光鸭 GCID 导出格式');
  }
  if (!Array.isArray(payload.files) || payload.files.length === 0) {
    throw new Error('导入文件不包含 files 记录');
  }
  const seen = new Set();
  const files = payload.files.map((item, index) => {
    const relativePath = normalizeRelativePath(item?.path);
    if (seen.has(relativePath)) {
      throw new Error(`存在重复路径：${relativePath}`);
    }
    seen.add(relativePath);
    const rawSize = String(item?.size ?? '');
    if (!/^[1-9]\d*$/.test(rawSize)) {
      throw new Error(`第 ${index + 1} 条记录的文件大小无效`);
    }
    const size = Number(rawSize);
    if (!Number.isSafeInteger(size) || size <= 0) {
      throw new Error(`第 ${index + 1} 条记录的文件大小超出安全范围`);
    }
    const gcid = String(item?.gcid || '').toLowerCase();
    if (!/^[0-9a-f]{40}$/.test(gcid)) {
      throw new Error(`第 ${index + 1} 条记录的 GCID 无效`);
    }
    const parts = relativePath.split('/');
    const name = parts.pop();
    return {
      path: relativePath,
      folderPath: parts.join('/'),
      name,
      size,
      gcid,
    };
  });
  if (payload.totalFilesCount != null && Number(payload.totalFilesCount) !== files.length) {
    throw new Error(`文件总数不一致：声明 ${payload.totalFilesCount}，实际 ${files.length}`);
  }
  return files;
}

function parseArgs(argv) {
  const options = {
    destination: 'Media Library',
    concurrency: 6,
    maxAttempts: 5,
    limit: null,
    authDb: DEFAULT_AUTH_DB,
    stateDb: null,
  };
  const valueOptions = new Set([
    '--input',
    '--destination',
    '--concurrency',
    '--max-attempts',
    '--limit',
    '--auth-db',
    '--state-db',
  ]);
  for (let index = 0; index < argv.length; index += 1) {
    const key = argv[index];
    if (!valueOptions.has(key)) {
      throw new Error(`未知参数：${key}`);
    }
    const value = argv[index + 1];
    if (value == null || value.startsWith('--')) {
      throw new Error(`${key} 缺少参数值`);
    }
    index += 1;
    if (key === '--input') options.input = path.resolve(value);
    if (key === '--destination') options.destination = String(value).trim();
    if (key === '--concurrency') options.concurrency = Number(value);
    if (key === '--max-attempts') options.maxAttempts = Number(value);
    if (key === '--limit') options.limit = Number(value);
    if (key === '--auth-db') options.authDb = path.resolve(value);
    if (key === '--state-db') options.stateDb = path.resolve(value);
  }
  if (!options.input) throw new Error('请通过 --input 指定 GCID JSON 文件');
  if (!options.destination || /[\\/]/.test(options.destination)) {
    throw new Error('目标文件夹名称不能为空，也不能包含斜杠');
  }
  if (!Number.isInteger(options.concurrency) || options.concurrency < 1 || options.concurrency > 16) {
    throw new Error('--concurrency 必须是 1–16 的整数');
  }
  if (!Number.isInteger(options.maxAttempts) || options.maxAttempts < 1 || options.maxAttempts > 20) {
    throw new Error('--max-attempts 必须是 1–20 的整数');
  }
  if (options.limit != null && (!Number.isInteger(options.limit) || options.limit < 1)) {
    throw new Error('--limit 必须是正整数');
  }
  return options;
}

function unixTime() {
  return Math.floor(Date.now() / 1000);
}

function safeFileSegment(value) {
  return String(value).replace(/[<>:"/\\|?*\u0000-\u001f]/g, '_').replace(/[. ]+$/g, '').slice(0, 80) || 'import';
}

function defaultStatePath(input, destination, payload) {
  const identity = crypto
    .createHash('sha256')
    .update(JSON.stringify({
      input: path.resolve(input),
      destination,
      generatedAt: payload.generatedAt,
      totalFilesCount: payload.totalFilesCount,
      totalSize: payload.totalSize,
    }))
    .digest('hex')
    .slice(0, 12);
  return path.join(DEFAULT_APP_DATA, 'imports', `${safeFileSegment(destination)}-${identity}.sqlite3`);
}

function initializeStateDatabase(statePath, files, metadata) {
  const database = new DatabaseSync(statePath);
  database.exec(`
    PRAGMA journal_mode = WAL;
    PRAGMA synchronous = NORMAL;
    PRAGMA busy_timeout = 5000;
    CREATE TABLE IF NOT EXISTS metadata (
      key TEXT PRIMARY KEY,
      value TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS folders (
      path TEXT PRIMARY KEY,
      file_id TEXT NOT NULL,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS import_files (
      path TEXT PRIMARY KEY,
      folder_path TEXT NOT NULL,
      name TEXT NOT NULL,
      size INTEGER NOT NULL,
      gcid TEXT NOT NULL,
      status TEXT NOT NULL DEFAULT 'pending',
      attempts INTEGER NOT NULL DEFAULT 0,
      task_id TEXT,
      file_id TEXT,
      error TEXT,
      updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS import_files_status_attempts
      ON import_files(status, attempts, path);
  `);
  const upsertMetadata = database.prepare(`
    INSERT INTO metadata (key, value) VALUES (?, ?)
    ON CONFLICT(key) DO UPDATE SET value = excluded.value
  `);
  const now = unixTime();
  database.exec('BEGIN IMMEDIATE');
  try {
    for (const [key, value] of Object.entries(metadata)) {
      upsertMetadata.run(key, String(value ?? ''));
    }
    const insertFile = database.prepare(`
      INSERT INTO import_files
        (path, folder_path, name, size, gcid, status, attempts, updated_at)
      VALUES (?, ?, ?, ?, ?, 'pending', 0, ?)
      ON CONFLICT(path) DO UPDATE SET
        folder_path = excluded.folder_path,
        name = excluded.name,
        size = excluded.size,
        gcid = excluded.gcid
      WHERE import_files.status NOT IN ('imported', 'existing')
    `);
    for (const file of files) {
      insertFile.run(file.path, file.folderPath, file.name, file.size, file.gcid, now);
    }
    database.exec(`
      UPDATE import_files
      SET status = 'pending', error = '上次导入进程中断，已自动续跑', updated_at = ${now}
      WHERE status = 'processing' AND updated_at < ${now - 15 * 60};
    `);
    database.exec('COMMIT');
  } catch (error) {
    database.exec('ROLLBACK');
    database.close();
    throw error;
  }
  return database;
}

function jwtExpiry(token) {
  try {
    const parts = String(token || '').split('.');
    if (parts.length !== 3) return 0;
    const payload = JSON.parse(Buffer.from(parts[1], 'base64url').toString('utf8'));
    return Number(payload.exp || 0);
  } catch {
    return 0;
  }
}

class AuthProvider {
  constructor(databasePath) {
    this.database = new DatabaseSync(databasePath);
    this.database.exec('PRAGMA busy_timeout = 5000');
    this.accessToken = '';
    this.refreshToken = '';
    this.deviceId = '';
    this.lastReadAt = 0;
    this.refreshPromise = null;
    this.reload();
  }

  reload() {
    const auth = this.database.prepare(
      'SELECT access_token, refresh_token FROM auth_session WHERE id = 1',
    ).get();
    const device = this.database.prepare(
      "SELECT value FROM app_state WHERE key = 'device_id'",
    ).get();
    if (!auth?.access_token || !device?.value) {
      throw new Error('没有找到可用的光鸭登录态，请先在光鸭桌面端登录');
    }
    this.accessToken = String(auth.access_token);
    this.refreshToken = String(auth.refresh_token || '');
    this.deviceId = String(device.value);
    this.lastReadAt = Date.now();
  }

  async token({ forceReload = false } = {}) {
    if (forceReload || Date.now() - this.lastReadAt > 10_000) this.reload();
    const expiresAt = jwtExpiry(this.accessToken);
    if (!expiresAt || expiresAt - unixTime() > 120) return this.accessToken;
    await this.refresh();
    return this.accessToken;
  }

  async refresh() {
    if (this.refreshPromise) return this.refreshPromise;
    this.refreshPromise = (async () => {
      this.reload();
      const expiresAt = jwtExpiry(this.accessToken);
      if (!expiresAt || expiresAt - unixTime() > 120) return;
      if (!this.refreshToken) throw new Error('光鸭登录态即将过期，但没有 refresh_token');
      const response = await fetch(`${ACCOUNT_BASE}/v1/auth/token`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          grant_type: 'refresh_token',
          refresh_token: this.refreshToken,
          client_id: OAUTH_CLIENT_ID,
        }),
      });
      const raw = await response.text();
      let payload;
      try {
        payload = raw ? JSON.parse(raw.replace(/^\uFEFF/, '')) : {};
      } catch {
        throw new Error(`刷新光鸭登录态返回了非 JSON 内容（HTTP ${response.status}）`);
      }
      if (!response.ok) {
        throw new Error(payload.error_description || payload.msg || `刷新光鸭登录态失败（HTTP ${response.status}）`);
      }
      const data = payload.data && typeof payload.data === 'object' ? payload.data : payload;
      const accessToken = String(data.access_token || '');
      const refreshToken = String(data.refresh_token || this.refreshToken);
      if (!accessToken) throw new Error('刷新光鸭登录态时没有返回 access_token');
      const result = this.database.prepare(`
        UPDATE auth_session
        SET access_token = ?, refresh_token = ?, updated_at = ?
        WHERE id = 1 AND refresh_token = ?
      `).run(accessToken, refreshToken, unixTime(), this.refreshToken);
      if (Number(result.changes) === 0) {
        this.reload();
        return;
      }
      this.accessToken = accessToken;
      this.refreshToken = refreshToken;
      this.lastReadAt = Date.now();
    })().finally(() => {
      this.refreshPromise = null;
    });
    return this.refreshPromise;
  }

  close() {
    this.database.close();
  }
}

class GuangyaApi {
  constructor(auth) {
    this.auth = auth;
  }

  async post(endpoint, body, { allowedCodes = [], maxRetries = 6 } = {}) {
    let lastError;
    for (let attempt = 0; attempt <= maxRetries; attempt += 1) {
      try {
        const token = await this.auth.token({ forceReload: attempt > 0 && lastError?.apiCode === 117 });
        const traceId = crypto.randomUUID().replaceAll('-', '');
        const spanId = crypto.randomUUID().replaceAll('-', '').slice(0, 16);
        const response = await fetch(`${API_BASE}${endpoint}`, {
          method: 'POST',
          headers: {
            'content-type': 'application/json',
            authorization: `Bearer ${token}`,
            dt: '4',
            did: this.auth.deviceId,
            traceparent: `00-${traceId}-${spanId}-01`,
          },
          body: JSON.stringify(body),
          signal: AbortSignal.timeout(60_000),
        });
        const raw = await response.text();
        let payload;
        try {
          payload = raw ? JSON.parse(raw.replace(/^\uFEFF/, '')) : {};
        } catch {
          throw new ApiError(
            `${endpoint} 返回非 JSON 内容（HTTP ${response.status}）`,
            { httpStatus: response.status, retryable: response.status >= 500 },
          );
        }
        const code = Number(payload.code || 0);
        if (response.ok && (code === 0 || allowedCodes.includes(code))) return payload;
        const retryable = response.status === 429 || response.status >= 500 || code === 117;
        throw new ApiError(
          payload.msg || `光鸭接口失败：HTTP ${response.status}/业务码 ${code}`,
          { apiCode: code, httpStatus: response.status, retryable },
        );
      } catch (error) {
        lastError = error instanceof ApiError
          ? error
          : new ApiError(error.message || String(error), { retryable: true });
        if (!lastError.retryable || attempt >= maxRetries) throw lastError;
        const waitMs = Math.min(30_000, 500 * (2 ** attempt)) + Math.floor(Math.random() * 250);
        await delay(waitMs);
      }
    }
    throw lastError;
  }
}

class ImportRunner {
  constructor({ api, database, destination, concurrency, maxAttempts, limit }) {
    this.api = api;
    this.database = database;
    this.destination = destination;
    this.concurrency = concurrency;
    this.maxAttempts = maxAttempts;
    this.limit = limit;
    this.claimed = 0;
    this.startedAt = Date.now();
    this.lastProgressAt = 0;
    this.folderPromises = new Map();
    this.directoryContentsPromises = new Map();
    this.createdThisRun = new Set();
    this.counts = {
      imported: 0,
      existing: 0,
      missed: 0,
      conflict: 0,
      failed: 0,
    };
    this.selectNext = database.prepare(`
      SELECT path, folder_path, name, size, gcid, attempts
      FROM import_files
      WHERE
        status = 'pending'
        OR (status = 'failed' AND attempts < ?)
      ORDER BY path
      LIMIT 1
    `);
    this.claimRow = database.prepare(`
      UPDATE import_files
      SET status = 'processing', attempts = attempts + 1, error = NULL, updated_at = ?
      WHERE path = ? AND status IN ('pending', 'failed')
    `);
    this.finishRow = database.prepare(`
      UPDATE import_files
      SET status = ?, task_id = ?, file_id = ?, error = ?, updated_at = ?
      WHERE path = ?
    `);
    this.getFolder = database.prepare('SELECT file_id FROM folders WHERE path = ?');
    this.saveFolder = database.prepare(`
      INSERT INTO folders (path, file_id, created_at, updated_at)
      VALUES (?, ?, ?, ?)
      ON CONFLICT(path) DO UPDATE SET file_id = excluded.file_id, updated_at = excluded.updated_at
    `);
  }

  claimNext() {
    if (this.limit != null && this.claimed >= this.limit) return null;
    const row = this.selectNext.get(this.maxAttempts);
    if (!row) return null;
    const result = this.claimRow.run(unixTime(), row.path);
    if (Number(result.changes) === 0) return this.claimNext();
    this.claimed += 1;
    return { ...row, attempts: Number(row.attempts) + 1 };
  }

  finish(row, status, { taskId = null, fileId = null, error = null } = {}) {
    this.finishRow.run(status, taskId, fileId, error, unixTime(), row.path);
    if (Object.hasOwn(this.counts, status)) this.counts[status] += 1;
  }

  async listFolder(parentId, { foldersOnly = false } = {}) {
    const records = [];
    for (let page = 0; page < 1000; page += 1) {
      const response = await this.api.post('/userres/v1/file/get_file_list', {
        page,
        pageSize: 100,
        parentId,
        ...(foldersOnly ? { resType: 2 } : {}),
        orderBy: 0,
        sortType: 0,
        needSubFolderStat: true,
      });
      const list = Array.isArray(response.data?.list) ? response.data.list : [];
      records.push(...list);
      const total = Number(response.data?.total || records.length);
      if (list.length === 0 || records.length >= total) break;
    }
    return records;
  }

  async findFolder(parentId, name) {
    const records = await this.listFolder(parentId, { foldersOnly: true });
    const record = records.find((item) => Number(item.resType) === 2 && item.fileName === name);
    return record?.fileId ? String(record.fileId) : null;
  }

  async createOrFindFolder(parentId, name, logicalPath) {
    const response = await this.api.post(
      '/userres/v1/file/create_dir',
      { parentId, dirName: name, failIfNameExist: true },
      { allowedCodes: [159] },
    );
    const createdId = response.data?.fileId ? String(response.data.fileId) : null;
    if (Number(response.code || 0) === 0 && createdId) {
      this.createdThisRun.add(logicalPath);
      return createdId;
    }
    const existingId = await this.findFolder(parentId, name);
    if (!existingId) throw new Error(`无法创建或定位云端目录：${logicalPath || this.destination}`);
    return existingId;
  }

  async ensureDestination() {
    const cached = this.getFolder.get('');
    if (cached?.file_id) return String(cached.file_id);
    const existing = await this.findFolder('', this.destination);
    const fileId = existing || await this.createOrFindFolder('', this.destination, '');
    const now = unixTime();
    this.saveFolder.run('', fileId, now, now);
    return fileId;
  }

  ensureFolder(folderPath) {
    if (!folderPath) return this.ensureDestination();
    if (this.folderPromises.has(folderPath)) return this.folderPromises.get(folderPath);
    const promise = (async () => {
      const cached = this.getFolder.get(folderPath);
      if (cached?.file_id) return String(cached.file_id);
      const parts = folderPath.split('/');
      const name = parts.pop();
      const parentPath = parts.join('/');
      const parentId = await this.ensureFolder(parentPath);
      const fileId = await this.createOrFindFolder(parentId, name, folderPath);
      const now = unixTime();
      this.saveFolder.run(folderPath, fileId, now, now);
      return fileId;
    })();
    this.folderPromises.set(folderPath, promise);
    promise.catch(() => this.folderPromises.delete(folderPath));
    return promise;
  }

  directoryContents(folderPath, folderId) {
    if (this.directoryContentsPromises.has(folderPath)) {
      return this.directoryContentsPromises.get(folderPath);
    }
    const promise = (async () => {
      if (this.createdThisRun.has(folderPath)) return new Map();
      const records = await this.listFolder(folderId);
      return new Map(records.map((item) => [String(item.fileName || ''), item]));
    })();
    this.directoryContentsPromises.set(folderPath, promise);
    promise.catch(() => this.directoryContentsPromises.delete(folderPath));
    return promise;
  }

  async waitForTask(taskId, row) {
    const deadline = Date.now() + 10 * 60_000;
    let attempt = 0;
    while (Date.now() < deadline) {
      const response = await this.api.post(
        '/userres/v1/file/get_info_by_task_id',
        { taskId },
        { allowedCodes: [...TRANSIENT_TASK_CODES] },
      );
      if (response.data?.fileId) return String(response.data.fileId);
      if (!TRANSIENT_TASK_CODES.has(Number(response.code || 0)) && Number(response.code || 0) !== 0) {
        throw new Error(response.msg || `秒传任务失败：${taskId}`);
      }
      attempt += 1;
      await delay(Math.min(5_000, 500 * Math.max(1, Math.ceil(attempt / 5))));
    }
    throw new ApiError(`云端入库超过 10 分钟仍未完成：${row.path}`, { retryable: true });
  }

  async importRow(row) {
    const folderId = await this.ensureFolder(row.folder_path);
    const contents = await this.directoryContents(row.folder_path, folderId);
    const existing = contents.get(row.name);
    if (existing) {
      if (Number(existing.resType) !== 1) {
        this.finish(row, 'conflict', { error: '同名项是文件夹' });
        return;
      }
      const existingSize = Number(existing.fileSize ?? existing.size ?? -1);
      if (existingSize === Number(row.size)) {
        this.finish(row, 'existing', { fileId: String(existing.fileId || '') || null });
        return;
      }
      this.finish(row, 'conflict', {
        fileId: String(existing.fileId || '') || null,
        error: `同名文件大小不一致：云端 ${existingSize}，导入 ${row.size}`,
      });
      return;
    }

    let tokenResponse;
    try {
      tokenResponse = await this.api.post(
        '/userres/v1/get_res_center_token',
        {
          capacity: 2,
          name: row.name,
          res: { fileSize: Number(row.size) },
          parentId: folderId,
        },
        { allowedCodes: [156] },
      );
    } catch (error) {
      if (error.apiCode !== 159) throw error;
      this.directoryContentsPromises.delete(row.folder_path);
      const refreshed = await this.directoryContents(row.folder_path, folderId);
      const duplicate = refreshed.get(row.name);
      if (duplicate && Number(duplicate.fileSize ?? duplicate.size ?? -1) === Number(row.size)) {
        this.finish(row, 'existing', { fileId: String(duplicate.fileId || '') || null });
        return;
      }
      throw error;
    }

    let taskId = String(tokenResponse.data?.taskId || '');
    if (!taskId) throw new Error('光鸭没有返回上传任务 ID');
    let instant = Number(tokenResponse.code || 0) === 156;
    if (!instant) {
      const flash = await this.api.post('/userres/v1/check_can_flash_upload', {
        taskId,
        gcid: row.gcid,
      });
      instant = flash.data?.canFlashUpload === true;
      if (flash.data?.taskId) taskId = String(flash.data.taskId);
    }
    if (!instant) {
      this.finish(row, 'missed', { taskId, error: '光鸭未命中该 GCID，且本地没有源文件可普通上传' });
      return;
    }
    const fileId = await this.waitForTask(taskId, row);
    contents.set(row.name, {
      fileId,
      fileName: row.name,
      fileSize: Number(row.size),
      resType: 1,
    });
    this.finish(row, 'imported', { taskId, fileId });
  }

  progress({ force = false } = {}) {
    const now = Date.now();
    if (!force && now - this.lastProgressAt < 10_000) return;
    this.lastProgressAt = now;
    const summary = this.database.prepare(`
      SELECT status, COUNT(*) AS count
      FROM import_files
      GROUP BY status
      ORDER BY status
    `).all();
    const counts = Object.fromEntries(summary.map((item) => [item.status, Number(item.count)]));
    const finished = (counts.imported || 0)
      + (counts.existing || 0)
      + (counts.missed || 0)
      + (counts.conflict || 0);
    const elapsedSeconds = Math.max((now - this.startedAt) / 1000, 1);
    const rate = this.claimed / elapsedSeconds;
    const remaining = (counts.pending || 0) + (counts.processing || 0) + (counts.failed || 0);
    const etaSeconds = rate > 0 ? Math.round(remaining / rate) : null;
    console.log(JSON.stringify({
      type: 'progress',
      at: new Date().toISOString(),
      finished,
      total: summary.reduce((sum, item) => sum + Number(item.count), 0),
      counts,
      ratePerSecond: Number(rate.toFixed(2)),
      etaSeconds,
    }));
  }

  async worker() {
    for (;;) {
      const row = this.claimNext();
      if (!row) return;
      try {
        await this.importRow(row);
      } catch (error) {
        this.finish(row, 'failed', { error: error.message || String(error) });
        console.error(JSON.stringify({
          type: 'file-error',
          at: new Date().toISOString(),
          path: row.path,
          attempt: row.attempts,
          error: error.message || String(error),
        }));
      }
      this.progress();
    }
  }

  async run() {
    const destinationId = await this.ensureDestination();
    console.log(JSON.stringify({
      type: 'started',
      at: new Date().toISOString(),
      destination: this.destination,
      destinationId,
      concurrency: this.concurrency,
      limit: this.limit,
    }));
    await Promise.all(Array.from({ length: this.concurrency }, () => this.worker()));
    this.progress({ force: true });
    return this.database.prepare(`
      SELECT status, COUNT(*) AS count
      FROM import_files
      GROUP BY status
      ORDER BY status
    `).all();
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const raw = await fs.readFile(options.input, 'utf8');
  const payload = JSON.parse(raw.replace(/^\uFEFF/, ''));
  const files = validateExport(payload);
  const statePath = options.stateDb || defaultStatePath(options.input, options.destination, payload);
  await fs.mkdir(path.dirname(statePath), { recursive: true });
  const database = initializeStateDatabase(statePath, files, {
    input: options.input,
    destination: options.destination,
    scriptVersion: payload.scriptVersion || '',
    exportVersion: payload.exportVersion || '',
    generatedAt: payload.generatedAt || '',
    totalFilesCount: files.length,
    totalSize: payload.totalSize || '',
  });
  const auth = new AuthProvider(options.authDb);
  try {
    const runner = new ImportRunner({
      api: new GuangyaApi(auth),
      database,
      destination: options.destination,
      concurrency: options.concurrency,
      maxAttempts: options.maxAttempts,
      limit: options.limit,
    });
    const summary = await runner.run();
    console.log(JSON.stringify({
      type: 'completed',
      at: new Date().toISOString(),
      statePath,
      summary: Object.fromEntries(summary.map((item) => [item.status, Number(item.count)])),
    }));
  } finally {
    auth.close();
    database.close();
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  main().catch((error) => {
    console.error(JSON.stringify({
      type: 'fatal',
      at: new Date().toISOString(),
      error: error.message || String(error),
    }));
    process.exitCode = 1;
  });
}
