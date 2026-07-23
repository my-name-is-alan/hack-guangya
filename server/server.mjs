import http from 'node:http';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
import { DatabaseSync } from 'node:sqlite';
import { fileURLToPath } from 'node:url';
import { pipeline } from 'node:stream/promises';
import chokidar from 'chokidar';
import OSS from 'ali-oss';
import { autoShareTargetFor, shareFilePayload, signHdhiveRequest } from './auto-share.mjs';
import { parseGuangyaShareLink } from '../ui/shareLink.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const uiRoot = path.resolve(here, '..', 'dist');
const port = Number(process.env.PORT || 8080);
const dataDir = path.resolve(process.env.DATA_DIR || path.join(here, '..', '.web-data'));
const watchRoot = path.resolve(process.env.GUANGYA_WATCH_ROOT || path.join(here, '..', 'watch'));
const archiveRoot = path.resolve(process.env.GUANGYA_ARCHIVE_ROOT || path.join(here, '..', 'archive'));
const fileRoots = (process.env.GUANGYA_FILE_ROOTS || watchRoot).split(',').map((value) => path.resolve(value.trim())).filter(Boolean);
const configFile = path.join(dataDir, 'config.json');
const databaseFile = path.join(dataDir, 'state.sqlite3');
const manualUploadRoot = path.join(dataDir, 'manual-uploads');
const apiBase = process.env.GUANGYA_API_BASE || 'https://api.guangyapan.com';
const accountBase = process.env.GUANGYA_ACCOUNT_BASE || 'https://account.guangyapan.com';
const oauthClientId = 'aMe-8VSlkrbQXpUR';
function envInteger(name, fallback, minimum, maximum) { const parsed = Number(process.env[name]); return Number.isFinite(parsed) ? Math.max(minimum, Math.min(maximum, Math.round(parsed))) : fallback; }
const ossTimeoutMs = envInteger('GUANGYA_OSS_TIMEOUT_MS', 600_000, 120_000, 3_600_000);
const ossRetryMax = envInteger('GUANGYA_OSS_RETRY_MAX', 3, 0, 10);
const ossParallel = envInteger('GUANGYA_OSS_PARALLEL', 3, 1, 8);
const cloudConfirmTimeoutMs = envInteger('GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS', 600_000, 1_000, 3_600_000);
const cloudConfirmPollMs = envInteger('GUANGYA_CLOUD_CONFIRM_POLL_MS', 1_000, 10, 5_000);
const autoShareQuietMs = envInteger('GUANGYA_AUTO_SHARE_QUIET_MS', 30_000, 1_000, 600_000);
const tokenRefreshIntervalMs = envInteger('GUANGYA_TOKEN_REFRESH_MS', 20 * 60_000, 60_000, 60 * 60_000);
let hdhiveBaseUrl = String(process.env.HDHIVE_BASE_URL || '').trim().replace(/\/$/, '');
let hdhiveSecret = String(process.env.HDHIVE_GUANGYA_SYNC_SECRET || '').trim();
fs.mkdirSync(dataDir, { recursive: true });
const protectedDataRoot = fs.realpathSync(dataDir);
const database = new DatabaseSync(databaseFile);
database.exec(`
  PRAGMA journal_mode = WAL;
  PRAGMA synchronous = NORMAL;
  CREATE TABLE IF NOT EXISTS auth_session (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    access_token TEXT,
    refresh_token TEXT,
    updated_at INTEGER NOT NULL
  );
  CREATE TABLE IF NOT EXISTS uploaded_files (
    mapping_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    modified_ms TEXT NOT NULL,
    task_id TEXT,
    remote_file_id TEXT,
    uploaded_at INTEGER NOT NULL,
    PRIMARY KEY (mapping_id, file_path)
  );
  CREATE TABLE IF NOT EXISTS app_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
  );
  CREATE TABLE IF NOT EXISTS auto_share_targets (
    mapping_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    target_type TEXT NOT NULL,
    remote_target_id TEXT NOT NULL,
    title TEXT NOT NULL,
    share_id TEXT NOT NULL,
    share_url TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (mapping_id, target_key)
  );
  CREATE TABLE IF NOT EXISTS auto_share_events (
    event_id TEXT PRIMARY KEY,
    mapping_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    share_url TEXT,
    status TEXT NOT NULL,
    action TEXT,
    message TEXT,
    resource_url TEXT,
    notification_status TEXT,
    payload TEXT NOT NULL,
    updated_at INTEGER NOT NULL
  );
  CREATE TABLE IF NOT EXISTS auto_share_pending (
    mapping_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    target_type TEXT NOT NULL,
    title TEXT NOT NULL,
    remote_target_id TEXT NOT NULL,
    added_paths TEXT NOT NULL,
    changed_paths TEXT NOT NULL,
    event_id TEXT NOT NULL,
    retry_count INTEGER NOT NULL DEFAULT 0,
    due_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (mapping_id, target_key)
  );
  CREATE TABLE IF NOT EXISTS auto_share_failures (
    mapping_id TEXT NOT NULL,
    target_key TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    error TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (mapping_id, target_key, relative_path)
  );
`);
if (!database.prepare("PRAGMA table_info(auth_session)").all().some((column) => column.name === 'refresh_token')) database.exec('ALTER TABLE auth_session ADD COLUMN refresh_token TEXT');
if (!database.prepare("PRAGMA table_info(auto_share_events)").all().some((column) => column.name === 'notification_status')) database.exec('ALTER TABLE auto_share_events ADD COLUMN notification_status TEXT');
const storedDevice = database.prepare("SELECT value FROM app_state WHERE key = 'device_id'").get();
const deviceId = storedDevice?.value || crypto.randomUUID();
database.prepare("INSERT INTO app_state (key, value, updated_at) VALUES ('device_id', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at").run(deviceId, Math.floor(Date.now() / 1000));
const clients = new Set();
const watchers = new Map();
const queue = new Map();
const history = new Map(database.prepare('SELECT mapping_id, file_path, size, modified_ms FROM uploaded_files').all().map((row) => [`${row.mapping_id}::${path.resolve(row.file_path)}`, `${row.size}:${row.modified_ms}`]));
const inflight = new Map();
const inflightItems = new Map();
const waitingFiles = new Map();
const remoteCache = new Map([['', '']]);
const pendingAutoShares = new Map();
let mappings = [];
let savedShares = [];
const storedAuth = database.prepare('SELECT access_token, refresh_token FROM auth_session WHERE id = 1').get();
let token = process.env.GUANGYA_TOKEN || storedAuth?.access_token || null;
let refreshToken = storedAuth?.refresh_token || null;
let refreshPromise = null;
let paused = false;
let active = 0;
const fileStabilityMs = Math.max(200, Number(process.env.GUANGYA_FILE_STABILITY_MS || 1200));
const fileBusyRetryMs = Math.max(500, Number(process.env.GUANGYA_FILE_BUSY_RETRY_MS || 3000));
const storedInstance = database.prepare("SELECT value FROM app_state WHERE key = 'hdhive_instance_id'").get();
const hdhiveInstanceId = String(process.env.HDHIVE_GUANGYA_SYNC_INSTANCE_ID || storedInstance?.value || crypto.randomUUID());
database.prepare("INSERT INTO app_state (key, value, updated_at) VALUES ('hdhive_instance_id', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at").run(hdhiveInstanceId, Math.floor(Date.now() / 1000));
if (!hdhiveBaseUrl) hdhiveBaseUrl = database.prepare("SELECT value FROM app_state WHERE key = 'hdhive_base_url'").get()?.value || '';
if (!hdhiveSecret) hdhiveSecret = database.prepare("SELECT value FROM app_state WHERE key = 'hdhive_secret'").get()?.value || '';

const PRESET_EXTENSIONS = {
  image: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif', 'avif', 'tif', 'tiff', 'raw', 'cr2', 'nef', 'arw', 'dng'],
  video: ['mp4', 'mov', 'mkv', 'avi', 'wmv', 'flv', 'webm', 'm4v', 'ts', 'mts', 'm2ts', '3gp'],
  subtitle: ['srt', 'ass', 'ssa', 'vtt', 'sub', 'idx', 'sup', 'lrc'],
  audio: ['mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg', 'opus', 'wma', 'aiff'],
};
const DEFAULT_SYNC_TYPES = [...PRESET_EXTENSIONS.image, ...PRESET_EXTENSIONS.video, ...PRESET_EXTENSIONS.audio];

function normalizeRemote(value) { return String(value || '').replaceAll('\\', '/').split('/').filter(Boolean).join('/'); }
function normalizeSyncTypes(value) {
  const result = [];
  for (const item of Array.isArray(value) ? value : []) {
    const normalized = String(item).trim().replace(/^\./, '').toLowerCase();
    const preset = PRESET_EXTENSIONS[normalized];
    const values = preset || (/^[a-z0-9]{1,16}$/.test(normalized) ? [normalized] : []);
    for (const extension of values) if (!result.includes(extension)) result.push(extension);
  }
  return result.length ? result : [...DEFAULT_SYNC_TYPES];
}
function normalizeMonitorMode(value) { return String(value || '').toLowerCase() === 'polling' ? 'polling' : 'native'; }
function syncType(file) {
  const extension = path.extname(file).slice(1).toLowerCase();
  return extension;
}
function shouldSync(file, syncTypes) { const extension = syncType(file); return Boolean(extension) && normalizeSyncTypes(syncTypes).includes(extension); }
function queueKey(mappingId, file) { return `${mappingId}::${path.resolve(file)}`; }
function autoShareReceipts() { return database.prepare('SELECT event_id, mapping_id, target_key, share_url, status, action, message, resource_url, notification_status, updated_at FROM auto_share_events ORDER BY updated_at DESC LIMIT 50').all(); }
function state() { return { logged_in: Boolean(token), paused, pending: queue.size + waitingFiles.size, active_uploads: active, mappings, saved_shares: savedShares, hdhive: { configured: Boolean(hdhiveBaseUrl && hdhiveSecret), base_url: hdhiveBaseUrl, instance_id: hdhiveInstanceId }, auto_share_receipts: autoShareReceipts() }; }
function publish(payload) { const line = `data: ${JSON.stringify(payload)}\n\n`; for (const response of clients) response.write(line); }
function publishState() { publish({ type: 'state', state: state() }); }
function status(level, message) { publish({ type: 'status', level, message }); }
function json(response, code, payload) { response.writeHead(code, { 'content-type': 'application/json; charset=utf-8' }); response.end(JSON.stringify(payload)); }
async function readBody(request) { const chunks = []; for await (const chunk of request) chunks.push(chunk); return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {}; }
async function saveConfig() { await fsp.mkdir(dataDir, { recursive: true }); await fsp.writeFile(configFile, JSON.stringify({ mappings, saved_shares: savedShares }, null, 2)); }
function saveAuthSession(accessToken, nextRefreshToken = null) { database.prepare('INSERT INTO auth_session (id, access_token, refresh_token, updated_at) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET access_token = excluded.access_token, refresh_token = COALESCE(excluded.refresh_token, auth_session.refresh_token), updated_at = excluded.updated_at').run(accessToken || null, nextRefreshToken || null, Math.floor(Date.now() / 1000)); }
function replaceAuthSession(accessToken, nextRefreshToken = null) { database.prepare('INSERT INTO auth_session (id, access_token, refresh_token, updated_at) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET access_token = excluded.access_token, refresh_token = excluded.refresh_token, updated_at = excluded.updated_at').run(accessToken || null, nextRefreshToken || null, Math.floor(Date.now() / 1000)); }
function saveAuthToken(value) { saveAuthSession(value, null); }
function uploadHistoryPath(item) { return item.history_path || item.file_path; }
function uploadEventPath(item) { return item.event_path || item.file_path; }
function saveUploadRecord(item, taskData) { database.prepare('INSERT INTO uploaded_files (mapping_id, file_path, size, modified_ms, task_id, remote_file_id, uploaded_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(mapping_id, file_path) DO UPDATE SET size = excluded.size, modified_ms = excluded.modified_ms, task_id = excluded.task_id, remote_file_id = excluded.remote_file_id, uploaded_at = excluded.uploaded_at').run(item.mapping_id, path.resolve(uploadHistoryPath(item)), item.size, String(item.mtime), taskData.taskId || null, taskData.remoteFileId || null, Math.floor(Date.now() / 1000)); }
function deleteMappingHistory(mappingId) { database.prepare('DELETE FROM uploaded_files WHERE mapping_id = ?').run(mappingId); }
function isWithinRoot(root, candidate) { const relative = path.relative(root, candidate); return !relative.startsWith('..') && !path.isAbsolute(relative); }
function allowedPath(value) { const resolved = path.resolve(String(value || fileRoots[0])); if (!fileRoots.some((root) => isWithinRoot(root, resolved))) throw new Error(`路径超出允许范围：${fileRoots.join(', ')}`); if (isWithinRoot(dataDir, resolved)) throw new Error('应用状态目录不可浏览或上传'); return resolved; }
function allowedArchivePath(value) { return allowedPath(value || archiveRoot); }
async function resolveServerPath(value, expectedType = null) {
  const resolved = allowedPath(value || fileRoots[0]);
  const resolvedReal = await fsp.realpath(resolved);
  if (isWithinRoot(protectedDataRoot, resolvedReal)) throw new Error('应用状态目录不可浏览或上传');
  const rootReals = await Promise.all(fileRoots.map((root) => fsp.realpath(root)));
  if (!rootReals.some((root) => isWithinRoot(root, resolvedReal))) throw new Error('服务器文件路径超出允许范围');
  const stat = await fsp.stat(resolvedReal);
  if (expectedType === 'directory' && !stat.isDirectory()) throw new Error('服务器路径不是目录');
  if (expectedType === 'file' && !stat.isFile()) throw new Error('服务器路径不是文件');
  return { absolute: resolvedReal, stat, rootReals };
}
async function listServerDirectory(value) {
  const current = await resolveServerPath(value, 'directory');
  const entries = await fsp.readdir(current.absolute, { withFileTypes: true });
  const items = [];
  for (const entry of entries) {
    try {
      const child = await resolveServerPath(path.join(current.absolute, entry.name));
      if (!child.stat.isDirectory() && !child.stat.isFile()) continue;
      items.push({ name: entry.name, path: child.absolute, type: child.stat.isDirectory() ? 'directory' : 'file', size: child.stat.isFile() ? child.stat.size : null, modified_at: child.stat.mtimeMs });
    } catch {}
  }
  items.sort((left, right) => left.type === right.type ? left.name.localeCompare(right.name, 'zh-CN') : left.type === 'directory' ? -1 : 1);
  const atRoot = current.rootReals.some((root) => root === current.absolute);
  const parentCandidate = path.dirname(current.absolute);
  const parent = !atRoot && current.rootReals.some((root) => isWithinRoot(root, parentCandidate)) ? parentCandidate : '';
  return { roots: current.rootReals, path: current.absolute, display_path: current.absolute, parent, items };
}
async function collectServerUploadFiles(values) {
  const selected = Array.isArray(values) ? [...new Set(values.map((value) => String(value || '').trim()).filter(Boolean))] : [];
  if (!selected.length) throw new Error('请至少选择一个服务器文件或文件夹');
  const files = new Map();
  const visitedDirectories = new Set();
  const addDirectory = async (absolute, remoteBase) => {
    const current = await resolveServerPath(absolute, 'directory');
    if (visitedDirectories.has(current.absolute)) return;
    visitedDirectories.add(current.absolute);
    for (const entry of await fsp.readdir(current.absolute, { withFileTypes: true })) {
      let child;
      try { child = await resolveServerPath(path.join(current.absolute, entry.name)); } catch { continue; }
      if (child.stat.isDirectory()) await addDirectory(child.absolute, normalizeRemote(path.posix.join(remoteBase, entry.name)));
      else if (child.stat.isFile()) {
        const remoteDir = normalizeRemote(remoteBase);
        files.set(`${child.absolute}::${remoteDir}`, { absolute: child.absolute, remoteDir });
        if (files.size > 10_000) throw new Error('一次最多选择 10000 个服务器文件');
      }
    }
  };
  for (const value of selected) {
    const resolved = await resolveServerPath(value);
    if (resolved.stat.isFile()) files.set(`${resolved.absolute}::`, { absolute: resolved.absolute, remoteDir: '' });
    else if (resolved.stat.isDirectory()) await addDirectory(resolved.absolute, path.basename(value));
  }
  return [...files.values()];
}
async function queueServerUploads(values, parentId) {
  const files = await collectServerUploadFiles(values);
  let queued = 0;
  let skipped = 0;
  for (const file of files) {
    const stat = await fsp.stat(file.absolute);
    const destination = `${String(parentId || '')}::${file.remoteDir}`;
    const mappingId = `__manual__:${crypto.createHash('sha256').update(destination).digest('hex').slice(0, 20)}`;
    const item = { mapping_id: mappingId, file_path: file.absolute, remote_parent_id: String(parentId || ''), remote_dir: file.remoteDir, size: stat.size, mtime: stat.mtimeMs };
    const key = queueKey(mappingId, file.absolute);
    const stamp = `${item.size}:${item.mtime}`;
    if (history.get(key) === stamp || inflight.get(key) === stamp || (queue.has(key) && `${queue.get(key).size}:${queue.get(key).mtime}` === stamp) || waitingFiles.has(key)) { skipped += 1; continue; }
    queue.set(key, item);
    queued += 1;
    publish({ type: 'file', state: token ? 'queued' : 'waiting-login', file_path: item.file_path, mapping_id: mappingId });
  }
  pump();
  return { queued, skipped, total: files.length };
}
function ignore(file) { const base = path.basename(file).toLowerCase(); return base.startsWith('~$') || ['.tmp', '.part', '.crdownload', '.download', '.swp', '.ds_store'].some((suffix) => base.endsWith(suffix)); }
function headers() { if (!token) throw new Error('尚未设置光鸭会话令牌'); const trace = `${crypto.randomBytes(16).toString('hex')}-${crypto.randomBytes(8).toString('hex')}`; return { 'content-type': 'application/json', authorization: `Bearer ${token}`, dt: '4', did: deviceId, traceparent: `00-${trace}-01` }; }
async function parseResponse(response, endpoint) {
  const raw = await response.text();
  if (!raw.trim() && response.ok) return { code: 0, data: {} };
  try { return JSON.parse(raw.replace(/^\uFEFF/, '')); } catch (error) { throw new Error(`光鸭接口 ${endpoint} 返回了非 JSON 响应（HTTP ${response.status}）：${raw.slice(0, 240)}（${error.message}）`); }
}
async function apiPost(endpoint, body, allowed = [], allowRefresh = true) {
  const response = await fetch(`${apiBase}${endpoint}`, { method: 'POST', headers: headers(), body: JSON.stringify(body || {}), signal: AbortSignal.timeout(120000) });
  const payload = await parseResponse(response, endpoint);
  const code = Number(payload.code || 0);
  if (response.status === 401 || code === 117) {
    if (allowRefresh && refreshToken) {
      await refreshSavedSession();
      return apiPost(endpoint, body, allowed, false);
    }
    token = null;
    saveAuthToken(null);
    publishState();
    throw new Error('登录态已失效，且自动续期失败，请重新扫码登录');
  }
  if (!response.ok || (code !== 0 && !allowed.includes(code))) throw new Error(payload.msg || `光鸭接口失败 ${response.status}/${code}`);
  return payload;
}

async function listReceivedShareFiles(accessToken, parentId = '') {
  if (!String(accessToken || '').trim()) throw new Error('分享访问令牌为空，请重新打开分享链接');
  const items = [];
  let cursor;
  let total = 0;
  for (let page = 0; page < 100; page += 1) {
    const body = { pageSize: 100, accessToken, orderBy: 0, sortType: 0, parentId: String(parentId || '') };
    if (cursor != null) body.cursor = cursor;
    const response = await apiPost('/userres/v1/get_share_page_files_list', body);
    const data = response.data || {};
    const current = Array.isArray(data.list) ? data.list : [];
    total = Math.max(total, Number(data.total || 0));
    items.push(...current);
    const hasMore = typeof data.hasMore === 'boolean'
      ? data.hasMore
      : current.length === 100 && (!total || items.length < total);
    if (!hasMore || !current.length || (total && items.length >= total)) break;
    const nextCursor = Number(data.cursor ?? items.length);
    if (nextCursor === cursor) break;
    cursor = nextCursor;
  }
  return { list: items, total: Math.max(total, items.length), parentId: String(parentId || '') };
}

async function listAllShares() {
  const items = [];
  let total = 0;
  for (let page = 0; page < 100; page += 1) {
    const response = await apiPost('/userres/v1/get_share_list', { page, pageSize: 100, orderType: 1, sortType: 1 });
    const data = response.data || {};
    const current = Array.isArray(data.list) ? data.list : [];
    total = Math.max(total, Number(data.total || 0));
    items.push(...current);
    if (!current.length || current.length < 100 || (total && items.length >= total)) break;
  }
  return { list: items, total: Math.max(total, items.length) };
}

async function findExistingShareForFiles(fileIds) {
  const expected = [...new Set(fileIds.map(String))].sort();
  const shares = await listAllShares();
  for (const item of shares.list) {
    if (item.shareStatus != null && Number(item.shareStatus) !== 1) continue;
    const shareUrl = pickShareUrl(item);
    const shareId = String(shareIdFromUrl(shareUrl) || item.shareId || '');
    if (!shareId) continue;
    try {
      const access = await apiPost('/userres/v1/get_share_access_token', { shareId, code: String(item.code || '') });
      const accessToken = String(access.data?.accessToken || '');
      if (!accessToken) continue;
      const root = await listReceivedShareFiles(accessToken, '');
      const actual = [...new Set(root.list.map((file) => String(file.fileId || '')).filter(Boolean))].sort();
      if (actual.length === expected.length && actual.every((value, index) => value === expected[index])) return item;
    } catch {
      // 单个旧分享已失效或受限时继续检查其它分享，不阻止创建。
    }
  }
  return null;
}

async function openReceivedShare(value) {
  const parsed = parseGuangyaShareLink(value);
  const response = await apiPost('/userres/v1/get_share_access_token', { shareId: parsed.shareId, code: parsed.code });
  const accessToken = String(response.data?.accessToken || '');
  if (!accessToken) throw new Error('光鸭没有返回分享访问令牌');
  return { share_id: parsed.shareId, code: parsed.code, access_token: accessToken, files: await listReceivedShareFiles(accessToken, '') };
}

async function restoreReceivedShare(body) {
  const accessToken = String(body.access_token || '').trim();
  if (!accessToken) throw new Error('分享访问令牌为空，请重新打开分享链接');
  const response = await apiPost('/userres/v1/restore_share', { accessToken, fileIds: validateFileIds(body.file_ids), parentId: String(body.parent_id || '') });
  await waitOperation(response.data?.taskId);
  return response.data || {};
}

async function getReceivedShareDownload(body) {
  const accessToken = String(body.access_token || '').trim();
  const fileIds = validateFileIds(body.file_ids);
  const packaged = body.packaged === true;
  if (!accessToken) throw new Error('分享访问令牌为空，请重新打开分享链接');
  if (!packaged && fileIds.length !== 1) throw new Error('单文件下载只能选择一个文件');
  if (!packaged) {
    const response = await apiPost('/userres/v1/get_share_download_url', { fileId: fileIds[0], accessToken }, [205, 206, 207, 504]);
    if (Number(response.code || 0) !== 0) throw new Error(`当前分享下载受限，请到光鸭官方页面处理（业务码 ${response.code}：${response.msg || ''}）`);
    const downloadUrl = String(response.data?.downloadUrl || response.data?.downloadURL || '');
    if (!downloadUrl) throw new Error('光鸭没有返回文件下载地址');
    return { download_url: downloadUrl, mode: 'single' };
  }
  const response = await apiPost('/scheduler/v1/create_packaging_task', { fileIds, accessToken }, [205, 206, 207, 504]);
  if (Number(response.code || 0) !== 0) throw new Error(`当前批量下载受限，请到光鸭官方页面处理（业务码 ${response.code}：${response.msg || ''}）`);
  const taskId = String(response.data?.taskId || '');
  if (!taskId) throw new Error('光鸭没有返回压缩任务 ID');
  for (let attempt = 0; attempt < 600; attempt += 1) {
    const result = await apiPost('/scheduler/v1/query_packaging_task', { taskId, accessToken });
    const downloadUrl = String(result.data?.signedURL || result.data?.signedUrl || '');
    if (downloadUrl) return { download_url: downloadUrl, mode: 'packaged' };
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error('光鸭打包超过 10 分钟仍未完成，请稍后重试');
}

async function getCloudDownload(body) {
  const fileIds = validateFileIds(body.file_ids);
  const packaged = body.packaged === true;
  if (!packaged && fileIds.length !== 1) throw new Error('单文件下载只能选择一个文件');
  if (!packaged) {
    const response = await apiPost('/userres/v1/get_res_download_url', { fileId: fileIds[0] });
    const downloadUrl = String(response.data?.signedURL || response.data?.signedUrl || '');
    if (!downloadUrl) throw new Error('光鸭没有返回文件下载地址');
    return { download_url: downloadUrl, mode: 'single' };
  }
  const response = await apiPost('/scheduler/v1/create_packaging_task', { fileIds }, [205, 206, 207, 504]);
  if (Number(response.code || 0) !== 0) throw new Error(`当前批量下载受限（业务码 ${response.code}：${response.msg || ''}）`);
  const taskId = String(response.data?.taskId || '');
  if (!taskId) throw new Error('光鸭没有返回压缩任务 ID');
  for (let attempt = 0; attempt < 600; attempt += 1) {
    const result = await apiPost('/scheduler/v1/query_packaging_task', { taskId });
    const downloadUrl = String(result.data?.signedURL || result.data?.signedUrl || '');
    if (downloadUrl) return { download_url: downloadUrl, mode: 'packaged' };
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error('光鸭打包超过 10 分钟仍未完成，请稍后重试');
}

function autoShareTarget(item) {
  return autoShareTargetFor(item.relative_path, item.mapping_id);
}
function shareIdFromUrl(value) { try { const parsed = new URL(String(value || '')); return parsed.pathname.replace(/^\/s\//, '').replace(/^\/+|\/+$/g, ''); } catch { return ''; } }
function pickShareUrl(data) { return String(data?.shareUrl || data?.shareURL || data?.share_url || data?.url || '').trim(); }
function autoShareKey(mappingId, targetKey) { return `${mappingId}::${targetKey}`; }
function targetHasWork(mappingId, targetKey) {
  const matches = (item) => { const target = autoShareTarget(item); return item?.mapping_id === mappingId && target?.key === targetKey; };
  return [...queue.values()].some(matches) || [...inflightItems.values()].some(matches) || [...waitingFiles.values()].some(matches);
}
function targetHasFailures(mappingId, targetKey) {
  return Boolean(database.prepare('SELECT 1 FROM auto_share_failures WHERE mapping_id = ? AND target_key = ? LIMIT 1').get(mappingId, targetKey));
}
function persistPendingAutoShare(pending, delay) {
  const now = Date.now();
  const dueAt = now + delay;
  pending.dueAt = dueAt;
  database.prepare(`INSERT INTO auto_share_pending (mapping_id, target_key, target_type, title, remote_target_id, added_paths, changed_paths, event_id, retry_count, due_at, updated_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(mapping_id, target_key) DO UPDATE SET target_type=excluded.target_type, title=excluded.title, remote_target_id=excluded.remote_target_id, added_paths=excluded.added_paths, changed_paths=excluded.changed_paths, event_id=excluded.event_id, retry_count=excluded.retry_count, due_at=excluded.due_at, updated_at=excluded.updated_at`)
    .run(pending.mappingId, pending.targetKey, pending.targetType, pending.title, pending.remoteTargetId, JSON.stringify([...pending.added]), JSON.stringify([...pending.changed]), pending.eventId, pending.retryCount || 0, dueAt, now);
}
function removePendingAutoShare(mappingId, targetKey) {
  pendingAutoShares.delete(autoShareKey(mappingId, targetKey));
  database.prepare('DELETE FROM auto_share_pending WHERE mapping_id = ? AND target_key = ?').run(mappingId, targetKey);
}
function recordAutoShareFailure(item, error) {
  const target = autoShareTarget(item);
  const mapping = mappings.find((entry) => entry.id === item.mapping_id);
  if (!target || !mapping?.auto_share) return;
  database.prepare(`INSERT INTO auto_share_failures (mapping_id, target_key, relative_path, error, updated_at) VALUES (?, ?, ?, ?, ?)
    ON CONFLICT(mapping_id, target_key, relative_path) DO UPDATE SET error=excluded.error, updated_at=excluded.updated_at`)
    .run(item.mapping_id, target.key, target.relativePath, error.message, Date.now());
}
function clearAutoShareFailure(item) {
  const target = autoShareTarget(item);
  if (!target) return;
  database.prepare('DELETE FROM auto_share_failures WHERE mapping_id = ? AND target_key = ? AND relative_path = ?').run(item.mapping_id, target.key, target.relativePath);
}
function saveAutoShareEvent(eventId, mappingId, targetKey, shareUrl, statusValue, action, messageText, resourceUrl, payload) {
  database.prepare(`INSERT INTO auto_share_events (event_id, mapping_id, target_key, share_url, status, action, message, resource_url, payload, updated_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(event_id) DO UPDATE SET share_url=excluded.share_url, status=excluded.status, action=excluded.action, message=excluded.message, resource_url=excluded.resource_url, payload=excluded.payload, updated_at=excluded.updated_at`)
    .run(eventId, mappingId, targetKey, shareUrl || null, statusValue, action || null, messageText || null, resourceUrl || null, JSON.stringify(payload || {}), Math.floor(Date.now() / 1000));
  publishState();
}
function hdhiveSignature(method, pathname, bodyText, timestamp) {
  return signHdhiveRequest(hdhiveSecret, method, pathname, bodyText, timestamp);
}
async function hdhiveRequest(method, pathname, body = null) {
  if (!hdhiveBaseUrl || !hdhiveSecret) throw new Error('尚未配置 Hdhive 接入地址和密钥');
  const bodyText = body == null ? '' : JSON.stringify(body);
  const timestamp = String(Math.floor(Date.now() / 1000));
  const response = await fetch(`${hdhiveBaseUrl}${pathname}`, { method, headers: { 'content-type': 'application/json', 'X-GuangYa-Instance-Id': hdhiveInstanceId, 'X-GuangYa-Timestamp': timestamp, 'X-GuangYa-Signature': hdhiveSignature(method, pathname, bodyText, timestamp) }, body: body == null ? undefined : bodyText, signal: AbortSignal.timeout(30_000) });
  const raw = await response.text();
  let parsed = {};
  try { parsed = raw ? JSON.parse(raw) : {}; } catch { throw new Error(`Hdhive 返回非 JSON 响应（HTTP ${response.status}）：${raw.slice(0, 200)}`); }
  if (!response.ok) throw new Error(parsed.description || parsed.message || parsed.error || `Hdhive 请求失败 ${response.status}`);
  return parsed.data || parsed;
}
function hdhiveReceiptMessage(result) {
  if (String(result.error_message || '').trim()) return String(result.error_message).trim();
  if (result.status === 'completed') {
    const outcome = ({ created: '影巢投稿完成', updated: '影巢内容更新完成', no_change: '影巢确认内容没有变化', baseline_initialized: '影巢已建立内容基线' })[result.action] || '影巢处理完成';
    return result.notification_status === 'sent' ? `${outcome}，消息已推送` : outcome;
  }
  if (result.status === 'needs_review') return '影巢需要人工补充信息';
  if (result.status === 'failed') return '影巢处理失败，请重试';
  return result.status === 'accepted' ? '影巢已接收，等待处理' : '影巢正在解析并投稿';
}
async function pollHdhiveReceipt(eventId, mappingId, targetKey, shareUrl, payload) {
  const pathname = `/api/integrations/guangya-sync/events/${encodeURIComponent(eventId)}`;
  for (let attempt = 0; attempt < 60; attempt += 1) {
    await new Promise((resolve) => setTimeout(resolve, Math.min(2_000 + attempt * 500, 10_000)));
    try {
      const result = await hdhiveRequest('GET', pathname);
      saveAutoShareEvent(eventId, mappingId, targetKey, shareUrl, result.status || 'processing', result.action, hdhiveReceiptMessage(result), result.resource_url, payload);
      database.prepare('UPDATE auto_share_events SET notification_status = ?, updated_at = ? WHERE event_id = ?').run(result.notification_status || null, Math.floor(Date.now() / 1000), eventId);
      publishState();
      if (['completed', 'needs_review', 'failed'].includes(result.status)) return;
    } catch (error) {
      if (attempt === 59) saveAutoShareEvent(eventId, mappingId, targetKey, shareUrl, 'failed', '', `查询 Hdhive 回执失败：${error.message}`, '', payload);
    }
  }
}
async function createManualShare(body) {
  const fileIds = validateFileIds(body.file_ids);
  const title = String(body.title || '').trim() || '云盘分享';
  const targetType = body.target_type === 'folder' ? 'folder' : 'file';
  const existing = await findExistingShareForFiles(fileIds);
  const reusedExisting = Boolean(existing);
  const response = existing || await apiPost('/userres/v1/share_file', shareFilePayload(fileIds, title));
  const data = existing || response.data || response;
  const shareUrl = pickShareUrl(data);
  const shareId = String(shareIdFromUrl(shareUrl) || data.shareCode || data.share_code || data.shareId || data.shareID || data.share_id || '');
  if (!shareUrl || !shareId) throw new Error('光鸭没有返回完整分享链接');
  const eventId = crypto.randomUUID();
  const mappingId = '__manual__';
  const payload = {
    event_id: eventId,
    occurred_at: new Date().toISOString(),
    mapping_id: mappingId,
    target_key: title,
    target_type: targetType,
    remote_target_id: String(fileIds[0]),
    share_id: shareId,
    share_url: shareUrl,
    title,
    intent: reusedExisting ? 'update' : 'new',
    change_hint: { added: [], changed: [], removed: [] },
  };
  saveAutoShareEvent(eventId, mappingId, title, shareUrl, 'sending', '', reusedExisting ? '已复用光鸭分享，正在提交影巢更新' : '光鸭分享成功，正在提交影巢', '', payload);
  let hdhiveStatus = 'delivery_failed';
  let hdhiveMessage = '光鸭分享成功，但尚未提交 Hdhive';
  try {
    const accepted = await hdhiveRequest('POST', '/api/integrations/guangya-sync/events', payload);
    hdhiveStatus = accepted.status || 'accepted';
    hdhiveMessage = reusedExisting ? '影巢已接收，正在更新备注' : '影巢已接收，正在解析并投稿';
    saveAutoShareEvent(eventId, mappingId, title, shareUrl, hdhiveStatus, '', hdhiveMessage, '', payload);
    void pollHdhiveReceipt(eventId, mappingId, title, shareUrl, payload);
  } catch (error) {
    hdhiveMessage = `光鸭分享成功，但提交影巢失败：${error.message}`;
    saveAutoShareEvent(eventId, mappingId, title, shareUrl, hdhiveStatus, '', hdhiveMessage, '', payload);
  }
  return { ...data, share_id: shareId, share_url: shareUrl, reused_existing: reusedExisting, hdhive_event_id: eventId, hdhive_status: hdhiveStatus, hdhive_message: hdhiveMessage };
}
async function resolveAutoShareTarget(item, taskData, target) {
  if (target.type === 'file') {
    if (!taskData.remoteFileId) throw new Error('云端没有返回文件 ID，无法自动分享');
    return String(taskData.remoteFileId);
  }
  const mapping = mappings.find((entry) => entry.id === item.mapping_id);
  if (!mapping) throw new Error('备份任务已不存在');
  const remotePath = [mapping.remote_parent_id ? '' : mapping.remote_path, target.key].filter(Boolean).join('/');
  return ensureRemote(mapping.remote_parent_id || '', remotePath);
}
async function processAutoShare(pending) {
  const { mappingId, targetKey } = pending;
  if (targetHasWork(mappingId, targetKey)) { scheduleAutoShareTimer(pending); return; }
  if (targetHasFailures(mappingId, targetKey)) {
    saveAutoShareEvent(pending.eventId, mappingId, targetKey, '', 'waiting_upload', '', '同一分享目标仍有上传失败文件，已暂停分享', '', { target_key: targetKey });
    scheduleAutoShareTimer(pending, Math.max(autoShareQuietMs, 60_000));
    return;
  }
  const mapping = mappings.find((entry) => entry.id === mappingId);
  if (!mapping?.auto_share) { removePendingAutoShare(mappingId, targetKey); return; }
  try {
    const stored = database.prepare('SELECT * FROM auto_share_targets WHERE mapping_id = ? AND target_key = ?').get(mappingId, targetKey);
    let shareUrl = stored?.share_url || '';
    let shareId = stored?.share_id || '';
    if (shareIdFromUrl(shareUrl)) shareId = shareIdFromUrl(shareUrl);
    let intent = 'update';
    if (!stored || stored.remote_target_id !== pending.remoteTargetId || !shareUrl) {
      const existing = await findExistingShareForFiles([pending.remoteTargetId]);
      const reusedExisting = Boolean(existing);
      const response = existing || await apiPost('/userres/v1/share_file', shareFilePayload([pending.remoteTargetId], pending.title));
      const data = existing || response.data || response;
      shareUrl = pickShareUrl(data);
      shareId = String(shareIdFromUrl(shareUrl) || data.shareCode || data.share_code || data.shareId || data.shareID || data.share_id || '');
      if (!shareUrl || !shareId) throw new Error('光鸭没有返回完整分享链接');
      intent = reusedExisting || stored?.share_id === shareId ? 'update' : 'new';
      database.prepare(`INSERT INTO auto_share_targets (mapping_id, target_key, target_type, remote_target_id, title, share_id, share_url, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT(mapping_id, target_key) DO UPDATE SET target_type=excluded.target_type, remote_target_id=excluded.remote_target_id, title=excluded.title, share_id=excluded.share_id, share_url=excluded.share_url, updated_at=excluded.updated_at`)
        .run(mappingId, targetKey, pending.targetType, pending.remoteTargetId, pending.title, shareId, shareUrl, Math.floor(Date.now() / 1000));
      status('success', reusedExisting ? `已复用光鸭已有分享：${pending.title}` : `光鸭分享成功：${pending.title}`);
    }
    const eventId = pending.eventId;
    const payload = { event_id: eventId, occurred_at: new Date().toISOString(), mapping_id: mappingId, target_key: targetKey, target_type: pending.targetType, remote_target_id: pending.remoteTargetId, share_id: shareId, share_url: shareUrl, title: pending.title, intent, change_hint: { added: [...pending.added], changed: [...pending.changed], removed: [] } };
    saveAutoShareEvent(eventId, mappingId, targetKey, shareUrl, 'sending', '', '光鸭分享成功，正在通知 Hdhive', '', payload);
    const accepted = await hdhiveRequest('POST', '/api/integrations/guangya-sync/events', payload);
    saveAutoShareEvent(eventId, mappingId, targetKey, shareUrl, accepted.status || 'accepted', '', 'Hdhive 已接收', '', payload);
    void pollHdhiveReceipt(eventId, mappingId, targetKey, shareUrl, payload);
    removePendingAutoShare(mappingId, targetKey);
  } catch (error) {
    const eventId = pending.eventId || crypto.randomUUID();
    pending.eventId = eventId;
    pending.retryCount = (pending.retryCount || 0) + 1;
    saveAutoShareEvent(eventId, mappingId, targetKey, '', 'failed', '', error.message, '', { target_key: targetKey });
    if (pending.retryCount <= 8) scheduleAutoShareTimer(pending, Math.min(30_000 * (2 ** (pending.retryCount - 1)), 30 * 60_000));
  }
}
function scheduleAutoShareTimer(pending, delay = autoShareQuietMs) {
  if (pending.timer) clearTimeout(pending.timer);
  persistPendingAutoShare(pending, delay);
  pending.timer = setTimeout(() => { pending.timer = null; void processAutoShare(pending); }, delay);
}
async function scheduleAutoShare(item, taskData) {
  const mapping = mappings.find((entry) => entry.id === item.mapping_id);
  if (!mapping?.auto_share) return;
  const target = autoShareTarget(item);
  if (!target) return;
  const remoteTargetId = await resolveAutoShareTarget(item, taskData, target);
  const key = autoShareKey(item.mapping_id, target.key);
  let pending = pendingAutoShares.get(key);
  if (!pending) {
    pending = { mappingId: item.mapping_id, targetKey: target.key, targetType: target.type, title: target.title, remoteTargetId, added: new Set(), changed: new Set(), eventId: crypto.randomUUID(), retryCount: 0, timer: null };
    pendingAutoShares.set(key, pending);
  }
  pending.remoteTargetId = remoteTargetId;
  pending[item.change_kind === 'changed' ? 'changed' : 'added'].add(target.relativePath);
  scheduleAutoShareTimer(pending);
}
function restorePendingAutoShares() {
  for (const row of database.prepare('SELECT * FROM auto_share_pending').all()) {
    let added = [];
    let changed = [];
    try { added = JSON.parse(row.added_paths || '[]'); } catch {}
    try { changed = JSON.parse(row.changed_paths || '[]'); } catch {}
    const pending = { mappingId: row.mapping_id, targetKey: row.target_key, targetType: row.target_type, title: row.title, remoteTargetId: row.remote_target_id, added: new Set(added), changed: new Set(changed), eventId: row.event_id || crypto.randomUUID(), retryCount: Number(row.retry_count || 0), timer: null };
    pendingAutoShares.set(autoShareKey(row.mapping_id, row.target_key), pending);
    scheduleAutoShareTimer(pending, Math.max(1_000, Number(row.due_at || 0) - Date.now()));
  }
}
async function backfillAutoShares(mappingId) {
  const mapping = mappings.find((entry) => entry.id === mappingId);
  if (!mapping) throw new Error('备份任务不存在');
  if (!mapping.auto_share) throw new Error('请先开启该任务的自动分享');
  const rows = database.prepare('SELECT file_path, remote_file_id FROM uploaded_files WHERE mapping_id = ? AND remote_file_id IS NOT NULL AND remote_file_id <> ?').all(mappingId, '');
  let scheduled = 0;
  for (const row of rows) {
    const relative = path.relative(mapping.local_path, row.file_path).replaceAll('\\', '/');
    if (!relative || relative.startsWith('../') || path.isAbsolute(relative)) continue;
    const item = { mapping_id: mappingId, file_path: row.file_path, relative_path: relative, change_kind: 'added', remote_parent_id: mapping.remote_parent_id || '', remote_dir: '' };
    await scheduleAutoShare(item, { remoteFileId: row.remote_file_id });
    scheduled += 1;
  }
  return { scheduled };
}
async function retryAutoShareEvent(eventId, overrides) {
  const row = database.prepare('SELECT * FROM auto_share_events WHERE event_id = ?').get(eventId);
  if (!row) throw new Error('自动分享回执不存在');
  const payload = JSON.parse(row.payload || '{}');
  if (row.status === 'delivery_failed' && shareIdFromUrl(payload.share_url)) payload.share_id = shareIdFromUrl(payload.share_url);
  let result;
  let receiptMessage;
  if (row.status === 'delivery_failed') {
    result = await hdhiveRequest('POST', '/api/integrations/guangya-sync/events', payload);
    receiptMessage = 'Hdhive 已重新接收投稿事件';
  } else {
    const body = {};
    if (overrides?.tmdb_id) { body.tmdb_id = String(overrides.tmdb_id); body.media_type = String(overrides.media_type || ''); }
    result = await hdhiveRequest('POST', `/api/integrations/guangya-sync/events/${encodeURIComponent(eventId)}/retry`, body);
    receiptMessage = 'Hdhive 已重新接收';
  }
  saveAutoShareEvent(eventId, row.mapping_id, row.target_key, row.share_url, result.status || 'accepted', result.action, receiptMessage, result.resource_url, payload);
  void pollHdhiveReceipt(eventId, row.mapping_id, row.target_key, row.share_url, payload);
  return result;
}
async function accountGet(endpoint, allowRefresh = true) { if (!token) throw new Error('尚未设置光鸭会话令牌'); const response = await fetch(`${accountBase}${endpoint}`, { headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, signal: AbortSignal.timeout(120000) }); const payload = await parseResponse(response, endpoint); if (response.status === 401 && allowRefresh && refreshToken) { await refreshSavedSession(); return accountGet(endpoint, false); } if (!response.ok) throw new Error(payload.msg || `账号接口失败 ${response.status}`); return payload; }
async function accountPost(endpoint, body) { const response = await fetch(`${accountBase}${endpoint}`, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body || {}), signal: AbortSignal.timeout(120000) }); return { status: response.status, payload: await parseResponse(response, endpoint) }; }
function authValue(payload, key) { return payload?.[key] || payload?.data?.[key] || null; }
async function startDeviceLogin() {
  const { status: statusCode, payload } = await accountPost('/v1/auth/device/code', { scope: 'user', client_id: oauthClientId });
  if (statusCode >= 400) throw new Error(payload.error_description || payload.msg || '无法创建扫码登录任务');
  return payload.data || payload;
}
async function pollDeviceLogin(deviceCode) {
  if (!String(deviceCode || '').trim()) throw new Error('缺少扫码登录任务');
  const { status: statusCode, payload } = await accountPost('/v1/auth/token', { grant_type: 'urn:ietf:params:oauth:grant-type:device_code', device_code: deviceCode, client_id: oauthClientId });
  const accessToken = authValue(payload, 'access_token');
  const nextRefreshToken = authValue(payload, 'refresh_token');
  if (accessToken) {
    token = String(accessToken);
    if (nextRefreshToken) refreshToken = String(nextRefreshToken);
    remoteCache.clear();
    remoteCache.set('', '');
    saveAuthSession(token, refreshToken);
    status('success', '扫码登录成功，可以开始使用云盘和备份任务');
    publishState();
    pump();
    return { authenticated: true };
  }
  if ([400, 202, 428].includes(statusCode)) {
    const message = payload.error === 'authorization_pending' ? '等待扫码确认' : (payload.error_description === 'Precondition Required' ? '等待扫码确认' : payload.error_description || payload.msg || '等待扫码确认');
    return { pending: true, message };
  }
  throw new Error(payload.error_description || payload.msg || '扫码登录失败');
}
async function refreshSavedSession() {
  if (!refreshToken) return false;
  if (!refreshPromise) refreshPromise = (async () => {
    const { status: statusCode, payload } = await accountPost('/v1/auth/token', { grant_type: 'refresh_token', refresh_token: refreshToken, client_id: oauthClientId });
    if (statusCode >= 400) throw new Error(payload.error_description || payload.msg || '刷新登录状态失败');
    const accessToken = authValue(payload, 'access_token');
    const nextRefreshToken = authValue(payload, 'refresh_token');
    if (!accessToken) throw new Error('刷新登录状态时没有返回 access_token');
    token = String(accessToken);
    if (nextRefreshToken) refreshToken = String(nextRefreshToken);
    saveAuthSession(token, refreshToken);
    publishState();
    pump();
    return true;
  })().finally(() => { refreshPromise = null; });
  return refreshPromise;
}
async function findFolder(parentId, name) { for (let page = 0; page < 100; page += 1) { const response = await apiPost('/userres/v1/file/get_file_list', { page, pageSize: 100, parentId, resType: 2, needSubFolderStat: true }); const list = response.data?.list || []; const found = list.find((item) => item.resType === 2 && item.fileName === name); if (found?.fileId) return String(found.fileId); if (!list.length || (page + 1) * 100 >= Number(response.data?.total || 0)) break; } return null; }
async function ensureRemote(baseParentId, remotePath) { const normalized = normalizeRemote(remotePath); if (!normalized) return String(baseParentId || ''); let parentId = String(baseParentId || ''); let prefix = ''; for (const part of normalized.split('/')) { prefix = prefix ? `${prefix}/${part}` : part; const cacheKey = `${baseParentId || ''}::${prefix}`; if (remoteCache.has(cacheKey)) { parentId = remoteCache.get(cacheKey); continue; } const response = await apiPost('/userres/v1/file/create_dir', { parentId, dirName: part, failIfNameExist: true }, [159]); const fileId = response.data?.fileId || (response.code === 159 ? await findFolder(parentId, part) : null); if (!fileId) throw new Error(`无法创建远程目录 ${prefix}`); parentId = String(fileId); remoteCache.set(cacheKey, parentId); } return parentId; }
function isCloudIndexPendingMessage(message) { return /文件上传中|上传处理中|正在上传|正在处理|正在入库|任务处理中|任务未完成|稍后再试/.test(String(message || '')); }
async function waitTask(taskId, eventPath) {
  const deadline = Date.now() + cloudConfirmTimeoutMs;
  let attempt = 0;
  while (Date.now() < deadline) {
    try {
      const response = await apiPost('/userres/v1/file/get_info_by_task_id', { taskId }, [145, 146, 155, 163]);
      if (response.data?.fileId) return response.data;
    } catch (error) {
      if (!isCloudIndexPendingMessage(error.message)) throw error;
    }
    attempt += 1;
    publish({ type: 'progress', file_path: eventPath, percent: 100, bytes_per_second: 0, stage: '文件已上传，云端正在入库' });
    const delayMs = Math.min(cloudConfirmPollMs * Math.max(1, Math.ceil(attempt / 5)), 5_000, Math.max(0, deadline - Date.now()));
    if (delayMs > 0) await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  throw new Error(`云端入库超过 ${Math.round(cloudConfirmTimeoutMs / 1000)} 秒仍未完成，请稍后刷新云盘确认`);
}
async function waitOperation(taskId) { if (!taskId) return; for (let index = 0; index < 90; index += 1) { const response = await apiPost('/userres/v1/get_task_status', { taskId }); const statusCode = Number(response.data?.status); const detail = response.data?.detail || {}; if ([2, 3].includes(statusCode) && detail.code && Number(detail.code) !== 0) throw new Error(detail.msg || '文件操作失败'); if (statusCode === 2) return; if (statusCode === 3) throw new Error(detail.msg || '文件操作失败'); await new Promise((resolve) => setTimeout(resolve, 1000)); } throw new Error('文件操作长时间未完成'); }
function uploadPartSize(size) { if (size <= 100 * 1024 * 1024) return 1024 * 1024; if (size <= 1024 * 1024 * 1024) return 2 * 1024 * 1024; if (size <= 10 * 1024 * 1024 * 1024) return 4 * 1024 * 1024; return 8 * 1024 * 1024; }
function gcidChunkSize(size) { if (size <= 0x08000000) return 256 * 1024; if (size <= 0x10000000) return 512 * 1024; if (size <= 0x20000000) return 1024 * 1024; return 2 * 1024 * 1024; }
async function calculateFileHash(filePath, algorithm) {
  const hash = crypto.createHash(algorithm);
  const stream = fs.createReadStream(filePath, { highWaterMark: 2 * 1024 * 1024 });
  for await (const chunk of stream) hash.update(chunk);
  return hash.digest('hex');
}
async function calculateFileGcid(filePath, size, eventPath) {
  const handle = await fsp.open(filePath, 'r');
  const chunkSize = gcidChunkSize(size);
  const buffer = Buffer.allocUnsafe(chunkSize);
  const outer = crypto.createHash('sha1');
  let position = 0;
  try {
    while (position < size) {
      const length = Math.min(chunkSize, size - position);
      const { bytesRead } = await handle.read(buffer, 0, length, position);
      if (!bytesRead) break;
      outer.update(crypto.createHash('sha1').update(buffer.subarray(0, bytesRead)).digest());
      position += bytesRead;
      publish({ type: 'progress', file_path: eventPath, percent: 0, bytes_per_second: 0, stage: `正在计算秒传指纹 ${size ? Math.floor(position * 100 / size) : 100}%` });
    }
  } finally {
    await handle.close();
  }
  return outer.digest('hex').toUpperCase();
}
class FileBusyError extends Error {
  constructor() {
    super('另外的程序正在使用该文件，释放后将自动上传');
    this.name = 'FileBusyError';
  }
}
function isFileBusyError(error) {
  if (!error) return false;
  if (error instanceof FileBusyError) return true;
  const busyCodes = process.platform === 'win32' ? ['EBUSY', 'ETXTBSY', 'EPERM', 'EACCES'] : ['EBUSY', 'ETXTBSY'];
  return busyCodes.includes(error.code);
}
async function probeUploadFile(filePath) {
  let handle;
  try {
    handle = await fsp.open(filePath, 'r');
  } catch (error) {
    if (isFileBusyError(error)) throw new FileBusyError();
    throw error;
  } finally {
    await handle?.close();
  }
}
async function prepareUploadItem(item) {
  await probeUploadFile(item.file_path);
  const first = await fsp.stat(item.file_path);
  if (!first.isFile()) throw new Error('源路径不是文件');
  await new Promise((resolve) => setTimeout(resolve, fileStabilityMs));
  await probeUploadFile(item.file_path);
  const second = await fsp.stat(item.file_path);
  if (first.size !== second.size || first.mtimeMs !== second.mtimeMs) throw new FileBusyError();
  return { ...item, size: second.size, mtime: item.history_path ? item.mtime : second.mtimeMs };
}
function scheduleBusyUploadRetry(key, item) {
  waitingFiles.set(key, item);
  publish({ type: 'file', state: 'waiting-file', file_path: uploadEventPath(item), stage: '另外的程序正在使用该文件，释放后将自动上传' });
  publishState();
  setTimeout(async () => {
    waitingFiles.delete(key);
    try {
      const stat = await fsp.stat(item.file_path);
      if (!stat.isFile()) return;
      if (!item.mapping_id.startsWith('__') && !mappings.some((mapping) => mapping.id === item.mapping_id && mapping.enabled)) return;
      const refreshed = { ...item, size: stat.size, mtime: item.history_path ? item.mtime : stat.mtimeMs };
      const stamp = `${refreshed.size}:${refreshed.mtime}`;
      if (history.get(key) === stamp || inflight.get(key) === stamp || (queue.has(key) && `${queue.get(key).size}:${queue.get(key).mtime}` === stamp)) return;
      queue.set(key, refreshed);
      publish({ type: 'file', state: 'waiting-file', file_path: uploadEventPath(refreshed), stage: '另外的程序正在使用该文件，释放后将自动上传' });
    } catch {
      // 文件暂时消失时等待后续文件系统事件重新入队。
    } finally {
      publishState();
      pump();
    }
  }, fileBusyRetryMs);
}
async function upload(item) {
  const stat = await fsp.stat(item.file_path);
  const eventPath = uploadEventPath(item);
  publish({ type: 'progress', file_path: eventPath, percent: 0, stage: '正在准备云端目录' });
  const parentId = await ensureRemote(item.remote_parent_id || '', item.remote_dir);
  publish({ type: 'progress', file_path: eventPath, percent: 0, stage: '正在申请上传凭证' });
  const res = { fileSize: stat.size };
  if (stat.size < 1024 * 1024) {
    publish({ type: 'progress', file_path: eventPath, percent: 0, stage: '正在计算秒传 MD5' });
    res.md5 = await calculateFileHash(item.file_path, 'md5');
  }
  const response = await apiPost('/userres/v1/get_res_center_token', { capacity: 2, name: path.basename(item.file_path), res, parentId }, [156]);
  const data = response.data;
  if (!data?.taskId) throw new Error('光鸭没有返回上传任务 ID');
  let taskId = data.taskId;
  let instantUpload = response.code === 156;
  if (!instantUpload && stat.size >= 1024 * 1024) {
    try {
      const gcid = await calculateFileGcid(item.file_path, stat.size, eventPath);
      const flash = await apiPost('/userres/v1/check_can_flash_upload', { taskId, gcid });
      instantUpload = flash.data?.canFlashUpload === true;
      if (instantUpload && flash.data?.taskId) taskId = String(flash.data.taskId);
    } catch (error) {
      status('warning', `秒传校验失败，继续普通上传：${error.message}`);
    }
  }
  if (!instantUpload) {
    if (!data.creds || !data.objectPath) throw new Error('光鸭没有返回完整上传凭证');
    publish({ type: 'file', state: 'uploading', file_path: eventPath });
    publish({ type: 'progress', file_path: eventPath, percent: 0, stage: '正在连接 OSS' });
    const client = new OSS({
      region: data.region,
      accessKeyId: data.creds.accessKeyID,
      accessKeySecret: data.creds.secretAccessKey || data.creds.accessKeySecret,
      stsToken: data.creds.sessionToken,
      bucket: data.bucketName,
      endpoint: data.endPoint,
      secure: true,
      timeout: ossTimeoutMs,
      retryMax: ossRetryMax,
      requestErrorRetryHandle: () => {
        publish({ type: 'progress', file_path: eventPath, stage: 'OSS 分片超时，正在自动重试', bytes_per_second: 0 });
        return true;
      },
    });
    const uploadStartedAt = Date.now();
    try {
      await client.multipartUpload(data.objectPath, item.file_path, { partSize: uploadPartSize(stat.size), parallel: ossParallel, timeout: ossTimeoutMs, progress: (fraction) => { const normalized = Math.max(0, Math.min(1, Number(fraction) || 0)); const uploadedBytes = Math.round(normalized * stat.size); const elapsedSeconds = Math.max((Date.now() - uploadStartedAt) / 1000, 0.001); publish({ type: 'progress', file_path: eventPath, percent: Math.round(normalized * 100), bytes_per_second: uploadedBytes / elapsedSeconds, stage: '正在上传' }); } });
    } catch (error) {
      if (['ResponseTimeoutError', 'ConnectionTimeoutError'].includes(error.name)) throw new Error(`OSS 分片上传连续超时（单次 ${Math.round(ossTimeoutMs / 1000)} 秒，已自动重试 ${ossRetryMax} 次）：${error.message}`);
      throw error;
    }
  } else {
    publish({ type: 'progress', file_path: eventPath, percent: 100, stage: '已命中秒传' });
  }
  if (item.mapping_id) {
    const pendingTask = { taskId, remoteFileId: null };
    saveUploadRecord(item, pendingTask);
    history.set(queueKey(item.mapping_id, uploadHistoryPath(item)), `${item.size}:${item.mtime}`);
  }
  publish({ type: 'progress', file_path: eventPath, percent: 100, bytes_per_second: 0, stage: '已上传，正在等待云端入库' });
  publish({ type: 'file', state: 'processing', file_path: eventPath, stage: '已上传，正在等待云端入库' });
  let taskData;
  try { taskData = await waitTask(taskId, eventPath); }
  catch (error) { throw new Error(`文件已上传并已写入记录，不会重复上传；云端入库确认失败：${error.message}`); }
  return { taskId, remoteFileId: taskData?.fileId || null };
}
async function applySourcePolicy(item) { const mapping = mappings.find((entry) => entry.id === item.mapping_id); if (!mapping || mapping.source_policy === 'keep') return null; const stat = await fsp.stat(item.file_path); if (stat.size !== item.size || stat.mtimeMs !== item.mtime) throw new Error('上传期间源文件发生变化，已保留源文件且不会执行上传后策略'); if (mapping.source_policy === 'delete') { await fsp.rm(item.file_path); return '已按任务策略删除源文件'; } if (mapping.source_policy !== 'archive' || !mapping.archive_path) throw new Error('归档策略没有配置归档目录'); const relative = path.relative(mapping.local_path, item.file_path); let destination = path.join(mapping.archive_path, relative); await fsp.mkdir(path.dirname(destination), { recursive: true }); try { await fsp.access(destination); const parsed = path.parse(destination); destination = path.join(parsed.dir, `${parsed.name}-${Math.round(item.mtime)}${parsed.ext}`); } catch {} try { await fsp.rename(item.file_path, destination); } catch { await fsp.copyFile(item.file_path, destination); await fsp.rm(item.file_path); } return `已移动到归档目录：${destination}`; }
function pump() {
  if (paused || !token) { publishState(); return; }
  while (active < 2 && queue.size) {
    const [key, item] = queue.entries().next().value;
    queue.delete(key);
    inflight.set(key, `${item.size}:${item.mtime}`);
    inflightItems.set(key, item);
    active += 1;
    publish({ type: 'file', state: 'preparing', file_path: uploadEventPath(item) });
    let waitingForFile = false;
    prepareUploadItem(item).then((ready) => {
      Object.assign(item, ready);
      return upload(item);
    }).then(async (taskData) => {
      history.set(key, `${item.size}:${item.mtime}`);
      saveUploadRecord(item, taskData);
      clearAutoShareFailure(item);
      try { await scheduleAutoShare(item, taskData); } catch (error) { status('error', `文件已上传，但自动分享排队失败：${error.message}`); }
      const action = await applySourcePolicy(item);
      if (action) status('success', action);
      publish({ type: 'file', state: 'done', file_path: uploadEventPath(item) });
    }).catch((error) => {
      if (isFileBusyError(error)) {
        waitingForFile = true;
        scheduleBusyUploadRetry(key, item);
        return;
      }
      recordAutoShareFailure(item, error);
      console.error(`上传失败：${item.file_path}：${error.stack || error.message}`);
      publish({ type: 'file', state: 'error', file_path: uploadEventPath(item), error: error.message });
    }).finally(async () => {
      if (!waitingForFile && item.cleanup_path && isWithinRoot(manualUploadRoot, item.cleanup_path)) await fsp.rm(item.cleanup_path, { recursive: true, force: true });
      inflight.delete(key);
      inflightItems.delete(key);
      active -= 1;
      publishState();
      pump();
    });
  }
  publishState();
}
async function enqueue(mapping, file) {
  if (!mapping.enabled || ignore(file) || !shouldSync(file, mapping.sync_types)) return;
  let stat;
  try { stat = await fsp.stat(file); } catch { return; }
  if (!stat.isFile()) return;
  const key = queueKey(mapping.id, file);
  const mark = `${stat.size}:${stat.mtimeMs}`;
  if (history.get(key) === mark || inflight.get(key) === mark) return;
  if (waitingFiles.has(key)) return;
  const queued = queue.get(key);
  if (queued && `${queued.size}:${queued.mtime}` === mark) return;
  const relative = path.relative(mapping.local_path, file).replaceAll('\\', '/');
  const relativeDir = path.posix.dirname(relative) === '.' ? '' : path.posix.dirname(relative);
  queue.set(key, { mapping_id: mapping.id, file_path: file, relative_path: relative, change_kind: history.has(key) ? 'changed' : 'added', remote_parent_id: mapping.remote_parent_id || '', remote_dir: [mapping.remote_parent_id ? '' : mapping.remote_path, relativeDir].filter(Boolean).join('/'), size: stat.size, mtime: stat.mtimeMs });
  publish({ type: 'file', state: token ? 'queued' : 'waiting-login', file_path: file });
  pump();
}
async function collectExistingFiles(root, syncTypes) { const result = []; async function visit(current) { let entries = []; try { entries = await fsp.readdir(current, { withFileTypes: true }); } catch { return; } for (const entry of entries) { const file = path.join(current, entry.name); if (entry.isDirectory()) await visit(file); else if (entry.isFile() && !ignore(file) && shouldSync(file, syncTypes)) result.push(file); } } await visit(root); return result; }
async function enqueueDirectory(mapping, directory) { const files = await collectExistingFiles(directory, mapping.sync_types); for (const file of files) await enqueue(mapping, file); }
async function startWatcher(mapping) { await watchers.get(mapping.id)?.close(); watchers.delete(mapping.id); if (!mapping.enabled) return; const polling = mapping.monitor_mode === 'polling'; const watcher = chokidar.watch(mapping.local_path, { ignoreInitial: true, persistent: true, usePolling: polling, interval: polling ? 5000 : 100, binaryInterval: polling ? 5000 : 300, awaitWriteFinish: { stabilityThreshold: 1200, pollInterval: polling ? 1000 : 200 } }); watcher.on('add', (file) => enqueue(mapping, file)); watcher.on('change', (file) => enqueue(mapping, file)); watcher.on('addDir', (directory) => { void enqueueDirectory(mapping, directory); }); watcher.on('error', (error) => { mapping.watch_error = error.message; status('error', `监控失败：${error.message}`); }); watchers.set(mapping.id, watcher); await new Promise((resolve, reject) => { watcher.once('ready', resolve); watcher.once('error', reject); }); mapping.watch_error = null; if (mapping.scan_existing) { const existing = await collectExistingFiles(mapping.local_path, mapping.sync_types); status('info', `正在扫描已有文件：${existing.length} 个`); for (const file of existing) await enqueue(mapping, file); if (existing.length) publishState(); } }
async function restartWatchers() { for (const watcher of watchers.values()) await watcher.close(); watchers.clear(); for (const mapping of mappings) { if (!mapping.enabled) continue; try { await startWatcher(mapping); } catch (error) { mapping.enabled = false; mapping.watch_error = error.message; console.error(`备份任务监控启动失败：${mapping.local_path}：${error.message}`); } } await saveConfig(); }
async function routeApi(request, response, url) { if (request.method === 'GET' && url.pathname === '/api/state') return json(response, 200, state()); if (request.method === 'GET' && url.pathname === '/api/events') { response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' }); response.write(`data: ${JSON.stringify({ type: 'state', state: state() })}\n\n`); clients.add(response); request.on('close', () => clients.delete(response)); return; } if (request.method === 'POST' && url.pathname === '/api/auth') { const body = await readBody(request); token = String(body.token || '').trim().replace(/^Bearer\s+/i, '') || null; saveAuthToken(token); publishState(); pump(); return json(response, 200, state()); } if (request.method === 'POST' && url.pathname === '/api/mappings') { const body = await readBody(request); const mapping = { id: crypto.randomUUID(), local_path: allowedPath(body.local_path), remote_path: normalizeRemote(body.remote_path), enabled: true }; const stat = await fsp.stat(mapping.local_path); if (!stat.isDirectory()) throw new Error('监控路径不是目录'); mappings.push(mapping); await saveConfig(); await startWatcher(mapping); publishState(); return json(response, 200, mapping); } if (request.method === 'DELETE' && url.pathname.startsWith('/api/mappings/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); await watchers.get(id)?.close(); watchers.delete(id); mappings = mappings.filter((item) => item.id !== id); deleteMappingHistory(id); await saveConfig(); publishState(); return json(response, 200, {}); } if (request.method === 'PATCH' && url.pathname.startsWith('/api/mappings/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); const body = await readBody(request); const mapping = mappings.find((item) => item.id === id); if (!mapping) return json(response, 404, { error: '监控目录不存在' }); mapping.enabled = Boolean(body.enabled); await saveConfig(); await startWatcher(mapping); publishState(); return json(response, 200, mapping); } if (request.method === 'POST' && url.pathname === '/api/queue/pause') { paused = true; publishState(); return json(response, 200, state()); } if (request.method === 'POST' && url.pathname === '/api/queue/resume') { paused = false; pump(); return json(response, 200, state()); } json(response, 404, { error: 'not found' }); }
async function apiOverview() { const assets = await apiPost('/assets/v1/get_assets', {}); let profile = {}; try { profile = await accountGet('/v1/user/me'); } catch { try { profile = (await apiPost('/activity/v1/get_user_data', {})).data || {}; } catch {} } return { assets: assets.data || {}, profile: profile?.data || profile || {} }; }

function validateFileIds(fileIds) { if (!Array.isArray(fileIds) || !fileIds.length) throw new Error('请至少选择一个文件或文件夹'); return fileIds.map(String); }
async function renameRemote(fileId, newName) { await apiPost('/userres/v1/file/rename', { fileId, newName }); }
async function batchRename(renames) {
  const work = (Array.isArray(renames) ? renames : []).map((item) => ({ fileId: String(item.fileId || ''), currentName: String(item.currentName || ''), newName: String(item.newName || '') })).filter((item) => item.currentName !== item.newName);
  if (!work.length) throw new Error('没有需要重命名的项目');
  const seen = new Set();
  for (const item of work) { const name = item.newName.trim(); if (!name || /[\\/:*?"<>|]/.test(name)) throw new Error(`无效的文件名：${item.newName}`); const key = name.toLocaleLowerCase(); if (seen.has(key)) throw new Error(`存在重复目标名称：${name}`); seen.add(key); }
  const staged = work.map((item, index) => ({ item, temporary: `.__gy_tmp_${crypto.randomUUID().replaceAll('-', '')}_${index}` }));
  let stagedCount = 0;
  for (const entry of staged) { try { await renameRemote(entry.item.fileId, entry.temporary); stagedCount += 1; } catch (error) { for (const rollback of staged.slice(0, stagedCount).reverse()) { try { await renameRemote(rollback.item.fileId, rollback.item.currentName); } catch {} } throw new Error(`暂存重命名失败（${entry.item.currentName}）：${error.message}`); } }
  for (let index = 0; index < staged.length; index += 1) { const entry = staged[index]; try { await renameRemote(entry.item.fileId, entry.item.newName); } catch (error) { for (const rollback of staged.slice(0, index).reverse()) { try { await renameRemote(rollback.item.fileId, rollback.item.currentName); } catch {} } for (const rollback of staged.slice(index).reverse()) { try { await renameRemote(rollback.item.fileId, rollback.item.currentName); } catch {} } throw new Error(`目标重命名失败（${entry.item.newName}）：${error.message}`); } }
  return { renamed: staged.length };
}
async function handleWebUpload(request, response, url) {
  if (!token) throw new Error('请先登录光鸭云盘');
  const fileName = path.basename(url.searchParams.get('fileName') || 'upload.bin');
  const relativePath = normalizeRemote(url.searchParams.get('relativePath') || fileName);
  const parts = relativePath.split('/');
  if (!fileName || parts.some((part) => part === '.' || part === '..')) throw new Error('上传路径无效');
  const remoteDir = path.posix.dirname(relativePath) === '.' ? '' : path.posix.dirname(relativePath);
  const temporaryRoot = path.join(manualUploadRoot, crypto.randomUUID());
  const temporaryFile = path.join(temporaryRoot, fileName);
  let queued = false;
  await fsp.mkdir(temporaryRoot, { recursive: true });
  try {
    await pipeline(request, fs.createWriteStream(temporaryFile));
    const stat = await fsp.stat(temporaryFile);
    const parentId = url.searchParams.get('parentId') || '';
    const modified = Number(url.searchParams.get('lastModified')) || stat.mtimeMs;
    const mappingId = `__browser__:${crypto.createHash('sha256').update(`${parentId}::${remoteDir}`).digest('hex').slice(0, 20)}`;
    const item = { mapping_id: mappingId, file_path: temporaryFile, history_path: path.join(dataDir, 'browser-history', relativePath), event_path: `[浏览器]/${relativePath}`, remote_parent_id: parentId, remote_dir: remoteDir, size: stat.size, mtime: modified, cleanup_path: temporaryRoot };
    const historyKey = queueKey(mappingId, uploadHistoryPath(item));
    const stamp = `${item.size}:${item.mtime}`;
    const waiting = queue.get(historyKey);
    if (history.get(historyKey) === stamp || inflight.get(historyKey) === stamp || (waiting && `${waiting.size}:${waiting.mtime}` === stamp) || waitingFiles.has(historyKey)) return json(response, 200, { queued: 0, skipped: 1, fileName });
    queue.set(historyKey, item);
    queued = true;
    publish({ type: 'file', state: 'queued', file_path: uploadEventPath(item), mapping_id: mappingId });
    pump();
    return json(response, 202, { queued: 1, skipped: 0, fileName });
  } finally {
    if (!queued) await fsp.rm(temporaryRoot, { recursive: true, force: true });
  }
}
async function routeApiV2(request, response, url) {
  if (request.method === 'GET' && url.pathname === '/api/state') return json(response, 200, state());
  if (request.method === 'GET' && url.pathname === '/api/events') { response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' }); response.write(`data: ${JSON.stringify({ type: 'state', state: state() })}\n\n`); clients.add(response); request.on('close', () => clients.delete(response)); return; }
  if (request.method === 'POST' && url.pathname === '/api/auth/device/start') return json(response, 200, await startDeviceLogin());
  if (request.method === 'POST' && url.pathname === '/api/auth/device/poll') { const body = await readBody(request); return json(response, 200, await pollDeviceLogin(body.device_code)); }
  if (request.method === 'POST' && url.pathname === '/api/auth') { const body = await readBody(request); token = String(body.token || '').trim().replace(/^Bearer\s+/i, '') || null; refreshToken = null; replaceAuthSession(token, null); publishState(); pump(); return json(response, 200, state()); }
  if (request.method === 'GET' && url.pathname === '/api/overview') return json(response, 200, await apiOverview());
  if (request.method === 'GET' && url.pathname === '/api/files') return json(response, 200, await apiPost('/userres/v1/file/get_file_list', { page: Number(url.searchParams.get('page') || 0), pageSize: 100, parentId: url.searchParams.get('parentId') || '', orderBy: 0, sortType: 0, needSubFolderStat: true }));
  if (request.method === 'POST' && url.pathname === '/api/upload') return handleWebUpload(request, response, url);
  if (request.method === 'GET' && url.pathname === '/api/server-files') {
    if (!token) throw new Error('请先登录光鸭云盘');
    return json(response, 200, await listServerDirectory(url.searchParams.get('path') || ''));
  }
  if (request.method === 'POST' && url.pathname === '/api/server-upload') {
    if (!token) throw new Error('请先登录光鸭云盘');
    const body = await readBody(request);
    return json(response, 200, await queueServerUploads(body.paths, body.parent_id));
  }
  if (request.method === 'POST' && url.pathname === '/api/files/copy') { const body = await readBody(request); const result = await apiPost('/userres/v1/file/copy_file', { fileIds: validateFileIds(body.file_ids), parentId: String(body.parent_id || '') }); await waitOperation(result.data?.taskId); return json(response, 200, result.data || {}); }
  if (request.method === 'POST' && url.pathname === '/api/files/move') { const body = await readBody(request); const result = await apiPost('/userres/v1/file/move_file', { fileIds: validateFileIds(body.file_ids), parentId: String(body.parent_id || '') }); await waitOperation(result.data?.taskId); return json(response, 200, result.data || {}); }
  if (request.method === 'POST' && url.pathname === '/api/files/delete') { const body = await readBody(request); const result = await apiPost('/userres/v1/file/delete_file', { fileIds: validateFileIds(body.file_ids) }); await waitOperation(result.data?.taskId); return json(response, 200, result.data || {}); }
  if (request.method === 'POST' && url.pathname === '/api/files/rename-batch') { const body = await readBody(request); return json(response, 200, await batchRename(body.renames)); }
  if (request.method === 'POST' && url.pathname === '/api/files/download') { const body = await readBody(request); return json(response, 200, await getCloudDownload(body)); }
  if (request.method === 'POST' && url.pathname === '/api/share') { const body = await readBody(request); return json(response, 200, await createManualShare(body)); }
  if (request.method === 'GET' && url.pathname === '/api/shares') return json(response, 200, await listAllShares());
  if (request.method === 'POST' && url.pathname === '/api/shares/delete') { const body = await readBody(request); const ids = Array.isArray(body.ids) ? body.ids : []; if (!ids.length) throw new Error('请至少选择一个分享'); const result = await apiPost('/userres/v1/delete_share', { ids }); return json(response, 200, result.data || {}); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/open') { const body = await readBody(request); return json(response, 200, await openReceivedShare(body.url)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/files') { const body = await readBody(request); return json(response, 200, await listReceivedShareFiles(body.access_token, body.parent_id)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/restore') { const body = await readBody(request); return json(response, 200, await restoreReceivedShare(body)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/download') { const body = await readBody(request); return json(response, 200, await getReceivedShareDownload(body)); }
  if (request.method === 'GET' && url.pathname === '/api/offline') return json(response, 200, await apiPost('/cloudcollection/v1/list_task', { page: 0, pageSize: 100 }));
  if (request.method === 'POST' && url.pathname === '/api/offline') { const body = await readBody(request); return json(response, 200, await apiPost('/cloudcollection/v1/create_task', { url: body.url, parentId: body.parent_id || '', newName: body.new_name || '' })); }
  if (request.method === 'GET' && url.pathname === '/api/hdhive/config') return json(response, 200, state().hdhive);
  if (request.method === 'POST' && url.pathname === '/api/hdhive/config') { const body = await readBody(request); const nextBase = String(body.base_url || '').trim().replace(/\/$/, ''); if (nextBase && !/^https?:\/\//i.test(nextBase)) throw new Error('Hdhive 地址必须是完整的 HTTP(S) URL'); hdhiveBaseUrl = nextBase; if (typeof body.secret === 'string' && body.secret.trim()) hdhiveSecret = body.secret.trim(); database.prepare("INSERT INTO app_state (key, value, updated_at) VALUES ('hdhive_base_url', ?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at").run(hdhiveBaseUrl, Math.floor(Date.now()/1000)); database.prepare("INSERT INTO app_state (key, value, updated_at) VALUES ('hdhive_secret', ?, ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value, updated_at=excluded.updated_at").run(hdhiveSecret, Math.floor(Date.now()/1000)); publishState(); return json(response, 200, state().hdhive); }
  if (request.method === 'POST' && /^\/api\/auto-share\/events\/[^/]+\/retry$/.test(url.pathname)) { const body = await readBody(request); const eventId = decodeURIComponent(url.pathname.split('/')[4]); return json(response, 202, await retryAutoShareEvent(eventId, body)); }
  if (request.method === 'POST' && url.pathname === '/api/share-links') { const body = await readBody(request); const value = { id: crypto.randomUUID(), label: String(body.label || '未命名分享').trim() || '未命名分享', url: String(body.url || '').trim(), created_at: Math.floor(Date.now() / 1000) }; if (!/^https?:\/\//i.test(value.url)) throw new Error('分享链接必须以 http:// 或 https:// 开头'); savedShares.unshift(value); await saveConfig(); publishState(); return json(response, 200, value); }
  if (request.method === 'DELETE' && url.pathname.startsWith('/api/share-links/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); savedShares = savedShares.filter((item) => item.id !== id); await saveConfig(); publishState(); return json(response, 200, {}); }
  if (request.method === 'POST' && url.pathname === '/api/mappings') { const body = await readBody(request); const localPath = allowedPath(body.local_path); const sourcePolicy = ['keep', 'archive', 'delete'].includes(body.source_policy) ? body.source_policy : 'keep'; const archivePath = sourcePolicy === 'archive' ? allowedArchivePath(body.archive_path || archiveRoot) : null; if (archivePath && (archivePath === localPath || archivePath.startsWith(`${localPath}${path.sep}`))) throw new Error('归档目录不能位于被监控目录内部'); if (body.auto_share && (!hdhiveBaseUrl || !hdhiveSecret)) throw new Error('开启自动分享前请先配置 Hdhive 地址和密钥'); const mapping = { id: crypto.randomUUID(), local_path: localPath, remote_path: normalizeRemote(body.remote_path), remote_parent_id: String(body.remote_parent_id || ''), enabled: true, source_policy: sourcePolicy, archive_path: archivePath, scan_existing: body.scan_existing !== false, sync_types: normalizeSyncTypes(body.sync_types), monitor_mode: normalizeMonitorMode(body.monitor_mode), auto_share: body.auto_share === true, watch_error: null }; const stat = await fsp.stat(mapping.local_path); if (!stat.isDirectory()) throw new Error('监控路径不是目录'); mappings.push(mapping); await fsp.mkdir(archiveRoot, { recursive: true }); await saveConfig(); try { await startWatcher(mapping); } catch (error) { mappings = mappings.filter((item) => item.id !== mapping.id); await saveConfig(); throw new Error(`创建目录监控失败：${error.message}`); } publishState(); return json(response, 200, mapping); }
  if (request.method === 'DELETE' && url.pathname.startsWith('/api/mappings/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); await watchers.get(id)?.close(); watchers.delete(id); mappings = mappings.filter((item) => item.id !== id); for (const [key, item] of queue) if (item.mapping_id === id) queue.delete(key); for (const [key, item] of waitingFiles) if (item.mapping_id === id) waitingFiles.delete(key); for (const key of history.keys()) if (key.startsWith(`${id}::`)) history.delete(key); for (const key of inflight.keys()) if (key.startsWith(`${id}::`)) inflight.delete(key); deleteMappingHistory(id); await saveConfig(); publishState(); return json(response, 200, {}); }
  if (request.method === 'POST' && /^\/api\/mappings\/[^/]+\/auto-share-backfill$/.test(url.pathname)) { const id = decodeURIComponent(url.pathname.split('/')[3]); return json(response, 202, await backfillAutoShares(id)); }
  if (request.method === 'PATCH' && url.pathname.startsWith('/api/mappings/')) {
    const id = decodeURIComponent(url.pathname.split('/').pop());
    const body = await readBody(request);
    const mapping = mappings.find((item) => item.id === id);
    if (!mapping) return json(response, 404, { error: '监控目录不存在' });
    const monitorChanged = typeof body.monitor_mode === 'string';
    if (Array.isArray(body.sync_types)) {
      mapping.sync_types = normalizeSyncTypes(body.sync_types);
      for (const [key, item] of queue) if (item.mapping_id === id && !shouldSync(item.file_path, mapping.sync_types)) queue.delete(key);
    }
    if (monitorChanged) mapping.monitor_mode = normalizeMonitorMode(body.monitor_mode);
    if (typeof body.auto_share === 'boolean') { if (body.auto_share && (!hdhiveBaseUrl || !hdhiveSecret)) throw new Error('开启自动分享前请先配置 Hdhive 地址和密钥'); mapping.auto_share = body.auto_share; }
    if (typeof body.enabled === 'boolean') {
      mapping.enabled = body.enabled;
      if (!mapping.enabled) {
        await watchers.get(id)?.close();
        watchers.delete(id);
        mapping.watch_error = null;
      }
    }
    if (mapping.enabled && (body.enabled === true || monitorChanged)) {
      try { await startWatcher(mapping); }
      catch (error) { mapping.enabled = false; mapping.watch_error = error.message; await saveConfig(); throw new Error(`启动目录监控失败：${error.message}`); }
    } else if (Array.isArray(body.sync_types) && mapping.enabled && mapping.scan_existing) {
      const existing = await collectExistingFiles(mapping.local_path, mapping.sync_types);
      for (const file of existing) await enqueue(mapping, file);
    }
    await saveConfig();
    publishState();
    return json(response, 200, mapping);
  }
  if (request.method === 'POST' && url.pathname === '/api/queue/pause') { paused = true; publishState(); return json(response, 200, state()); }
  if (request.method === 'POST' && url.pathname === '/api/queue/resume') { paused = false; pump(); return json(response, 200, state()); }
  json(response, 404, { error: 'not found' });
}
async function serveStatic(response, url) { const requested = url.pathname === '/' ? '/index.html' : url.pathname; const file = path.resolve(uiRoot, `.${requested}`); if (!file.startsWith(uiRoot + path.sep)) return json(response, 403, { error: 'forbidden' }); try { const content = await fsp.readFile(file); const type = file.endsWith('.html') ? 'text/html; charset=utf-8' : file.endsWith('.js') ? 'text/javascript; charset=utf-8' : file.endsWith('.css') ? 'text/css; charset=utf-8' : file.endsWith('.svg') ? 'image/svg+xml' : 'application/octet-stream'; response.writeHead(200, { 'content-type': type }); response.end(content); } catch { json(response, 404, { error: 'not found' }); } }

await fsp.mkdir(dataDir, { recursive: true }); await fsp.rm(manualUploadRoot, { recursive: true, force: true }); await fsp.mkdir(manualUploadRoot, { recursive: true }); await fsp.mkdir(watchRoot, { recursive: true }); await fsp.mkdir(archiveRoot, { recursive: true });
try { const config = JSON.parse(await fsp.readFile(configFile, 'utf8')); mappings = Array.isArray(config.mappings) ? config.mappings.map((item) => ({ source_policy: 'keep', archive_path: null, scan_existing: true, remote_parent_id: '', sync_types: DEFAULT_SYNC_TYPES, monitor_mode: 'native', auto_share: false, watch_error: null, ...item, local_path: allowedPath(item.local_path), archive_path: item.archive_path ? allowedArchivePath(item.archive_path) : null, sync_types: normalizeSyncTypes(item.sync_types), monitor_mode: normalizeMonitorMode(item.monitor_mode), auto_share: item.auto_share === true })) : []; savedShares = Array.isArray(config.saved_shares) ? config.saved_shares : []; } catch { mappings = []; savedShares = []; }
await restartWatchers();
restorePendingAutoShares();
const server = http.createServer(async (request, response) => { const url = new URL(request.url, `http://${request.headers.host || 'localhost'}`); try { if (url.pathname.startsWith('/api/')) await routeApiV2(request, response, url); else await serveStatic(response, url); } catch (error) { json(response, 400, { error: error.message }); } });
server.listen(port, '0.0.0.0', async () => {
  console.log(`Guangya Web listening on http://0.0.0.0:${port}, file roots: ${fileRoots.join(', ')}, OSS timeout: ${ossTimeoutMs}ms, retries: ${ossRetryMax}, parallel: ${ossParallel}, cloud confirm timeout: ${cloudConfirmTimeoutMs}ms`);
  if (refreshToken) {
    try { await refreshSavedSession(); }
    catch (error) { status('warning', `已恢复上次登录，但刷新会话失败：${error.message}`); }
  }
  if (process.env.SELF_TEST === '1') {
    const response = await fetch(`http://127.0.0.1:${port}/api/state`);
    console.log(`SELF_TEST ${response.status} ${await response.text()}`);
    server.close();
    for (const watcher of watchers.values()) await watcher.close();
  }
});

setInterval(() => {
  if (!refreshToken) return;
  void refreshSavedSession().catch((error) => {
    status('warning', `自动续期失败，将稍后重试：${error.message}`);
  });
}, tokenRefreshIntervalMs);
