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
import { createAccessControl } from './access-control.mjs';
import { uploadPartSize } from './upload-parts.mjs';
import { parseGuangyaShareLink } from '../ui/shareLink.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const uiRoot = path.resolve(here, '..', 'dist');
const port = Number(process.env.PORT || 8080);
const adminUsername = String(process.env.GUANGYA_ADMIN_USERNAME || 'admin');
const adminPassword = String(process.env.GUANGYA_ADMIN_PASSWORD || '');
const requestedListenHost = String(process.env.LISTEN_HOST || process.env.HOST || (adminPassword ? '0.0.0.0' : '127.0.0.1')).trim();
const loopbackHosts = new Set(['127.0.0.1', '::1', 'localhost']);
if (!adminPassword && !loopbackHosts.has(requestedListenHost.toLowerCase())) throw new Error('未配置 GUANGYA_ADMIN_PASSWORD 时只允许监听回环地址');
const listenHost = requestedListenHost;
const configuredDataDir = path.resolve(process.env.DATA_DIR || path.join(here, '..', '.web-data'));
const configuredWatchRoot = path.resolve(process.env.GUANGYA_WATCH_ROOT || path.join(here, '..', 'watch'));
const configuredArchiveRoot = path.resolve(process.env.GUANGYA_ARCHIVE_ROOT || path.join(here, '..', 'archive'));
for (const directory of [configuredDataDir, configuredWatchRoot, configuredArchiveRoot]) fs.mkdirSync(directory, { recursive: true });
function canonicalizePathSync(value) {
  const resolved = path.resolve(String(value));
  let existing = resolved;
  while (!fs.existsSync(existing)) {
    const parent = path.dirname(existing);
    if (parent === existing) break;
    existing = parent;
  }
  const realExisting = fs.realpathSync.native ? fs.realpathSync.native(existing) : fs.realpathSync(existing);
  return path.resolve(realExisting, path.relative(existing, resolved));
}
const dataDir = canonicalizePathSync(configuredDataDir);
const watchRoot = canonicalizePathSync(configuredWatchRoot);
const archiveRoot = canonicalizePathSync(configuredArchiveRoot);
const fileRoots = (process.env.GUANGYA_FILE_ROOTS || watchRoot).split(',').map((value) => value.trim()).filter(Boolean).map(canonicalizePathSync);
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
const defaultUploadConcurrency = envInteger('GUANGYA_UPLOAD_CONCURRENCY', 2, 1, 8);
const defaultDownloadConcurrency = envInteger('GUANGYA_DOWNLOAD_CONCURRENCY', 2, 1, 8);
const defaultMonitorMode = String(process.env.GUANGYA_DEFAULT_MONITOR_MODE || 'native').toLowerCase() === 'polling' ? 'polling' : 'native';
const defaultCacheMaxEntries = 10_000;
const minCacheMaxEntries = 100;
const maxCacheMaxEntries = 100_000;
const cloudConfirmTimeoutMs = envInteger('GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS', 600_000, 1_000, 3_600_000);
const cloudConfirmPollMs = envInteger('GUANGYA_CLOUD_CONFIRM_POLL_MS', 1_000, 10, 5_000);
const autoShareQuietMs = envInteger('GUANGYA_AUTO_SHARE_QUIET_MS', 30_000, 1_000, 600_000);
const tokenRefreshIntervalMs = envInteger('GUANGYA_TOKEN_REFRESH_MS', 20 * 60_000, 60_000, 60 * 60_000);
const maxJsonBodyBytes = envInteger('GUANGYA_MAX_JSON_BODY_BYTES', 64 * 1024, 4 * 1024, 1024 * 1024);
const requestTimeoutMs = envInteger('GUANGYA_REQUEST_TIMEOUT_MS', 30_000, 5_000, 120_000);
const hdhiveAllowedHosts = new Set(String(process.env.HDHIVE_ALLOWED_HOSTS || '')
  .split(',').map((value) => value.trim().toLowerCase()).filter(Boolean));
function normalizeHdhiveBaseUrl(value) {
  const input = String(value || '').trim();
  if (!input) return '';
  let parsed;
  try { parsed = new URL(input); } catch { throw new Error('Hdhive 地址必须是完整的 HTTP(S) URL'); }
  if (!['http:', 'https:'].includes(parsed.protocol)) throw new Error('Hdhive 地址必须使用 HTTP 或 HTTPS');
  if (parsed.username || parsed.password || parsed.search || parsed.hash) throw new Error('Hdhive 地址不能包含账号、查询参数或片段');
  if (hdhiveAllowedHosts.size
    && !hdhiveAllowedHosts.has(parsed.host.toLowerCase())
    && !hdhiveAllowedHosts.has(parsed.hostname.toLowerCase())) throw new Error('Hdhive 地址不在 HDHIVE_ALLOWED_HOSTS 允许列表中');
  parsed.pathname = parsed.pathname.replace(/\/+$/, '');
  return parsed.toString().replace(/\/$/, '');
}
function hdhiveTargetUrl(pathname) {
  const target = new URL(hdhiveBaseUrl);
  target.pathname = `${target.pathname.replace(/\/+$/, '')}/${String(pathname).replace(/^\/+/, '')}`;
  target.search = '';
  target.hash = '';
  return target;
}
let hdhiveBaseUrl = normalizeHdhiveBaseUrl(process.env.HDHIVE_BASE_URL || '');
let hdhiveSecret = String(process.env.HDHIVE_GUANGYA_SYNC_SECRET || '').trim();
const protectedDataRoot = dataDir;
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
    status TEXT NOT NULL DEFAULT 'cloud_confirmed',
    item_json TEXT,
    remote_parent_id TEXT NOT NULL DEFAULT '',
    remote_dir TEXT NOT NULL DEFAULT '',
    relative_path TEXT NOT NULL DEFAULT '',
    uploaded_at INTEGER NOT NULL,
    PRIMARY KEY (mapping_id, file_path)
  );
  CREATE TABLE IF NOT EXISTS upload_checkpoints (
    mapping_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    size INTEGER NOT NULL,
    modified_ms TEXT NOT NULL,
    params_json TEXT NOT NULL,
    checkpoint_json TEXT NOT NULL,
    uploaded_bytes INTEGER NOT NULL DEFAULT 0,
    item_json TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (mapping_id, file_path)
  );
  CREATE TABLE IF NOT EXISTS app_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
  );
  CREATE TABLE IF NOT EXISTS file_fingerprints (
    file_path TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    modified_ms TEXT NOT NULL,
    gcid TEXT NOT NULL,
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
if (!database.prepare("PRAGMA table_info(uploaded_files)").all().some((column) => column.name === 'status')) {
  database.exec("ALTER TABLE uploaded_files ADD COLUMN status TEXT");
}
database.exec("UPDATE uploaded_files SET status = CASE WHEN remote_file_id IS NOT NULL AND remote_file_id <> '' THEN 'cloud_confirmed' ELSE 'oss_complete' END WHERE status IS NULL OR status NOT IN ('oss_complete', 'cloud_confirmed')");
if (!database.prepare("PRAGMA table_info(uploaded_files)").all().some((column) => column.name === 'item_json')) database.exec('ALTER TABLE uploaded_files ADD COLUMN item_json TEXT');
for (const [column, definition] of [
  ['remote_parent_id', "TEXT NOT NULL DEFAULT ''"],
  ['remote_dir', "TEXT NOT NULL DEFAULT ''"],
  ['relative_path', "TEXT NOT NULL DEFAULT ''"],
]) {
  if (!database.prepare("PRAGMA table_info(uploaded_files)").all().some((entry) => entry.name === column)) {
    database.exec(`ALTER TABLE uploaded_files ADD COLUMN ${column} ${definition}`);
  }
}
const accessControl = createAccessControl({
  database,
  initialCode: adminPassword,
  username: adminUsername,
});
function appStateValue(key) {
  return database.prepare('SELECT value FROM app_state WHERE key = ?').get(key)?.value;
}
function saveAppStateValue(key, value) {
  database.prepare('INSERT INTO app_state (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at')
    .run(key, String(value), Math.floor(Date.now() / 1000));
}
function storedConcurrency(key, fallback) {
  const value = Number(appStateValue(key));
  return Number.isInteger(value) && value >= 1 && value <= 8 ? value : fallback;
}
const multipartModes = new Set(['auto', '4m', '8m', '16m']);
let uploadConcurrency = storedConcurrency('transfer_upload_concurrency', defaultUploadConcurrency);
let downloadConcurrency = storedConcurrency('transfer_download_concurrency', defaultDownloadConcurrency);
let multipartMode = String(appStateValue('transfer_multipart') || 'auto').toLowerCase();
if (!multipartModes.has(multipartMode)) multipartMode = 'auto';
let cacheEnabled = appStateValue('cache_enabled') !== 'false';
const storedCacheMaxEntries = Number(appStateValue('cache_max_entries'));
let cacheMaxEntries = Number.isInteger(storedCacheMaxEntries)
  && storedCacheMaxEntries >= minCacheMaxEntries
  && storedCacheMaxEntries <= maxCacheMaxEntries
  ? storedCacheMaxEntries
  : defaultCacheMaxEntries;
let hdhiveEnabled = appStateValue('hdhive_enabled') !== 'false';
let hdhiveGeneration = 0;
saveAppStateValue('transfer_upload_concurrency', uploadConcurrency);
saveAppStateValue('transfer_download_concurrency', downloadConcurrency);
saveAppStateValue('transfer_multipart', multipartMode);
saveAppStateValue('cache_enabled', cacheEnabled);
saveAppStateValue('cache_max_entries', cacheMaxEntries);
saveAppStateValue('hdhive_enabled', hdhiveEnabled);
const storedDevice = database.prepare("SELECT value FROM app_state WHERE key = 'device_id'").get();
const deviceId = storedDevice?.value || crypto.randomUUID();
database.prepare("INSERT INTO app_state (key, value, updated_at) VALUES ('device_id', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at").run(deviceId, Math.floor(Date.now() / 1000));
const clients = new Set();
const watchers = new Map();
const queue = new Map();
const flashPreflightCache = new Map();
const history = new Map(database.prepare("SELECT mapping_id, file_path, size, modified_ms FROM uploaded_files WHERE status = 'cloud_confirmed'").all().map((row) => [`${row.mapping_id}::${path.resolve(row.file_path)}`, `${row.size}:${row.modified_ms}`]));
const pendingUploads = new Map(database.prepare("SELECT mapping_id, file_path, size, modified_ms, task_id, item_json, remote_parent_id, remote_dir, relative_path FROM uploaded_files WHERE status = 'oss_complete'").all().map((row) => [`${row.mapping_id}::${path.resolve(row.file_path)}`, row]));
const inflight = new Map();
const inflightItems = new Map();
const waitingFiles = new Map();
const remoteCache = new Map([['', '']]);
if (cacheEnabled) trimManagedCaches();
else clearManagedCaches();
const pendingAutoShares = new Map();
let mappings = [];
let savedShares = [];
const storedAuth = database.prepare('SELECT access_token, refresh_token FROM auth_session WHERE id = 1').get();
let token = process.env.GUANGYA_TOKEN || storedAuth?.access_token || null;
let refreshToken = storedAuth?.refresh_token || null;
let refreshPromise = null;
const smsChallenges = new Map();
let paused = false;
let active = 0;
let activeFlashPreflights = 0;
const flashPreflightConcurrency = 1;
const flashPreflightTokenMaxAgeMs = 10 * 60 * 1000;
const fileStabilityMs = Math.max(200, Number(process.env.GUANGYA_FILE_STABILITY_MS || 1200));
const fileBusyRetryMs = Math.max(500, Number(process.env.GUANGYA_FILE_BUSY_RETRY_MS || 3000));
const storedInstance = database.prepare("SELECT value FROM app_state WHERE key = 'hdhive_instance_id'").get();
const hdhiveInstanceId = String(process.env.HDHIVE_GUANGYA_SYNC_INSTANCE_ID || storedInstance?.value || crypto.randomUUID());
database.prepare("INSERT INTO app_state (key, value, updated_at) VALUES ('hdhive_instance_id', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at").run(hdhiveInstanceId, Math.floor(Date.now() / 1000));
if (!hdhiveBaseUrl) hdhiveBaseUrl = normalizeHdhiveBaseUrl(database.prepare("SELECT value FROM app_state WHERE key = 'hdhive_base_url'").get()?.value || '');
if (!hdhiveSecret) hdhiveSecret = database.prepare("SELECT value FROM app_state WHERE key = 'hdhive_secret'").get()?.value || '';

const PRESET_EXTENSIONS = {
  image: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif', 'avif', 'tif', 'tiff', 'raw', 'cr2', 'nef', 'arw', 'dng'],
  video: ['mp4', 'mov', 'mkv', 'avi', 'wmv', 'flv', 'webm', 'm4v', 'ts', 'mts', 'm2ts', '3gp'],
  subtitle: ['srt', 'ass', 'ssa', 'vtt', 'sub', 'idx', 'sup', 'lrc'],
  audio: ['mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg', 'opus', 'wma', 'aiff'],
};
const DEFAULT_SYNC_TYPES = [...PRESET_EXTENSIONS.image, ...PRESET_EXTENSIONS.video, ...PRESET_EXTENSIONS.audio];
const SEARCH_EXTENSION_GROUPS = {
  image: new Set(PRESET_EXTENSIONS.image),
  video: new Set(PRESET_EXTENSIONS.video),
  audio: new Set(PRESET_EXTENSIONS.audio),
  document: new Set(['pdf', 'doc', 'docx', 'xls', 'xlsx', 'ppt', 'pptx', 'txt', 'md', 'csv', 'rtf', 'odt', 'ods', 'odp', 'epub', 'mobi']),
  archive: new Set(['zip', 'rar', '7z', 'tar', 'gz', 'bz2', 'xz', 'tgz', 'iso']),
};
const SEARCH_FILE_TYPES = { image: 1, video: 2, audio: 3, document: 4, archive: 5 };
const SEARCH_TYPES = new Set([...Object.keys(SEARCH_EXTENSION_GROUPS), 'folder']);

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
function normalizeMonitorMode(value) {
  const normalized = String(value || '').toLowerCase();
  if (!normalized) return defaultMonitorMode;
  return normalized === 'polling' ? 'polling' : 'native';
}
function syncType(file) {
  const extension = path.extname(file).slice(1).toLowerCase();
  return extension;
}
function shouldSync(file, syncTypes) { const extension = syncType(file); return Boolean(extension) && normalizeSyncTypes(syncTypes).includes(extension); }
function queueKey(mappingId, file) { return `${mappingId}::${path.resolve(file)}`; }
function uploadStamp(item) { return `${item.size}:${item.mtime}`; }
function flashPreflightCached(key, item) {
  return flashPreflightCache.get(key)?.stamp === uploadStamp(item);
}
function takeFlashPreflightToken(key, item) {
  const cached = flashPreflightCache.get(key);
  flashPreflightCache.delete(key);
  if (!cached || cached.stamp !== uploadStamp(item) || Date.now() - cached.createdAt > flashPreflightTokenMaxAgeMs) return null;
  return cached.data || null;
}
function mappingAcceptsUpload(item) {
  return item.mapping_id.startsWith('__')
    || mappings.some((mapping) => mapping.id === item.mapping_id && mapping.enabled);
}
function prependQueuedItem(key, item) {
  const queued = [...queue.entries()];
  queue.clear();
  queue.set(key, item);
  for (const [queuedKey, queuedItem] of queued) queue.set(queuedKey, queuedItem);
}
function autoShareReceipts() { return database.prepare('SELECT event_id, mapping_id, target_key, share_url, status, action, message, resource_url, notification_status, updated_at FROM auto_share_events ORDER BY updated_at DESC LIMIT 50').all(); }
function transferSettings() {
  return {
    upload_concurrency: uploadConcurrency,
    download_concurrency: downloadConcurrency,
    multipart: multipartMode,
    multipart_part_size: multipartMode,
  };
}
function settingsState() {
  const transfer = transferSettings();
  return {
    ...transfer,
    transfer,
    cache: cacheSettings(),
    hdhive: {
      enabled: hdhiveEnabled,
      configured: Boolean(hdhiveBaseUrl && hdhiveSecret),
      base_url: hdhiveBaseUrl,
      instance_id: hdhiveInstanceId,
    },
  };
}
function validateConcurrency(value, label) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 8) throw new Error(`${label}必须是 1 到 8 之间的整数`);
  return parsed;
}
function updateTransferSettings(body) {
  if (Object.hasOwn(body, 'upload_concurrency')) uploadConcurrency = validateConcurrency(body.upload_concurrency, '上传并发数');
  if (Object.hasOwn(body, 'download_concurrency')) downloadConcurrency = validateConcurrency(body.download_concurrency, '下载并发数');
  const requestedMultipart = body.multipart ?? body.multipart_mode ?? body.multipart_part_size;
  if (requestedMultipart != null) {
    const normalized = String(requestedMultipart).toLowerCase();
    if (!multipartModes.has(normalized)) throw new Error('分片设置必须是 auto、4m、8m 或 16m');
    multipartMode = normalized;
  }
  saveAppStateValue('transfer_upload_concurrency', uploadConcurrency);
  saveAppStateValue('transfer_download_concurrency', downloadConcurrency);
  saveAppStateValue('transfer_multipart', multipartMode);
  publishState();
  pump();
  return transferSettings();
}
function cacheSettings() {
  return { enabled: cacheEnabled, max_entries: cacheMaxEntries };
}
function validateCacheMaxEntries(value) {
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minCacheMaxEntries || parsed > maxCacheMaxEntries) {
    throw new Error(`缓存条目上限必须是 ${minCacheMaxEntries} 到 ${maxCacheMaxEntries} 之间的整数`);
  }
  return parsed;
}
function trimFileFingerprintCache() {
  database.prepare(`DELETE FROM file_fingerprints
    WHERE file_path IN (
      SELECT file_path FROM file_fingerprints
      ORDER BY updated_at DESC, file_path DESC
      LIMIT -1 OFFSET ?
    )`).run(cacheMaxEntries);
}
function trimRemoteCache() {
  let excess = remoteCache.size - (remoteCache.has('') ? 1 : 0) - cacheMaxEntries;
  for (const key of remoteCache.keys()) {
    if (excess <= 0) break;
    if (key === '') continue;
    remoteCache.delete(key);
    excess -= 1;
  }
}
function trimManagedCaches() {
  trimFileFingerprintCache();
  trimRemoteCache();
}
function updateCacheSettings(body) {
  if (Object.hasOwn(body, 'enabled') && typeof body.enabled !== 'boolean') throw new Error('缓存开关必须是布尔值');
  if (Object.hasOwn(body, 'max_entries')) cacheMaxEntries = validateCacheMaxEntries(body.max_entries);
  if (Object.hasOwn(body, 'enabled')) cacheEnabled = body.enabled;
  saveAppStateValue('cache_enabled', cacheEnabled);
  saveAppStateValue('cache_max_entries', cacheMaxEntries);
  if (cacheEnabled) trimManagedCaches();
  else clearManagedCaches();
  return cacheSettings();
}
function cacheState() {
  const fingerprints = database.prepare('SELECT file_path, modified_ms, gcid FROM file_fingerprints').all();
  const fingerprintBytes = fingerprints.reduce((total, row) => total
    + Buffer.byteLength(String(row.file_path))
    + Buffer.byteLength(String(row.modified_ms))
    + Buffer.byteLength(String(row.gcid)) + 24, 0);
  const remoteEntries = [...remoteCache.entries()].filter(([key]) => key !== '');
  const remoteBytes = remoteEntries.reduce((total, [key, value]) => total
    + Buffer.byteLength(String(key)) + Buffer.byteLength(String(value)), 0);
  return {
    file_fingerprints: { entries: fingerprints.length, size_bytes: fingerprintBytes },
    remote_cache: { entries: remoteEntries.length, size_bytes: remoteBytes },
    entries: fingerprints.length + remoteEntries.length,
    bytes: fingerprintBytes + remoteBytes,
    file_fingerprints_entries: fingerprints.length,
    file_fingerprints_bytes: fingerprintBytes,
    remote_cache_entries: remoteEntries.length,
    remote_cache_bytes: remoteBytes,
    total_size_bytes: fingerprintBytes + remoteBytes,
    policy: cacheSettings(),
  };
}
function clearManagedCaches() {
  database.exec('DELETE FROM file_fingerprints');
  remoteCache.clear();
  remoteCache.set('', '');
  return cacheState();
}
function state() { return { logged_in: Boolean(token), paused, pending: queue.size + waitingFiles.size + pendingUploads.size, active_uploads: active, upload_concurrency: uploadConcurrency, download_concurrency: downloadConcurrency, multipart: multipartMode, multipart_part_size: multipartMode, mappings, saved_shares: savedShares, hdhive: { enabled: hdhiveEnabled, configured: Boolean(hdhiveBaseUrl && hdhiveSecret), base_url: hdhiveBaseUrl, instance_id: hdhiveInstanceId }, auto_share_receipts: autoShareReceipts() }; }
function publish(payload) { const line = `data: ${JSON.stringify(payload)}\n\n`; for (const response of clients) response.write(line); }
function publishState() { publish({ type: 'state', state: state() }); }
function status(level, message) { publish({ type: 'status', level, message }); }
function json(response, code, payload, headers = {}) { response.writeHead(code, { 'content-type': 'application/json; charset=utf-8', ...headers }); response.end(JSON.stringify(payload)); }
function enforceLoopbackHost(request, response) {
  if (adminPassword) return true;
  try {
    const hostname = new URL(`http://${request.headers.host || ''}`).hostname.toLowerCase();
    if (['localhost', '127.0.0.1', '[::1]'].includes(hostname)) return true;
  } catch {}
  json(response, 403, { error: '无密码本地模式仅接受回环 Host' });
  return false;
}
function hasSameOrigin(request) {
  const origin = request.headers.origin;
  if (!origin) return true;
  try {
    const originUrl = new URL(origin);
    const forwardedProtocol = String(request.headers['x-forwarded-proto'] || '').split(',')[0].trim();
    const protocol = forwardedProtocol || (request.socket.encrypted ? 'https' : 'http');
    return originUrl.origin === new URL(`${protocol}://${request.headers.host}`).origin;
  } catch { return false; }
}
function enforceMutationOrigin(request, response) {
  if (!['POST', 'PUT', 'PATCH', 'DELETE'].includes(request.method) || hasSameOrigin(request)) return true;
  json(response, 403, { error: '拒绝非同源的变更请求' });
  return false;
}
function httpError(statusCode, message, headers = {}) {
  const error = new Error(message);
  error.statusCode = statusCode;
  error.headers = headers;
  return error;
}
async function readBody(request, { maxBytes = maxJsonBodyBytes } = {}) {
  const contentLength = String(request.headers['content-length'] || '').trim();
  if (/^\d+$/.test(contentLength) && Number(contentLength) > maxBytes) {
    throw httpError(413, `请求体不能超过 ${maxBytes} 字节`, { connection: 'close' });
  }
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > maxBytes) throw httpError(413, `请求体不能超过 ${maxBytes} 字节`, { connection: 'close' });
    chunks.push(chunk);
  }
  if (!chunks.length) return {};
  try { return JSON.parse(Buffer.concat(chunks, size).toString('utf8')); }
  catch { throw httpError(400, '请求体必须是有效的 JSON'); }
}
async function saveConfig() { await fsp.mkdir(dataDir, { recursive: true }); await fsp.writeFile(configFile, JSON.stringify({ mappings, saved_shares: savedShares }, null, 2)); }
function saveAuthSession(accessToken, nextRefreshToken = null) { database.prepare('INSERT INTO auth_session (id, access_token, refresh_token, updated_at) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET access_token = excluded.access_token, refresh_token = COALESCE(excluded.refresh_token, auth_session.refresh_token), updated_at = excluded.updated_at').run(accessToken || null, nextRefreshToken || null, Math.floor(Date.now() / 1000)); }
function replaceAuthSession(accessToken, nextRefreshToken = null) { database.prepare('INSERT INTO auth_session (id, access_token, refresh_token, updated_at) VALUES (1, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET access_token = excluded.access_token, refresh_token = excluded.refresh_token, updated_at = excluded.updated_at').run(accessToken || null, nextRefreshToken || null, Math.floor(Date.now() / 1000)); }
function saveAuthToken(value) { saveAuthSession(value, null); }
function invalidateAuthSession() {
  token = null;
  refreshToken = null;
  replaceAuthSession(null, null);
  publishState();
}
function uploadHistoryPath(item) { return item.history_path || item.file_path; }
function uploadEventPath(item) { return item.event_path || item.file_path; }
function uploadCheckpointIdentity(item) {
  return [item.mapping_id, path.resolve(uploadHistoryPath(item))];
}
function clearUploadCheckpoint(item) {
  database.prepare('DELETE FROM upload_checkpoints WHERE mapping_id = ? AND file_path = ?')
    .run(...uploadCheckpointIdentity(item));
}
function loadUploadCheckpoint(item) {
  const [mappingId, filePath] = uploadCheckpointIdentity(item);
  const row = database.prepare('SELECT * FROM upload_checkpoints WHERE mapping_id = ? AND file_path = ?')
    .get(mappingId, filePath);
  if (!row) return null;
  if (Number(row.size) !== Number(item.size) || String(row.modified_ms) !== String(item.mtime)) {
    clearUploadCheckpoint(item);
    return null;
  }
  try {
    return {
      params: JSON.parse(row.params_json),
      checkpoint: JSON.parse(row.checkpoint_json),
      uploadedBytes: Number(row.uploaded_bytes || 0),
    };
  } catch {
    clearUploadCheckpoint(item);
    return null;
  }
}
function saveUploadCheckpoint(item, params, checkpoint, uploadedBytes) {
  const [mappingId, filePath] = uploadCheckpointIdentity(item);
  const resumableParams = {
    taskId: params?.taskId,
    objectPath: params?.objectPath,
    provider: params?.provider,
    bucketName: params?.bucketName,
    endPoint: params?.endPoint,
    region: params?.region,
  };
  const serializableCheckpoint = { ...(checkpoint || {}) };
  delete serializableCheckpoint.file;
  database.prepare(`
    INSERT INTO upload_checkpoints
      (mapping_id, file_path, size, modified_ms, params_json, checkpoint_json,
       uploaded_bytes, item_json, updated_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(mapping_id, file_path) DO UPDATE SET
      size = excluded.size,
      modified_ms = excluded.modified_ms,
      params_json = excluded.params_json,
      checkpoint_json = excluded.checkpoint_json,
      uploaded_bytes = excluded.uploaded_bytes,
      item_json = excluded.item_json,
      updated_at = excluded.updated_at
  `).run(
    mappingId,
    filePath,
    item.size,
    String(item.mtime),
    JSON.stringify(resumableParams),
    JSON.stringify(serializableCheckpoint),
    Math.max(0, Math.round(Number(uploadedBytes) || 0)),
    JSON.stringify(item),
    Math.floor(Date.now() / 1000),
  );
}
async function resumeUploadParams(params, fileSize) {
  if (!params?.taskId || !params?.objectPath) throw new Error('本地续传记录缺少云端任务信息');
  const response = await apiPost('/userres/v1/get_res_center_resume_token', {
    capacity: 2,
    res: { fileSize },
    taskId: params.taskId,
    object: { objectPath: params.objectPath, provider: params.provider },
  }, [156]);
  return { response, params: response.data || {} };
}
function restoreUploadCheckpoints() {
  const rows = database.prepare('SELECT mapping_id, file_path, size, modified_ms, item_json FROM upload_checkpoints ORDER BY updated_at').all();
  for (const row of rows) {
    try {
      const item = JSON.parse(row.item_json);
      const stat = fs.statSync(item.file_path);
      const unchanged = stat.isFile()
        && Number(stat.size) === Number(row.size)
        && Number(item.size) === Number(row.size)
        && String(item.mtime) === String(row.modified_ms);
      if (!unchanged) {
        database.prepare('DELETE FROM upload_checkpoints WHERE mapping_id = ? AND file_path = ?')
          .run(row.mapping_id, row.file_path);
        continue;
      }
      const key = queueKey(item.mapping_id, uploadHistoryPath(item));
      if (!queue.has(key) && !pendingUploads.has(key) && !history.has(key)) queue.set(key, item);
    } catch {
      // 源文件已不存在时保留 OSS 端分片到服务端自然过期，本地不再排队。
    }
  }
}
function savePendingUploadRecord(item, taskData) {
  const filePath = path.resolve(uploadHistoryPath(item));
  const itemJson = JSON.stringify(item);
  database.prepare("INSERT INTO uploaded_files (mapping_id, file_path, size, modified_ms, task_id, remote_file_id, status, item_json, remote_parent_id, remote_dir, relative_path, uploaded_at) VALUES (?, ?, ?, ?, ?, NULL, 'oss_complete', ?, ?, ?, ?, ?) ON CONFLICT(mapping_id, file_path) DO UPDATE SET size = excluded.size, modified_ms = excluded.modified_ms, task_id = excluded.task_id, remote_file_id = NULL, status = 'oss_complete', item_json = excluded.item_json, remote_parent_id = excluded.remote_parent_id, remote_dir = excluded.remote_dir, relative_path = excluded.relative_path, uploaded_at = excluded.uploaded_at").run(item.mapping_id, filePath, item.size, String(item.mtime), taskData.taskId || null, itemJson, item.remote_parent_id || '', item.remote_dir || '', item.relative_path || '', Math.floor(Date.now() / 1000));
  const key = queueKey(item.mapping_id, filePath);
  history.delete(key);
  pendingUploads.set(key, { mapping_id: item.mapping_id, file_path: filePath, size: item.size, modified_ms: String(item.mtime), task_id: taskData.taskId || null, item_json: itemJson, remote_parent_id: item.remote_parent_id || '', remote_dir: item.remote_dir || '', relative_path: item.relative_path || '' });
}
function confirmPendingUploadRecord(key, taskId, remoteFileId) {
  const row = pendingUploads.get(key);
  if (!row || String(row.task_id || '') !== String(taskId || '')) return null;
  const result = database.prepare("UPDATE uploaded_files SET remote_file_id = ?, status = 'cloud_confirmed', item_json = NULL, uploaded_at = ? WHERE mapping_id = ? AND file_path = ? AND status = 'oss_complete' AND task_id = ?").run(remoteFileId || null, Math.floor(Date.now() / 1000), row.mapping_id, row.file_path, taskId);
  if (Number(result.changes || 0) !== 1) return null;
  pendingUploads.delete(key);
  history.set(key, `${row.size}:${row.modified_ms}`);
  return row;
}
function clearPendingUpload(key) {
  const row = pendingUploads.get(key);
  if (!row) return;
  database.prepare("DELETE FROM uploaded_files WHERE mapping_id = ? AND file_path = ? AND status = 'oss_complete'").run(row.mapping_id, row.file_path);
  pendingUploads.delete(key);
}
function deleteMappingTransientUploads(mappingId) { database.prepare("DELETE FROM uploaded_files WHERE mapping_id = ? AND status <> 'cloud_confirmed'").run(mappingId); database.prepare('DELETE FROM upload_checkpoints WHERE mapping_id = ?').run(mappingId); for (const key of pendingUploads.keys()) if (key.startsWith(`${mappingId}::`)) pendingUploads.delete(key); }
function reuseMatchingConfirmedUpload(item) {
  const filePath = path.resolve(uploadHistoryPath(item));
  const candidates = database.prepare(`
    SELECT mapping_id, file_path, task_id, remote_file_id
    FROM uploaded_files
    WHERE status = 'cloud_confirmed'
      AND substr(mapping_id, 1, 2) <> '__'
      AND size = ?
      AND modified_ms = ?
      AND remote_parent_id = ?
      AND remote_dir = ?
      AND relative_path = ?
    ORDER BY uploaded_at DESC
    LIMIT 100
  `).all(item.size, String(item.mtime), item.remote_parent_id || '', item.remote_dir || '', item.relative_path || '');
  const canonicalFilePath = canonicalizePathSync(filePath);
  const matched = candidates.find((candidate) => {
    if (path.resolve(candidate.file_path) === filePath) return true;
    try { return canonicalizePathSync(candidate.file_path) === canonicalFilePath; }
    catch { return false; }
  });
  if (!matched) return null;
  database.prepare(`
    INSERT INTO uploaded_files
      (mapping_id, file_path, size, modified_ms, task_id, remote_file_id, status, item_json,
       remote_parent_id, remote_dir, relative_path, uploaded_at)
    VALUES (?, ?, ?, ?, ?, ?, 'cloud_confirmed', NULL, ?, ?, ?, ?)
    ON CONFLICT(mapping_id, file_path) DO UPDATE SET
      size = excluded.size,
      modified_ms = excluded.modified_ms,
      task_id = excluded.task_id,
      remote_file_id = excluded.remote_file_id,
      status = excluded.status,
      item_json = NULL,
      remote_parent_id = excluded.remote_parent_id,
      remote_dir = excluded.remote_dir,
      relative_path = excluded.relative_path,
      uploaded_at = excluded.uploaded_at
  `).run(item.mapping_id, filePath, item.size, String(item.mtime), matched.task_id || null, matched.remote_file_id || null, item.remote_parent_id || '', item.remote_dir || '', item.relative_path || '', Math.floor(Date.now() / 1000));
  history.set(queueKey(item.mapping_id, filePath), `${item.size}:${item.mtime}`);
  return { sourceMappingId: matched.mapping_id, taskId: matched.task_id || '', remoteFileId: matched.remote_file_id || null };
}
function reuseAutoShareBinding(item, sourceMappingId) {
  const target = autoShareTarget(item);
  if (!target) return false;
  const stored = database.prepare(`
    SELECT target_type, remote_target_id, title, share_id, share_url
    FROM auto_share_targets
    WHERE target_key = ? AND mapping_id IN (?, ?)
    ORDER BY CASE WHEN mapping_id = ? THEN 0 ELSE 1 END, updated_at DESC
    LIMIT 1
  `).get(target.key, item.mapping_id, sourceMappingId, item.mapping_id);
  if (!stored) return false;
  database.prepare(`
    INSERT INTO auto_share_targets
      (mapping_id, target_key, target_type, remote_target_id, title, share_id, share_url, updated_at)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
    ON CONFLICT(mapping_id, target_key) DO UPDATE SET
      target_type = excluded.target_type,
      remote_target_id = excluded.remote_target_id,
      title = excluded.title,
      share_id = excluded.share_id,
      share_url = excluded.share_url,
      updated_at = excluded.updated_at
  `).run(item.mapping_id, target.key, stored.target_type, stored.remote_target_id, stored.title, stored.share_id, stored.share_url, Math.floor(Date.now() / 1000));
  return true;
}
function isWithinRoot(root, candidate) { const relative = path.relative(root, candidate); return !relative.startsWith('..') && !path.isAbsolute(relative); }
function samePath(left, right) {
  const normalizedLeft = path.resolve(left);
  const normalizedRight = path.resolve(right);
  return process.platform === 'win32'
    ? normalizedLeft.toLowerCase() === normalizedRight.toLowerCase()
    : normalizedLeft === normalizedRight;
}
function allowedPath(value) { const resolved = canonicalizePathSync(String(value || fileRoots[0])); if (!fileRoots.some((root) => isWithinRoot(root, resolved))) throw new Error(`路径超出允许范围：${fileRoots.join(', ')}`); if (isWithinRoot(dataDir, resolved)) throw new Error('应用状态目录不可浏览或上传'); return resolved; }
function allowedArchivePath(value) { return allowedPath(value || archiveRoot); }
async function resolveMappingPath(mapping, value, expectedType = null) {
  const mappingRoot = await fsp.realpath(mapping.local_path);
  if (!samePath(mappingRoot, mapping.local_path)
    || !fileRoots.some((root) => isWithinRoot(root, mappingRoot))
    || isWithinRoot(protectedDataRoot, mappingRoot)) throw new Error('备份任务目录的真实路径已改变或超出允许范围');
  const absolute = await fsp.realpath(path.resolve(String(value)));
  if (!isWithinRoot(mappingRoot, absolute)
    || !fileRoots.some((root) => isWithinRoot(root, absolute))
    || isWithinRoot(protectedDataRoot, absolute)) throw new Error('备份源路径的真实位置超出任务目录');
  const stat = await fsp.stat(absolute);
  if (expectedType === 'directory' && !stat.isDirectory()) throw new Error('备份源路径不是目录');
  if (expectedType === 'file' && !stat.isFile()) throw new Error('备份源路径不是文件');
  const relative = path.relative(mappingRoot, absolute);
  if (path.isAbsolute(relative) || relative.startsWith('..')) throw new Error('备份源路径超出任务目录');
  return { absolute, mappingRoot, relative, stat };
}
function assertSourceIdentity(item, stat) {
  if (item.source_dev != null && Number(item.source_dev) !== Number(stat.dev)) throw new Error('备份源文件已被替换，已停止处理');
  if (item.source_ino != null && Number(item.source_ino) !== Number(stat.ino)) throw new Error('备份源文件已被替换，已停止处理');
}
async function validateWatchedSource(item) {
  if (!item?.mapping_id || String(item.mapping_id).startsWith('__')) {
    const stat = await fsp.stat(item.file_path);
    return { absolute: path.resolve(item.file_path), stat };
  }
  const mapping = mappings.find((entry) => entry.id === item.mapping_id);
  if (!mapping) throw new Error('备份任务已不存在');
  const source = await resolveMappingPath(mapping, item.file_path, 'file');
  const expectedRelative = String(item.relative_path || '').replaceAll('\\', '/');
  const actualRelative = source.relative.replaceAll('\\', '/');
  if (expectedRelative && expectedRelative !== actualRelative) throw new Error('备份源文件路径已被替换，已停止处理');
  assertSourceIdentity(item, source.stat);
  return source;
}
async function resolveServerPath(value, expectedType = null) {
  const resolved = allowedPath(value || fileRoots[0]);
  const resolvedReal = await fsp.realpath(resolved);
  if (isWithinRoot(protectedDataRoot, resolvedReal)) throw new Error('应用状态目录不可浏览或上传');
  const rootReals = fileRoots;
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
    if (history.get(key) === stamp || pendingUploads.has(key) || inflight.get(key) === stamp || (queue.has(key) && `${queue.get(key).size}:${queue.get(key).mtime}` === stamp) || waitingFiles.has(key)) { skipped += 1; continue; }
    queue.set(key, item);
    queued += 1;
    publish({ type: 'file', state: token ? 'queued' : 'waiting-login', file_path: item.file_path, mapping_id: mappingId, uploaded_bytes: 0, total_bytes: item.size });
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
    invalidateAuthSession();
    throw new Error('登录态已失效，且自动续期失败，请重新扫码登录');
  }
  if (!response.ok || (code !== 0 && !allowed.includes(code))) {
    const error = new Error(payload.msg || `光鸭接口失败 ${response.status}/${code}`);
    error.httpStatus = response.status;
    error.apiCode = code;
    error.retryable = response.status >= 500 || response.status === 429;
    throw error;
  }
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
  return [...queue.values()].some(matches)
    || [...inflightItems.values()].some(matches)
    || [...waitingFiles.values()].some(matches)
    || [...pendingUploads.values()].some(matches);
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
  if (!hdhiveEnabled) throw new Error('Hdhive 已关闭');
  if (!hdhiveBaseUrl || !hdhiveSecret) throw new Error('尚未配置 Hdhive 接入地址和密钥');
  const bodyText = body == null ? '' : JSON.stringify(body);
  const timestamp = String(Math.floor(Date.now() / 1000));
  const response = await fetch(hdhiveTargetUrl(pathname), { method, headers: { 'content-type': 'application/json', 'X-GuangYa-Instance-Id': hdhiveInstanceId, 'X-GuangYa-Timestamp': timestamp, 'X-GuangYa-Signature': hdhiveSignature(method, pathname, bodyText, timestamp) }, body: body == null ? undefined : bodyText, redirect: 'error', signal: AbortSignal.timeout(30_000) });
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
  if (!hdhiveEnabled) return;
  const generation = hdhiveGeneration;
  const pathname = `/api/integrations/guangya-sync/events/${encodeURIComponent(eventId)}`;
  for (let attempt = 0; attempt < 60; attempt += 1) {
    if (!hdhiveEnabled || generation !== hdhiveGeneration) return;
    await new Promise((resolve) => setTimeout(resolve, Math.min(2_000 + attempt * 500, 10_000)));
    if (!hdhiveEnabled || generation !== hdhiveGeneration) return;
    try {
      const result = await hdhiveRequest('GET', pathname);
      saveAutoShareEvent(eventId, mappingId, targetKey, shareUrl, result.status || 'processing', result.action, hdhiveReceiptMessage(result), result.resource_url, payload);
      database.prepare('UPDATE auto_share_events SET notification_status = ?, updated_at = ? WHERE event_id = ?').run(result.notification_status || null, Math.floor(Date.now() / 1000), eventId);
      publishState();
      if (['completed', 'needs_review', 'failed'].includes(result.status)) return;
    } catch (error) {
      if (!hdhiveEnabled || generation !== hdhiveGeneration) return;
      if (attempt === 59) saveAutoShareEvent(eventId, mappingId, targetKey, shareUrl, 'failed', '', `查询 Hdhive 回执失败：${error.message}`, '', payload);
    }
  }
}
async function createManualShare(body) {
  const fileIds = validateFileIds(body.file_ids);
  const title = String(body.title || '').trim() || '云盘分享';
  const targetType = body.target_type === 'folder' ? 'folder' : 'file';
  // 光鸭分享是创建时的内容快照。手动分享不复用旧链接，避免曾经
  // 在空目录阶段创建的分享一直显示为空。
  const response = await apiPost('/userres/v1/share_file', shareFilePayload(fileIds, title, body));
  const data = response.data || response;
  const reusedExisting = false;
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
  if (!hdhiveEnabled) {
    const hdhiveStatus = 'disabled';
    const hdhiveMessage = '光鸭分享成功，Hdhive 已关闭，未提交投稿';
    saveAutoShareEvent(eventId, mappingId, title, shareUrl, hdhiveStatus, '', hdhiveMessage, '', payload);
    return { ...data, share_id: shareId, share_url: shareUrl, reused_existing: reusedExisting, hdhive_event_id: eventId, hdhive_status: hdhiveStatus, hdhive_message: hdhiveMessage };
  }
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
  if (!hdhiveEnabled) {
    persistPendingAutoShare(pending, autoShareQuietMs);
    return;
  }
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
  if (!hdhiveEnabled) {
    pending.timer = null;
    return;
  }
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
function resumeHdhiveReceiptPolling() {
  if (!hdhiveEnabled) return;
  const rows = database.prepare("SELECT event_id, mapping_id, target_key, share_url, payload FROM auto_share_events WHERE status IN ('accepted', 'processing')").all();
  for (const row of rows) {
    let payload = {};
    try { payload = JSON.parse(row.payload || '{}'); } catch {}
    void pollHdhiveReceipt(row.event_id, row.mapping_id, row.target_key, row.share_url, payload);
  }
}
function setHdhiveEnabled(enabled) {
  const next = Boolean(enabled);
  if (next === hdhiveEnabled) {
    saveAppStateValue('hdhive_enabled', next);
    return;
  }
  hdhiveEnabled = next;
  hdhiveGeneration += 1;
  saveAppStateValue('hdhive_enabled', hdhiveEnabled);
  if (!hdhiveEnabled) {
    for (const pending of pendingAutoShares.values()) {
      if (pending.timer) clearTimeout(pending.timer);
      pending.timer = null;
    }
    return;
  }
  for (const pending of pendingAutoShares.values()) scheduleAutoShareTimer(pending, 1_000);
  resumeHdhiveReceiptPolling();
}
async function backfillAutoShares(mappingId) {
  const mapping = mappings.find((entry) => entry.id === mappingId);
  if (!mapping) throw new Error('备份任务不存在');
  if (!mapping.auto_share) throw new Error('请先开启该任务的自动分享');
  const rows = database.prepare("SELECT file_path, remote_file_id FROM uploaded_files WHERE mapping_id = ? AND status = 'cloud_confirmed' AND remote_file_id IS NOT NULL AND remote_file_id <> ?").all(mappingId, '');
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
async function accountGet(endpoint, allowRefresh = true) {
  if (!token) throw new Error('尚未设置光鸭会话令牌');
  const response = await fetch(`${accountBase}${endpoint}`, { headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, signal: AbortSignal.timeout(120000) });
  const payload = await parseResponse(response, endpoint);
  if (response.status === 401) {
    if (allowRefresh && refreshToken) {
      await refreshSavedSession();
      return accountGet(endpoint, false);
    }
    invalidateAuthSession();
    throw new Error('登录态已失效，且自动续期失败，请重新扫码登录');
  }
  if (!response.ok) throw new Error(payload.msg || `账号接口失败 ${response.status}`);
  return payload;
}
async function accountPost(endpoint, body, extraHeaders = {}) { const response = await fetch(`${accountBase}${endpoint}`, { method: 'POST', headers: { 'content-type': 'application/json', ...extraHeaders }, body: JSON.stringify(body || {}), signal: AbortSignal.timeout(120000) }); return { status: response.status, payload: await parseResponse(response, endpoint) }; }
function authValue(payload, key) { return payload?.[key] ?? payload?.data?.[key] ?? null; }
function accountError(payload, fallback) { return payload?.error_description || payload?.description || payload?.msg || payload?.message || payload?.error || fallback; }
function normalizeChinesePhone(value) {
  let normalized = String(value || '').trim().replace(/[\s()-]/g, '');
  if (normalized.startsWith('+86')) normalized = normalized.slice(3);
  else if (normalized.startsWith('86') && normalized.length === 13) normalized = normalized.slice(2);
  if (!/^1[3-9]\d{9}$/.test(normalized)) throw new Error('请输入有效的中国大陆手机号');
  return `+86 ${normalized}`;
}
function smsSdkHeaders(captchaToken = '') {
  return {
    'x-client-id': oauthClientId,
    'x-sdk-version': '9.0.2',
    'x-protocol-version': '301',
    'accept-language': 'zh-CN',
    ...(captchaToken ? { 'x-captcha-token': captchaToken } : {}),
  };
}
function captchaUrl(payload) {
  return String(payload?.url || payload?.data?.url || payload?.details?.[0]?.url || payload?.data?.details?.[0]?.url || '').trim();
}
function captchaFailure(payload) {
  const value = String(payload?.error || payload?.data?.error || '').toUpperCase();
  return value === 'CAPTCHA_REQUIRED' || value === 'CAPTCHA_INVALID';
}
function captchaRequiredPayload(url, phoneNumber, extra = {}) {
  return { authenticated: false, captcha_required: true, url, captcha_url: url, phone_number: phoneNumber, ...extra };
}
async function initializeSmsCaptcha(action, phoneNumber, previousToken = '') {
  const captcha = await accountPost('/v1/shield/captcha/init', {
    client_id: oauthClientId,
    action,
    device_id: deviceId,
    captcha_token: previousToken || null,
    meta: { phone_number: phoneNumber },
  }, smsSdkHeaders(previousToken));
  const url = captchaUrl(captcha.payload);
  if (url) return { url, token: '' };
  if (captcha.status >= 400 || captcha.payload?.error) throw new Error(accountError(captcha.payload, '无法初始化人机验证'));
  const tokenValue = String(authValue(captcha.payload, 'captcha_token') || '').trim();
  if (!tokenValue) throw new Error('人机验证没有返回可用令牌');
  return { url: '', token: tokenValue };
}
async function sendSmsLogin(body) {
  const phoneNumber = normalizeChinesePhone(body.phone_number ?? body.phone);
  let captchaToken = String(body.captcha_token || '').trim();
  if (!captchaToken) {
    const initialized = await initializeSmsCaptcha('POST:/v1/auth/verification', phoneNumber);
    if (initialized.url) return captchaRequiredPayload(initialized.url, phoneNumber);
    captchaToken = initialized.token;
  }
  let verification;
  for (let attempt = 0; attempt < 2; attempt += 1) {
    verification = await accountPost('/v1/auth/verification', {
      phone_number: phoneNumber,
      target: 'ANY',
      client_id: oauthClientId,
    }, smsSdkHeaders(captchaToken));
    const url = captchaUrl(verification.payload);
    if (url) return captchaRequiredPayload(url, phoneNumber);
    if (verification.status < 400 && !verification.payload?.error) break;
    if (attempt === 0 && captchaFailure(verification.payload)) {
      const initialized = await initializeSmsCaptcha('POST:/v1/auth/verification', phoneNumber, captchaToken);
      if (initialized.url) return captchaRequiredPayload(initialized.url, phoneNumber);
      captchaToken = initialized.token;
      continue;
    }
    throw new Error(accountError(verification.payload, '短信验证码发送失败'));
  }
  const verificationId = String(authValue(verification.payload, 'verification_id') || '').trim();
  const isUser = authValue(verification.payload, 'is_user');
  if (!verificationId || typeof isUser !== 'boolean') throw new Error('短信接口没有返回完整的验证任务');
  smsChallenges.set(verificationId, { phoneNumber, isUser, expiresAt: Date.now() + 10 * 60_000 });
  return { verification_id: verificationId, request_id: verificationId, is_user: isUser, phone_number: phoneNumber, captcha_required: false };
}
async function completeSmsLogin(body) {
  const verificationId = String(body.verification_id || body.request_id || '').trim();
  const verificationCode = String(body.verification_code ?? body.code ?? '').trim();
  if (!verificationId) throw new Error('缺少短信验证任务');
  if (!/^\d{4,8}$/.test(verificationCode)) throw new Error('请输入有效的短信验证码');
  const saved = smsChallenges.get(verificationId);
  if (saved && saved.expiresAt <= Date.now()) {
    smsChallenges.delete(verificationId);
    throw new Error('短信验证任务已过期，请重新获取验证码');
  }
  if (!saved && typeof body.is_user !== 'boolean') throw new Error('短信验证任务不存在，请重新获取验证码');
  const phoneNumber = saved?.phoneNumber || normalizeChinesePhone(body.phone_number ?? body.phone);
  const isUser = saved ? saved.isUser : body.is_user === true;
  let verificationToken = saved?.verificationCode === verificationCode ? String(saved.verificationToken || '') : '';
  if (!verificationToken) {
    const verified = await accountPost('/v1/auth/verification/verify', {
      verification_id: verificationId,
      verification_code: verificationCode,
      client_id: oauthClientId,
    }, smsSdkHeaders(body.captcha_token));
    if (verified.status >= 400 || verified.payload?.error) throw new Error(accountError(verified.payload, '短信验证码校验失败'));
    verificationToken = String(authValue(verified.payload, 'verification_token') || '').trim();
    if (saved) {
      saved.verificationCode = verificationCode;
      saved.verificationToken = verificationToken;
    }
  }
  if (!verificationToken) throw new Error('短信校验没有返回登录凭据');
  const endpoint = isUser ? '/v1/auth/signin' : '/v1/auth/signup';
  const credentialsBody = isUser ? {
    username: phoneNumber,
    verification_code: verificationCode,
    verification_token: verificationToken,
    client_id: oauthClientId,
  } : {
    phone_number: phoneNumber,
    verification_code: verificationCode,
    verification_token: verificationToken,
    client_id: oauthClientId,
    name: `光鸭用户${phoneNumber.slice(-4)}`,
  };
  let captchaToken = String(body.captcha_token || '').trim();
  let credentials;
  for (let attempt = 0; attempt < 2; attempt += 1) {
    credentials = await accountPost(endpoint, credentialsBody, smsSdkHeaders(captchaToken));
    const url = captchaUrl(credentials.payload);
    const challengeInfo = { verification_id: verificationId, request_id: verificationId, is_user: isUser };
    if (url) return captchaRequiredPayload(url, phoneNumber, challengeInfo);
    if (credentials.status < 400 && !credentials.payload?.error) break;
    if (attempt === 0 && captchaFailure(credentials.payload)) {
      const initialized = await initializeSmsCaptcha(`POST:${endpoint}`, phoneNumber, captchaToken);
      if (initialized.url) return captchaRequiredPayload(initialized.url, phoneNumber, challengeInfo);
      captchaToken = initialized.token;
      continue;
    }
    throw new Error(accountError(credentials.payload, '手机号登录失败'));
  }
  const accessToken = String(authValue(credentials.payload, 'access_token') || '').trim();
  const nextRefreshToken = String(authValue(credentials.payload, 'refresh_token') || '').trim();
  if (!accessToken) throw new Error('手机号登录没有返回 access_token');
  token = accessToken;
  refreshToken = nextRefreshToken || null;
  remoteCache.clear();
  remoteCache.set('', '');
  replaceAuthSession(token, refreshToken);
  smsChallenges.delete(verificationId);
  status('success', '手机号登录成功，可以开始使用云盘和备份任务');
  publishState();
  pump();
  schedulePendingUploadRecovery(0);
  return { authenticated: true, is_user: isUser };
}
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
    schedulePendingUploadRecovery(0);
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
    if (statusCode >= 400) {
      const message = payload.error_description || payload.msg || '刷新登录状态失败';
      if ([400, 401, 403].includes(statusCode)) {
        invalidateAuthSession();
        throw new Error(`登录态已失效，请重新扫码登录：${message}`);
      }
      throw new Error(message);
    }
    const accessToken = authValue(payload, 'access_token');
    const nextRefreshToken = authValue(payload, 'refresh_token');
    if (!accessToken) throw new Error('刷新登录状态时没有返回 access_token');
    token = String(accessToken);
    if (nextRefreshToken) refreshToken = String(nextRefreshToken);
    saveAuthSession(token, refreshToken);
    publishState();
    pump();
    schedulePendingUploadRecovery(0);
    return true;
  })().finally(() => { refreshPromise = null; });
  return refreshPromise;
}
async function findFolder(parentId, name) { for (let page = 0; page < 100; page += 1) { const response = await apiPost('/userres/v1/file/get_file_list', { page, pageSize: 100, parentId, resType: 2, needSubFolderStat: true }); const list = response.data?.list || []; const found = list.find((item) => item.resType === 2 && item.fileName === name); if (found?.fileId) return String(found.fileId); if (!list.length || (page + 1) * 100 >= Number(response.data?.total || 0)) break; } return null; }
async function ensureRemote(baseParentId, remotePath) {
  const normalized = normalizeRemote(remotePath);
  if (!normalized) return String(baseParentId || '');
  let parentId = String(baseParentId || '');
  let prefix = '';
  for (const part of normalized.split('/')) {
    prefix = prefix ? `${prefix}/${part}` : part;
    const cacheKey = `${baseParentId || ''}::${prefix}`;
    if (cacheEnabled && remoteCache.has(cacheKey)) {
      parentId = remoteCache.get(cacheKey);
      continue;
    }
    const response = await apiPost('/userres/v1/file/create_dir', { parentId, dirName: part, failIfNameExist: true }, [159]);
    const fileId = response.data?.fileId || (response.code === 159 ? await findFolder(parentId, part) : null);
    if (!fileId) throw new Error(`无法创建远程目录 ${prefix}`);
    parentId = String(fileId);
    if (cacheEnabled) {
      remoteCache.delete(cacheKey);
      remoteCache.set(cacheKey, parentId);
      trimRemoteCache();
    }
  }
  return parentId;
}
function isCloudIndexPendingMessage(message) { return /文件上传中|上传处理中|正在上传|正在处理|正在入库|任务处理中|任务未完成|稍后再试/.test(String(message || '')); }
function isExplicitPermanentCloudTaskFailure(error) {
  return error?.retryable === false && (Number.isFinite(error?.apiCode) || Number.isFinite(error?.httpStatus));
}
async function waitTask(taskId, eventPath) {
  const deadline = Date.now() + cloudConfirmTimeoutMs;
  let attempt = 0;
  while (Date.now() < deadline) {
    try {
      const response = await apiPost('/userres/v1/file/get_info_by_task_id', { taskId }, [145, 146, 155, 163]);
      if (response.data?.fileId) return response.data;
    } catch (error) {
      if (!isCloudIndexPendingMessage(error.message)) {
        if (isExplicitPermanentCloudTaskFailure(error)) error.permanentCloudTaskFailure = true;
        throw error;
      }
    }
    attempt += 1;
    publish({ type: 'progress', file_path: eventPath, percent: 100, bytes_per_second: 0, stage: '文件已上传，云端正在入库' });
    const delayMs = Math.min(cloudConfirmPollMs * Math.max(1, Math.ceil(attempt / 5)), 5_000, Math.max(0, deadline - Date.now()));
    if (delayMs > 0) await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  const error = new Error(`云端入库超过 ${Math.round(cloudConfirmTimeoutMs / 1000)} 秒仍未完成，请稍后刷新云盘确认`);
  error.retryable = true;
  throw error;
}
async function waitOperation(taskId) { if (!taskId) return; for (let index = 0; index < 90; index += 1) { const response = await apiPost('/userres/v1/get_task_status', { taskId }); const statusCode = Number(response.data?.status); const detail = response.data?.detail || {}; if ([2, 3].includes(statusCode) && detail.code && Number(detail.code) !== 0) throw new Error(detail.msg || '文件操作失败'); if (statusCode === 2) return; if (statusCode === 3) throw new Error(detail.msg || '文件操作失败'); await new Promise((resolve) => setTimeout(resolve, 1000)); } throw new Error('文件操作长时间未完成'); }
function gcidChunkSize(size) { if (size <= 0x08000000) return 256 * 1024; if (size <= 0x10000000) return 512 * 1024; if (size <= 0x20000000) return 1024 * 1024; return 2 * 1024 * 1024; }
async function calculateFileHash(filePath, algorithm) {
  const hash = crypto.createHash(algorithm);
  const stream = fs.createReadStream(filePath, { highWaterMark: 2 * 1024 * 1024 });
  for await (const chunk of stream) hash.update(chunk);
  return hash.digest('hex');
}
async function calculateFileGcid(filePath, size, modifiedMs, eventPath) {
  const resolvedPath = path.resolve(filePath);
  const modified = String(modifiedMs);
  const cached = cacheEnabled
    ? database.prepare('SELECT gcid FROM file_fingerprints WHERE file_path = ? AND size = ? AND modified_ms = ?')
      .get(resolvedPath, size, modified)
    : null;
  if (cached?.gcid) {
    publish({ type: 'progress', file_path: eventPath, percent: 0, bytes_per_second: 0, stage: '已复用秒传指纹' });
    return cached.gcid;
  }
  const handle = await fsp.open(resolvedPath, 'r');
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
  const gcid = outer.digest('hex').toUpperCase();
  if (cacheEnabled) {
    database.prepare(`INSERT INTO file_fingerprints (file_path, size, modified_ms, gcid, updated_at)
      VALUES (?, ?, ?, ?, ?)
      ON CONFLICT(file_path) DO UPDATE SET size = excluded.size, modified_ms = excluded.modified_ms, gcid = excluded.gcid, updated_at = excluded.updated_at`)
      .run(resolvedPath, size, modified, gcid, Math.floor(Date.now() / 1000));
    trimFileFingerprintCache();
  }
  return gcid;
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
  const initialSource = await validateWatchedSource(item);
  item.file_path = initialSource.absolute;
  await probeUploadFile(item.file_path);
  const first = await fsp.stat(item.file_path);
  if (!first.isFile()) throw new Error('源路径不是文件');
  assertSourceIdentity(item, first);
  await new Promise((resolve) => setTimeout(resolve, fileStabilityMs));
  const stableSource = await validateWatchedSource(item);
  item.file_path = stableSource.absolute;
  await probeUploadFile(item.file_path);
  const second = await fsp.stat(item.file_path);
  if (first.size !== second.size || first.mtimeMs !== second.mtimeMs
    || Number(first.dev) !== Number(second.dev) || Number(first.ino) !== Number(second.ino)) throw new FileBusyError();
  return { ...item, size: second.size, mtime: item.history_path ? item.mtime : second.mtimeMs, source_dev: second.dev, source_ino: second.ino };
}
function scheduleBusyUploadRetry(key, item) {
  waitingFiles.set(key, item);
  publish({ type: 'file', state: 'waiting-file', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size, stage: '另外的程序正在使用该文件，释放后将自动上传' });
  publishState();
  setTimeout(async () => {
    waitingFiles.delete(key);
    try {
      const stat = await fsp.stat(item.file_path);
      if (!stat.isFile()) return;
      if (!item.mapping_id.startsWith('__') && !mappings.some((mapping) => mapping.id === item.mapping_id && mapping.enabled)) return;
      const refreshed = { ...item, size: stat.size, mtime: item.history_path ? item.mtime : stat.mtimeMs };
      const stamp = `${refreshed.size}:${refreshed.mtime}`;
      if (history.get(key) === stamp || pendingUploads.has(key) || inflight.get(key) === stamp || (queue.has(key) && `${queue.get(key).size}:${queue.get(key).mtime}` === stamp)) return;
      queue.set(key, refreshed);
      publish({ type: 'file', state: 'waiting-file', file_path: uploadEventPath(refreshed), uploaded_bytes: 0, total_bytes: refreshed.size, stage: '另外的程序正在使用该文件，释放后将自动上传' });
    } catch {
      // 文件暂时消失时等待后续文件系统事件重新入队。
    } finally {
      publishState();
      pump();
    }
  }, fileBusyRetryMs);
}
async function preflightFlashUpload(item) {
  const source = await validateWatchedSource(item);
  item.file_path = source.absolute;
  const stat = source.stat;
  item.size = stat.size;
  if (!item.history_path) item.mtime = stat.mtimeMs;
  const eventPath = uploadEventPath(item);
  if (loadUploadCheckpoint(item)) return { kind: 'skipped' };

  publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, bytes_per_second: 0, stage: '正在后台校验秒传' });
  const parentId = await ensureRemote(item.remote_parent_id || '', item.remote_dir);
  const res = { fileSize: stat.size };
  if (stat.size < 1024 * 1024) {
    publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, bytes_per_second: 0, stage: '正在后台计算秒传 MD5' });
    res.md5 = await calculateFileHash(item.file_path, 'md5');
  }
  const response = await apiPost('/userres/v1/get_res_center_token', {
    capacity: 2,
    name: path.basename(item.file_path),
    res,
    parentId,
  }, [156]);
  const data = response.data;
  if (!data?.taskId) throw new Error('光鸭没有返回上传任务 ID');
  let taskId = data.taskId;
  let instantUpload = response.code === 156;
  if (!instantUpload && stat.size >= 1024 * 1024) {
    try {
      const gcid = await calculateFileGcid(item.file_path, stat.size, stat.mtimeMs, eventPath);
      const flash = await apiPost('/userres/v1/check_can_flash_upload', { taskId, gcid });
      instantUpload = flash.data?.canFlashUpload === true;
      if (instantUpload && flash.data?.taskId) taskId = String(flash.data.taskId);
    } catch (error) {
      status('warning', `后台秒传校验失败，稍后继续普通上传：${error.message}`);
    }
  }
  if (!instantUpload) return { kind: 'miss', data };

  clearUploadCheckpoint(item);
  publish({ type: 'progress', file_path: eventPath, percent: 100, uploaded_bytes: stat.size, total_bytes: stat.size, bytes_per_second: 0, stage: '已命中秒传' });
  if (item.mapping_id) savePendingUploadRecord(item, { taskId, remoteFileId: null });
  publish({ type: 'file', state: 'processing', file_path: eventPath, uploaded_bytes: stat.size, total_bytes: stat.size, stage: '已秒传，正在等待云端入库' });
  schedulePendingUploadRecovery(0);
  return { kind: 'accepted' };
}
async function upload(item) {
  const source = await validateWatchedSource(item);
  item.file_path = source.absolute;
  const stat = source.stat;
  item.size = stat.size;
  if (!item.history_path) item.mtime = stat.mtimeMs;
  const eventPath = uploadEventPath(item);
  publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, stage: '正在准备云端目录' });
  const parentId = await ensureRemote(item.remote_parent_id || '', item.remote_dir);
  publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, stage: '正在申请上传凭证' });
  let checkpoint = loadUploadCheckpoint(item);
  let response;
  let data;
  let flashPrechecked = false;
  if (checkpoint) {
    publish({
      type: 'progress',
      file_path: eventPath,
      percent: stat.size ? Math.round(checkpoint.uploadedBytes * 100 / stat.size) : 0,
      uploaded_bytes: checkpoint.uploadedBytes,
      total_bytes: stat.size,
      stage: '正在恢复上传断点',
    });
    try {
      const resumed = await resumeUploadParams(checkpoint.params, stat.size);
      response = resumed.response;
      data = resumed.params;
      checkpoint.params = data;
    } catch (error) {
      status('warning', `恢复上传断点失败，将重新创建上传任务：${error.message}`);
      clearUploadCheckpoint(item);
      checkpoint = null;
    }
  }
  if (!data && !checkpoint) {
    data = takeFlashPreflightToken(queueKey(item.mapping_id, uploadHistoryPath(item)), item);
    if (data) {
      flashPrechecked = true;
      publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, bytes_per_second: 0, stage: '秒传未命中，正在进入上传通道' });
    }
  }
  if (!data) {
    const res = { fileSize: stat.size };
    if (stat.size < 1024 * 1024) {
      publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, stage: '正在计算秒传 MD5' });
      res.md5 = await calculateFileHash(item.file_path, 'md5');
    }
    response = await apiPost('/userres/v1/get_res_center_token', { capacity: 2, name: path.basename(item.file_path), res, parentId }, [156]);
    data = response.data;
  }
  if (!data?.taskId) throw new Error('光鸭没有返回上传任务 ID');
  let taskId = data.taskId;
  let instantUpload = response?.code === 156;
  if (!instantUpload && !checkpoint && !flashPrechecked && stat.size >= 1024 * 1024) {
    try {
      const gcid = await calculateFileGcid(item.file_path, stat.size, stat.mtimeMs, eventPath);
      const flash = await apiPost('/userres/v1/check_can_flash_upload', { taskId, gcid });
      instantUpload = flash.data?.canFlashUpload === true;
      if (instantUpload && flash.data?.taskId) taskId = String(flash.data.taskId);
    } catch (error) {
      status('warning', `秒传校验失败，继续普通上传：${error.message}`);
    }
  }
  if (!instantUpload) {
    if (!data.creds || !data.objectPath) throw new Error('光鸭没有返回完整上传凭证');
    let currentParams = data;
    let multipartCheckpoint = checkpoint?.checkpoint
      ? { ...checkpoint.checkpoint, file: item.file_path }
      : undefined;
    const uploadedAtStart = Math.max(0, Number(checkpoint?.uploadedBytes || 0));
    let lastUploadedBytes = uploadedAtStart;
    publish({ type: 'file', state: 'uploading', file_path: eventPath, uploaded_bytes: uploadedAtStart, total_bytes: stat.size });
    publish({ type: 'progress', file_path: eventPath, percent: stat.size ? Math.round(uploadedAtStart * 100 / stat.size) : 0, uploaded_bytes: uploadedAtStart, total_bytes: stat.size, stage: checkpoint ? '正在从断点继续上传' : '正在连接 OSS' });
    const uploadStartedAt = Date.now();
    for (let attempt = 0; ; attempt += 1) {
      const client = new OSS({
        region: currentParams.region,
        accessKeyId: currentParams.creds.accessKeyID,
        accessKeySecret: currentParams.creds.secretAccessKey || currentParams.creds.accessKeySecret,
        stsToken: currentParams.creds.sessionToken,
        bucket: currentParams.bucketName,
        endpoint: currentParams.endPoint,
        secure: true,
        timeout: ossTimeoutMs,
        retryMax: ossRetryMax,
        requestErrorRetryHandle: () => {
          publish({ type: 'progress', file_path: eventPath, uploaded_bytes: lastUploadedBytes, total_bytes: stat.size, stage: 'OSS 分片超时，正在自动重试', bytes_per_second: 0 });
          return true;
        },
      });
      try {
        await client.multipartUpload(currentParams.objectPath, item.file_path, {
          checkpoint: multipartCheckpoint,
          partSize: uploadPartSize(stat.size, multipartMode),
          parallel: ossParallel,
          timeout: ossTimeoutMs,
          progress: (fraction, nextCheckpoint) => {
            const normalized = Math.max(0, Math.min(1, Number(fraction) || 0));
            const uploadedBytes = Math.round(normalized * stat.size);
            lastUploadedBytes = Math.max(lastUploadedBytes, uploadedBytes);
            const elapsedSeconds = Math.max((Date.now() - uploadStartedAt) / 1000, 0.001);
            const transferredThisRun = Math.max(0, uploadedBytes - uploadedAtStart);
            if (nextCheckpoint) {
              multipartCheckpoint = { ...nextCheckpoint, file: item.file_path };
              saveUploadCheckpoint(item, currentParams, nextCheckpoint, uploadedBytes);
            }
            publish({ type: 'progress', file_path: eventPath, percent: Math.round(normalized * 100), uploaded_bytes: uploadedBytes, total_bytes: stat.size, bytes_per_second: transferredThisRun / elapsedSeconds, stage: checkpoint ? '正在断点续传' : '正在上传' });
          },
        });
        break;
      } catch (error) {
        if (error.code === 'SecurityTokenExpired') {
          const resumed = await resumeUploadParams(currentParams, stat.size);
          currentParams = resumed.params;
          taskId = currentParams.taskId || taskId;
          if (multipartCheckpoint) saveUploadCheckpoint(item, currentParams, multipartCheckpoint, lastUploadedBytes);
          continue;
        }
        if (attempt < ossRetryMax) {
          await new Promise((resolve) => setTimeout(resolve, Math.min(2_000 * (attempt + 1), 10_000)));
          continue;
        }
        const message = ['ResponseTimeoutError', 'ConnectionTimeoutError'].includes(error.name)
          ? `OSS 分片上传连续超时（单次 ${Math.round(ossTimeoutMs / 1000)} 秒）：${error.message}`
          : `OSS 分片上传中断：${error.message}`;
        const resumableError = new Error(message);
        resumableError.requeueUpload = Boolean(loadUploadCheckpoint(item));
        throw resumableError;
      }
    }
    clearUploadCheckpoint(item);
  } else {
    clearUploadCheckpoint(item);
    publish({ type: 'progress', file_path: eventPath, percent: 100, uploaded_bytes: stat.size, total_bytes: stat.size, stage: '已命中秒传' });
  }
  if (item.mapping_id) {
    const pendingTask = { taskId, remoteFileId: null };
    savePendingUploadRecord(item, pendingTask);
  }
  publish({ type: 'progress', file_path: eventPath, percent: 100, uploaded_bytes: stat.size, total_bytes: stat.size, bytes_per_second: 0, stage: '已上传，正在等待云端入库' });
  publish({ type: 'file', state: 'processing', file_path: eventPath, uploaded_bytes: stat.size, total_bytes: stat.size, stage: '已上传，正在等待云端入库' });
  let taskData;
  try { taskData = await waitTask(taskId, eventPath); }
  catch (error) {
    const key = queueKey(item.mapping_id, uploadHistoryPath(item));
    if (error.permanentCloudTaskFailure) {
      clearPendingUpload(key);
      error.requeueUpload = true;
      throw error;
    }
    schedulePendingUploadRecovery();
    return { taskId, remoteFileId: null, pending: true, pendingError: error.message };
  }
  return { taskId, remoteFileId: taskData?.fileId || null };
}
function archiveDestination(baseDestination, modifiedMs, attempt) {
  if (attempt === 0) return baseDestination;
  const parsed = path.parse(baseDestination);
  const counter = attempt === 1 ? '' : `-${attempt}`;
  return path.join(parsed.dir, `${parsed.name}-${Math.round(modifiedMs)}${counter}${parsed.ext}`);
}
async function moveFileExclusive(source, destination, sourceStat) {
  try {
    await fsp.link(source, destination);
  } catch (error) {
    if (error.code === 'EEXIST') throw error;
    if (!['EXDEV', 'EPERM', 'EACCES', 'ENOTSUP'].includes(error.code)) throw error;
    await fsp.copyFile(source, destination, fs.constants.COPYFILE_EXCL);
    try {
      const [currentSource, copied] = await Promise.all([fsp.stat(source), fsp.stat(destination)]);
      if (currentSource.size !== sourceStat.size || currentSource.mtimeMs !== sourceStat.mtimeMs) throw new Error('跨卷归档期间源文件发生变化，已保留源文件');
      if (!copied.isFile() || copied.size !== sourceStat.size) throw new Error('跨卷归档校验失败，已保留源文件');
      await fsp.utimes(destination, sourceStat.atime, sourceStat.mtime);
      await fsp.unlink(source);
    } catch (copyError) {
      await fsp.unlink(destination).catch(() => {});
      throw copyError;
    }
    return;
  }
  try {
    const currentSource = await fsp.stat(source);
    if (currentSource.size !== sourceStat.size || currentSource.mtimeMs !== sourceStat.mtimeMs) throw new Error('归档期间源文件发生变化，已保留源文件');
    await fsp.unlink(source);
  } catch (error) {
    await fsp.unlink(destination).catch(() => {});
    throw error;
  }
}
async function applySourcePolicy(item) {
  const mapping = mappings.find((entry) => entry.id === item.mapping_id);
  if (!mapping || mapping.source_policy === 'keep') return null;
  const source = await validateWatchedSource(item);
  item.file_path = source.absolute;
  const stat = source.stat;
  if (stat.size !== item.size || stat.mtimeMs !== item.mtime) throw new Error('上传期间源文件发生变化，已保留源文件且不会执行上传后策略');
  if (mapping.source_policy === 'delete') { await fsp.rm(item.file_path); return '已按任务策略删除源文件'; }
  if (mapping.source_policy !== 'archive' || !mapping.archive_path) throw new Error('归档策略没有配置归档目录');
  const relative = path.relative(source.mappingRoot, item.file_path);
  if (!relative || path.isAbsolute(relative) || relative.startsWith('..')) throw new Error('归档源文件超出备份任务目录');
  const archiveBase = await fsp.realpath(mapping.archive_path);
  if (!samePath(archiveBase, mapping.archive_path)
    || !fileRoots.some((root) => isWithinRoot(root, archiveBase))
    || isWithinRoot(protectedDataRoot, archiveBase)) throw new Error('归档目录的真实路径已改变或超出允许范围');
  const destinationParent = path.join(archiveBase, path.dirname(relative));
  await fsp.mkdir(destinationParent, { recursive: true });
  const destinationParentReal = await fsp.realpath(destinationParent);
  if (!isWithinRoot(archiveBase, destinationParentReal)) throw new Error('归档目标目录通过符号链接超出允许范围');
  const baseDestination = path.join(destinationParentReal, path.basename(relative));
  for (let attempt = 0; attempt < 100_000; attempt += 1) {
    const destination = archiveDestination(baseDestination, item.mtime, attempt);
    try {
      await moveFileExclusive(item.file_path, destination, stat);
      return `已移动到归档目录：${destination}`;
    } catch (error) {
      if (error.code === 'EEXIST') continue;
      throw error;
    }
  }
  throw new Error('归档目录中同名文件过多，已保留源文件');
}
function rebuildPendingItem(row) {
  if (row.item_json) {
    try {
      const item = JSON.parse(row.item_json);
      if (item && typeof item === 'object') return { ...item, mapping_id: row.mapping_id, size: Number(row.size), mtime: Number(row.modified_ms) };
    } catch {}
  }
  const mapping = mappings.find((entry) => entry.id === row.mapping_id);
  if (!mapping) return null;
  const relative = path.relative(mapping.local_path, row.file_path).replaceAll('\\', '/');
  if (!relative || relative.startsWith('../') || path.isAbsolute(relative)) return null;
  const relativeDir = path.posix.dirname(relative) === '.' ? '' : path.posix.dirname(relative);
  return { mapping_id: mapping.id, file_path: row.file_path, relative_path: relative, change_kind: 'added', remote_parent_id: mapping.remote_parent_id || '', remote_dir: [mapping.remote_parent_id ? '' : mapping.remote_path, relativeDir].filter(Boolean).join('/'), size: Number(row.size), mtime: Number(row.modified_ms) };
}
async function finalizeConfirmedUpload(key, item, taskData, recovered = false) {
  if (!confirmPendingUploadRecord(key, taskData.taskId, taskData.remoteFileId)) return false;
  clearAutoShareFailure(item);
  try { await scheduleAutoShare(item, taskData); }
  catch (error) { status('error', `文件已确认入库，但自动分享排队失败：${error.message}`); }
  try {
    const action = await applySourcePolicy(item);
    if (action) status('success', action);
  } catch (error) {
    if (error.code !== 'ENOENT') status('warning', `文件已确认入库，但上传后策略执行失败：${error.message}`);
  }
  if (recovered && item.cleanup_path && isWithinRoot(manualUploadRoot, item.cleanup_path)) await fsp.rm(item.cleanup_path, { recursive: true, force: true });
  publish({ type: 'file', state: 'done', file_path: uploadEventPath(item), uploaded_bytes: item.size, total_bytes: item.size });
  const mapping = mappings.find((entry) => entry.id === item.mapping_id && entry.enabled);
  if (mapping) {
    try {
      const current = await fsp.stat(item.file_path);
      if (current.isFile() && (current.size !== item.size || current.mtimeMs !== item.mtime)) await enqueue(mapping, item.file_path);
    } catch {}
  }
  return true;
}
function scheduleCloudUploadRetry(key, item, reason) {
  if (!item?.file_path) {
    status('error', `云端明确拒绝上传任务，且本地源文件信息不足，无法自动重传：${reason}`);
    return;
  }
  waitingFiles.set(key, item);
  publish({ type: 'file', state: 'waiting-file', file_path: uploadEventPath(item), stage: '云端入库失败，稍后将重新上传' });
  setTimeout(async () => {
    waitingFiles.delete(key);
    try {
      const stat = await fsp.stat(item.file_path);
      if (!stat.isFile()) throw new Error('本地源文件已不存在');
      const refreshed = { ...item, size: stat.size, mtime: item.history_path ? item.mtime : stat.mtimeMs };
      queue.set(key, refreshed);
      publish({ type: 'file', state: 'queued', file_path: uploadEventPath(refreshed), stage: '正在重新上传' });
    } catch (error) {
      status('error', `云端明确拒绝上传任务，无法自动重传：${uploadEventPath(item)}：${error.message}`);
      if (item.cleanup_path && isWithinRoot(manualUploadRoot, item.cleanup_path)) await fsp.rm(item.cleanup_path, { recursive: true, force: true });
    } finally {
      publishState();
      pump();
    }
  }, fileBusyRetryMs);
}
let pendingRecoveryPromise = null;
let pendingRecoveryTimer = null;
function schedulePendingUploadRecovery(delayMs = Math.max(1_000, cloudConfirmPollMs * 5)) {
  if (!token || !pendingUploads.size || pendingRecoveryPromise || pendingRecoveryTimer) return;
  pendingRecoveryTimer = setTimeout(() => {
    pendingRecoveryTimer = null;
    void recoverPendingUploads();
  }, delayMs);
}
async function recoverPendingUploads() {
  if (!token || pendingRecoveryPromise) return pendingRecoveryPromise;
  pendingRecoveryPromise = (async () => {
    for (const [key, row] of [...pendingUploads]) {
      if (!token || inflight.has(key) || !pendingUploads.has(key)) continue;
      const item = rebuildPendingItem(row);
      const eventPath = item ? uploadEventPath(item) : row.file_path;
      if (!row.task_id) {
        clearPendingUpload(key);
        if (item) scheduleCloudUploadRetry(key, item, '缺少云端任务 ID');
        else status('error', `未确认上传记录缺少任务 ID，已清除但无法自动重传：${eventPath}`);
        continue;
      }
      publish({ type: 'file', state: 'processing', file_path: eventPath, stage: '正在恢复云端入库确认' });
      try {
        const data = await waitTask(row.task_id, eventPath);
        if (!item) {
          if (!confirmPendingUploadRecord(key, row.task_id, data.fileId)) continue;
          status('warning', `已恢复云端入库确认，但旧记录缺少任务上下文，未执行自动分享和源文件策略：${eventPath}`);
        } else {
          await finalizeConfirmedUpload(key, item, { taskId: row.task_id, remoteFileId: data.fileId }, true);
        }
      } catch (error) {
        if (error.permanentCloudTaskFailure) {
          clearPendingUpload(key);
          if (item) scheduleCloudUploadRetry(key, item, error.message);
          else status('error', `云端明确拒绝未确认上传任务，已清除记录但无法自动重传：${eventPath}：${error.message}`);
        } else {
          status('warning', `云端入库仍未确认，将保留记录并稍后重试：${eventPath}：${error.message}`);
        }
      }
    }
  })().finally(() => {
    pendingRecoveryPromise = null;
    publishState();
    if (token && pendingUploads.size) schedulePendingUploadRecovery();
  });
  return pendingRecoveryPromise;
}
async function cleanupUnreferencedManualUploads() {
  const retained = new Set();
  const resumableRows = database.prepare('SELECT item_json FROM upload_checkpoints').all();
  for (const row of [...pendingUploads.values(), ...resumableRows]) {
    if (!row.item_json) continue;
    try {
      const cleanupPath = path.resolve(JSON.parse(row.item_json)?.cleanup_path || '');
      if (isWithinRoot(manualUploadRoot, cleanupPath)) retained.add(cleanupPath);
    } catch {}
  }
  for (const entry of await fsp.readdir(manualUploadRoot, { withFileTypes: true })) {
    const candidate = path.join(manualUploadRoot, entry.name);
    if (!retained.has(candidate)) await fsp.rm(candidate, { recursive: true, force: true });
  }
}
function pumpFlashPreflight() {
  if (paused || !token || active === 0 || activeFlashPreflights >= flashPreflightConcurrency) return;
  const candidate = [...queue.entries()].find(([key, item]) => !flashPreflightCached(key, item));
  if (!candidate) return;
  const [key, item] = candidate;
  queue.delete(key);
  inflight.set(key, uploadStamp(item));
  inflightItems.set(key, item);
  activeFlashPreflights += 1;
  publish({ type: 'file', state: 'preparing', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size, stage: '正在后台校验秒传' });
  let preserveSource = true;
  prepareUploadItem(item).then((ready) => {
    Object.assign(item, ready);
    return preflightFlashUpload(item);
  }).then(async (result) => {
    if (result.kind === 'miss') {
      if (!mappingAcceptsUpload(item)) {
        flashPreflightCache.delete(key);
        return;
      }
      flashPreflightCache.set(key, { stamp: uploadStamp(item), data: result.data, createdAt: Date.now() });
      prependQueuedItem(key, item);
      publish({ type: 'file', state: 'queued', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size, stage: '秒传未命中，等待上传通道' });
      return;
    }
    if (result.kind === 'skipped') {
      if (!mappingAcceptsUpload(item)) {
        flashPreflightCache.delete(key);
        return;
      }
      flashPreflightCache.set(key, { stamp: uploadStamp(item), data: null, createdAt: Date.now() });
      prependQueuedItem(key, item);
      publish({ type: 'file', state: 'queued', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size, stage: '已有上传断点，等待上传通道' });
      return;
    }
    flashPreflightCache.delete(key);
    if (result.kind === 'accepted') return;
  }).catch((error) => {
    if (isFileBusyError(error)) {
      scheduleBusyUploadRetry(key, item);
      return;
    }
    if (error.requeueUpload) {
      scheduleCloudUploadRetry(key, item, error.message);
      return;
    }
    if (!mappingAcceptsUpload(item)) {
      flashPreflightCache.delete(key);
      return;
    }
    flashPreflightCache.set(key, { stamp: uploadStamp(item), data: null, createdAt: Date.now() });
    prependQueuedItem(key, item);
    status('warning', `后台秒传预检失败，已回到上传队列：${error.message}`);
    publish({ type: 'file', state: 'queued', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size, stage: '秒传预检失败，等待普通上传' });
  }).finally(async () => {
    if (!preserveSource && item.cleanup_path && isWithinRoot(manualUploadRoot, item.cleanup_path)) await fsp.rm(item.cleanup_path, { recursive: true, force: true });
    inflight.delete(key);
    inflightItems.delete(key);
    activeFlashPreflights = Math.max(0, activeFlashPreflights - 1);
    publishState();
    pump();
  });
}
function pump() {
  if (paused || !token) { publishState(); return; }
  while (active < uploadConcurrency && queue.size) {
    const [key, item] = queue.entries().next().value;
    queue.delete(key);
    inflight.set(key, `${item.size}:${item.mtime}`);
    inflightItems.set(key, item);
    active += 1;
    publish({ type: 'file', state: 'preparing', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size });
    let preserveSource = false;
    prepareUploadItem(item).then((ready) => {
      Object.assign(item, ready);
      return upload(item);
    }).then(async (taskData) => {
      if (taskData.pending) {
        preserveSource = true;
        status('warning', `文件已上传到 OSS，云端尚未确认入库；已保留记录并会自动重试：${uploadEventPath(item)}：${taskData.pendingError}`);
        publish({ type: 'file', state: 'processing', file_path: uploadEventPath(item), stage: '等待云端入库，下次将自动恢复确认' });
        return;
      }
      await finalizeConfirmedUpload(key, item, taskData);
    }).catch((error) => {
      if (isFileBusyError(error)) {
        preserveSource = true;
        scheduleBusyUploadRetry(key, item);
        return;
      }
      if (error.requeueUpload) {
        preserveSource = true;
        scheduleCloudUploadRetry(key, item, error.message);
      }
      recordAutoShareFailure(item, error);
      console.error(`上传失败：${item.file_path}：${error.stack || error.message}`);
      publish({ type: 'file', state: 'error', file_path: uploadEventPath(item), total_bytes: item.size, error: error.message });
    }).finally(async () => {
      if (!preserveSource && item.cleanup_path && isWithinRoot(manualUploadRoot, item.cleanup_path)) await fsp.rm(item.cleanup_path, { recursive: true, force: true });
      inflight.delete(key);
      inflightItems.delete(key);
      active -= 1;
      publishState();
      pump();
    });
  }
  pumpFlashPreflight();
  publishState();
}
async function enqueue(mapping, file) {
  if (!mapping.enabled) return;
  let source;
  try { source = await resolveMappingPath(mapping, file, 'file'); } catch { return; }
  const filePath = source.absolute;
  const stat = source.stat;
  if (ignore(filePath) || !shouldSync(filePath, mapping.sync_types)) return;
  const key = queueKey(mapping.id, filePath);
  const mark = `${stat.size}:${stat.mtimeMs}`;
  if (history.get(key) === mark || pendingUploads.has(key) || inflight.get(key) === mark) return;
  if (waitingFiles.has(key)) return;
  const queued = queue.get(key);
  if (queued && `${queued.size}:${queued.mtime}` === mark) return;
  const relative = source.relative.replaceAll('\\', '/');
  const relativeDir = path.posix.dirname(relative) === '.' ? '' : path.posix.dirname(relative);
  const item = { mapping_id: mapping.id, file_path: filePath, relative_path: relative, change_kind: history.has(key) ? 'changed' : 'added', remote_parent_id: mapping.remote_parent_id || '', remote_dir: [mapping.remote_parent_id ? '' : mapping.remote_path, relativeDir].filter(Boolean).join('/'), size: stat.size, mtime: stat.mtimeMs, source_dev: stat.dev, source_ino: stat.ino };
  let reused = null;
  try { reused = reuseMatchingConfirmedUpload(item); }
  catch (error) { status('warning', error.message); }
  if (reused) {
    if (mapping.auto_share && !reuseAutoShareBinding(item, reused.sourceMappingId)) {
      try { await scheduleAutoShare(item, reused); }
      catch (error) { status('error', `历史文件无需重复上传，但自动分享排队失败：${error.message}`); }
    }
    publishState();
    return;
  }
  queue.set(key, item);
  publish({ type: 'file', state: token ? 'queued' : 'waiting-login', file_path: filePath });
  pump();
}
async function collectExistingFiles(mapping, root) {
  const result = [];
  async function visit(current) {
    let directory;
    try { directory = await resolveMappingPath(mapping, current, 'directory'); } catch { return; }
    let entries = [];
    try { entries = await fsp.readdir(directory.absolute, { withFileTypes: true }); } catch { return; }
    for (const entry of entries) {
      if (entry.isSymbolicLink()) continue;
      const file = path.join(directory.absolute, entry.name);
      if (entry.isDirectory()) await visit(file);
      else if (entry.isFile() && !ignore(file) && shouldSync(file, mapping.sync_types)) result.push(file);
    }
  }
  await visit(root);
  return result;
}
async function enqueueDirectory(mapping, directory) {
  const files = await collectExistingFiles(mapping, directory);
  for (const file of files) await enqueue(mapping, file);
}
async function startWatcher(mapping) {
  await watchers.get(mapping.id)?.close();
  watchers.delete(mapping.id);
  if (!mapping.enabled) return;
  const root = await resolveMappingPath(mapping, mapping.local_path, 'directory');
  const polling = mapping.monitor_mode === 'polling';
  const watcher = chokidar.watch(root.absolute, {
    ignoreInitial: true,
    persistent: true,
    followSymlinks: false,
    usePolling: polling,
    interval: polling ? 5000 : 100,
    binaryInterval: polling ? 5000 : 300,
    awaitWriteFinish: { stabilityThreshold: 1200, pollInterval: polling ? 1000 : 200 },
  });
  watcher.on('add', (file) => { void enqueue(mapping, file); });
  watcher.on('change', (file) => { void enqueue(mapping, file); });
  watcher.on('addDir', (directory) => { void enqueueDirectory(mapping, directory); });
  watcher.on('error', (error) => { mapping.watch_error = error.message; status('error', `监控失败：${error.message}`); });
  watchers.set(mapping.id, watcher);
  await new Promise((resolve, reject) => { watcher.once('ready', resolve); watcher.once('error', reject); });
  mapping.watch_error = null;
  if (mapping.scan_existing) {
    const existing = await collectExistingFiles(mapping, root.absolute);
    status('info', `正在扫描已有文件：${existing.length} 个`);
    for (const file of existing) await enqueue(mapping, file);
    if (existing.length) publishState();
  }
}
async function restartWatchers() { for (const watcher of watchers.values()) await watcher.close(); watchers.clear(); for (const mapping of mappings) { if (!mapping.enabled) continue; try { await startWatcher(mapping); } catch (error) { mapping.enabled = false; mapping.watch_error = error.message; console.error(`备份任务监控启动失败：${mapping.local_path}：${error.message}`); } } await saveConfig(); }
async function routeApi(request, response, url) { if (request.method === 'GET' && url.pathname === '/api/state') return json(response, 200, state()); if (request.method === 'GET' && url.pathname === '/api/events') { response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' }); response.write(`data: ${JSON.stringify({ type: 'state', state: state() })}\n\n`); clients.add(response); request.on('close', () => clients.delete(response)); return; } if (request.method === 'POST' && url.pathname === '/api/auth') { const body = await readBody(request); token = String(body.token || '').trim().replace(/^Bearer\s+/i, '') || null; saveAuthToken(token); publishState(); pump(); return json(response, 200, state()); } if (request.method === 'POST' && url.pathname === '/api/mappings') { const body = await readBody(request); const mapping = { id: crypto.randomUUID(), local_path: allowedPath(body.local_path), remote_path: normalizeRemote(body.remote_path), enabled: true }; const stat = await fsp.stat(mapping.local_path); if (!stat.isDirectory()) throw new Error('监控路径不是目录'); mappings.push(mapping); await saveConfig(); await startWatcher(mapping); publishState(); return json(response, 200, mapping); } if (request.method === 'DELETE' && url.pathname.startsWith('/api/mappings/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); await watchers.get(id)?.close(); watchers.delete(id); mappings = mappings.filter((item) => item.id !== id); deleteMappingTransientUploads(id); await saveConfig(); publishState(); return json(response, 200, {}); } if (request.method === 'PATCH' && url.pathname.startsWith('/api/mappings/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); const body = await readBody(request); const mapping = mappings.find((item) => item.id === id); if (!mapping) return json(response, 404, { error: '监控目录不存在' }); mapping.enabled = Boolean(body.enabled); await saveConfig(); await startWatcher(mapping); publishState(); return json(response, 200, mapping); } if (request.method === 'POST' && url.pathname === '/api/queue/pause') { paused = true; publishState(); return json(response, 200, state()); } if (request.method === 'POST' && url.pathname === '/api/queue/resume') { paused = false; pump(); return json(response, 200, state()); } json(response, 404, { error: 'not found' }); }
async function apiOverview() { const assets = await apiPost('/assets/v1/get_assets', {}); let profile = {}; try { profile = await accountGet('/v1/user/me'); } catch { try { profile = (await apiPost('/activity/v1/get_user_data', {})).data || {}; } catch {} } return { assets: assets.data || {}, profile: profile?.data || profile || {} }; }
function cloudFileExtension(record) {
  const supplied = String(record?.ext || '').trim().replace(/^\./, '').toLowerCase();
  if (supplied) return supplied;
  const name = String(record?.fileName || record?.name || '');
  const index = name.lastIndexOf('.');
  return index > 0 && index < name.length - 1 ? name.slice(index + 1).toLowerCase() : '';
}
function matchesSearchType(record, requestedType) {
  if (!requestedType) return true;
  const folder = Number(record?.resType) === 2;
  if (requestedType === 'folder') return folder;
  return !folder && SEARCH_EXTENSION_GROUPS[requestedType].has(cloudFileExtension(record));
}
function fileTypeForExtension(extension) {
  const type = Object.keys(SEARCH_EXTENSION_GROUPS).find((name) => SEARCH_EXTENSION_GROUPS[name].has(extension));
  return type ? SEARCH_FILE_TYPES[type] : null;
}
async function searchCloudFiles(url) {
  const query = String(url.searchParams.get('query') || '').trim();
  const page = Math.max(0, Math.floor(Number(url.searchParams.get('page') || 0) || 0));
  const requestedType = String(url.searchParams.get('type') || '').trim().toLowerCase();
  const requestedExtension = String(url.searchParams.get('extension') || '').trim().replace(/^\./, '').toLowerCase();
  if (requestedType && !SEARCH_TYPES.has(requestedType)) throw new Error('文件类型必须是 image、video、audio、document、archive 或 folder');
  if (requestedExtension && !/^[a-z0-9]{1,16}$/.test(requestedExtension)) throw new Error('文件后缀格式无效');
  let result;
  if (!query && (requestedType || requestedExtension)) {
    const folder = requestedType === 'folder';
    const fileType = !folder && (SEARCH_FILE_TYPES[requestedType] || fileTypeForExtension(requestedExtension));
    const body = { parentId: '*', pageSize: 100, page, resType: folder ? 2 : 1, orderBy: 3, sortType: 1 };
    if (fileType) body.fileTypes = [fileType];
    result = await apiPost('/userres/v1/file/get_file_list', body);
  } else {
    result = await apiPost('/userres/v1/file/search_files', { name: query, pageSize: 100, page });
  }
  const data = result.data || {};
  const remoteList = Array.isArray(data.list) ? data.list : [];
  const list = remoteList.filter((record) => matchesSearchType(record, requestedType)
    && (!requestedExtension || (Number(record?.resType) !== 2 && cloudFileExtension(record) === requestedExtension)));
  return {
    ...result,
    data: {
      ...data,
      list,
      total: requestedType || requestedExtension ? list.length : Number(data.total ?? list.length),
      remote_total: Number(data.total ?? remoteList.length),
      remote_count: remoteList.length,
      page,
      pageSize: 100,
      page_size: 100,
    },
  };
}

function validateFileIds(fileIds) { if (!Array.isArray(fileIds) || !fileIds.length) throw new Error('请至少选择一个文件或文件夹'); return fileIds.map(String); }
function validateFolderName(folderName) {
  const name = String(folderName || '').trim().normalize('NFC');
  if (!name || name === '.' || name === '..' || [...name].length > 255 || /[\\/:*?"<>|\u0000-\u001f\u007f-\u009f]/.test(name)) throw new Error('无效的文件夹名称');
  return name;
}
async function createRemoteFolder(parentId, folderName) {
  const response = await apiPost('/userres/v1/file/create_dir', { parentId: String(parentId || ''), dirName: validateFolderName(folderName), failIfNameExist: true });
  return response.data || {};
}
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
    if (history.get(historyKey) === stamp || pendingUploads.has(historyKey) || inflight.get(historyKey) === stamp || (waiting && `${waiting.size}:${waiting.mtime}` === stamp) || waitingFiles.has(historyKey)) return json(response, 200, { queued: 0, skipped: 1, fileName });
    queue.set(historyKey, item);
    queued = true;
    publish({ type: 'file', state: 'queued', file_path: uploadEventPath(item), mapping_id: mappingId, uploaded_bytes: 0, total_bytes: item.size });
    pump();
    return json(response, 202, { queued: 1, skipped: 0, fileName });
  } finally {
    if (!queued) await fsp.rm(temporaryRoot, { recursive: true, force: true });
  }
}
async function routeApiV2(request, response, url) {
  if (request.method === 'POST' && url.pathname === '/api/access/code') {
    const body = await readBody(request);
    if (accessControl.required() && !(await accessControl.verifyCode(body.current_code))) throw new Error('当前访问码错误');
    const expiredCookie = accessControl.updateCode(request, body.new_code ?? body.code ?? body.access_code);
    response.writeHead(200, {
      'content-type': 'application/json; charset=utf-8',
      'cache-control': 'no-store',
      'set-cookie': expiredCookie,
    });
    response.end(JSON.stringify({ required: true, authenticated: false, changed: true }));
    return;
  }
  if (request.method === 'GET' && url.pathname === '/api/settings') return json(response, 200, settingsState());
  if (request.method === 'POST' && url.pathname === '/api/settings/transfer') {
    const body = await readBody(request);
    const transfer = updateTransferSettings(body);
    return json(response, 200, { ...transfer, transfer });
  }
  if (request.method === 'GET' && url.pathname === '/api/settings/cache') return json(response, 200, cacheSettings());
  if (request.method === 'POST' && url.pathname === '/api/settings/cache') {
    const body = await readBody(request);
    return json(response, 200, updateCacheSettings(body));
  }
  if (request.method === 'GET' && url.pathname === '/api/cache') return json(response, 200, cacheState());
  if (request.method === 'POST' && url.pathname === '/api/cache/clear') return json(response, 200, clearManagedCaches());
  if (request.method === 'GET' && url.pathname === '/api/state') return json(response, 200, state());
  if (request.method === 'GET' && url.pathname === '/api/events') { response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' }); response.write(`data: ${JSON.stringify({ type: 'state', state: state() })}\n\n`); clients.add(response); request.on('close', () => clients.delete(response)); return; }
  if (request.method === 'POST' && url.pathname === '/api/auth/device/start') return json(response, 200, await startDeviceLogin());
  if (request.method === 'POST' && url.pathname === '/api/auth/device/poll') { const body = await readBody(request); return json(response, 200, await pollDeviceLogin(body.device_code)); }
  if (request.method === 'POST' && url.pathname === '/api/auth/sms/send') { const body = await readBody(request); return json(response, 200, await sendSmsLogin(body)); }
  if (request.method === 'POST' && url.pathname === '/api/auth/sms/login') { const body = await readBody(request); return json(response, 200, await completeSmsLogin(body)); }
  if (request.method === 'POST' && url.pathname === '/api/auth') { const body = await readBody(request); token = String(body.token || '').trim().replace(/^Bearer\s+/i, '') || null; refreshToken = null; replaceAuthSession(token, null); publishState(); pump(); schedulePendingUploadRecovery(0); return json(response, 200, state()); }
  if (request.method === 'GET' && url.pathname === '/api/overview') return json(response, 200, await apiOverview());
  if (request.method === 'GET' && url.pathname === '/api/files') return json(response, 200, await apiPost('/userres/v1/file/get_file_list', { page: Number(url.searchParams.get('page') || 0), pageSize: 100, parentId: url.searchParams.get('parentId') || '', orderBy: 0, sortType: 0, needSubFolderStat: true }));
  if (request.method === 'GET' && url.pathname === '/api/search') return json(response, 200, await searchCloudFiles(url));
  if (request.method === 'POST' && url.pathname === '/api/upload') return handleWebUpload(request, response, url);
  if (request.method === 'GET' && url.pathname === '/api/server-files') {
    if (!token) throw new Error('请先登录光鸭云盘');
    return json(response, 200, await listServerDirectory(url.searchParams.get('path') || ''));
  }
  if (request.method === 'POST' && url.pathname === '/api/server-upload') {
    if (!token) throw new Error('请先登录光鸭云盘');
    const body = await readBody(request, { maxBytes: 1024 * 1024 });
    return json(response, 200, await queueServerUploads(body.paths, body.parent_id));
  }
  if (request.method === 'POST' && url.pathname === '/api/files/create-folder') { const body = await readBody(request); return json(response, 200, await createRemoteFolder(body.parent_id, body.folder_name)); }
  if (request.method === 'POST' && url.pathname === '/api/files/copy') { const body = await readBody(request); const result = await apiPost('/userres/v1/file/copy_file', { fileIds: validateFileIds(body.file_ids), parentId: String(body.parent_id || '') }); await waitOperation(result.data?.taskId); return json(response, 200, result.data || {}); }
  if (request.method === 'POST' && url.pathname === '/api/files/move') { const body = await readBody(request); const result = await apiPost('/userres/v1/file/move_file', { fileIds: validateFileIds(body.file_ids), parentId: String(body.parent_id || '') }); await waitOperation(result.data?.taskId); return json(response, 200, result.data || {}); }
  if (request.method === 'POST' && url.pathname === '/api/files/delete') { const body = await readBody(request); const result = await apiPost('/userres/v1/file/delete_file', { fileIds: validateFileIds(body.file_ids) }); await waitOperation(result.data?.taskId); return json(response, 200, result.data || {}); }
  if (request.method === 'POST' && url.pathname === '/api/files/rename-batch') { const body = await readBody(request); return json(response, 200, await batchRename(body.renames)); }
  if (request.method === 'POST' && url.pathname === '/api/files/download') { const body = await readBody(request); return json(response, 200, await getCloudDownload(body)); }
  if (request.method === 'POST' && url.pathname === '/api/share') { const body = await readBody(request); return json(response, 200, await createManualShare(body)); }
  if (request.method === 'GET' && url.pathname === '/api/shares') return json(response, 200, await listAllShares());
  if (request.method === 'POST' && url.pathname === '/api/shares/delete') { const body = await readBody(request); const ids = Array.isArray(body.ids) ? body.ids : Array.isArray(body.share_ids) ? body.share_ids : []; if (!ids.length || ids.some((id) => id == null || id === '')) throw new Error('分享记录 ID 无效，请刷新后重试'); const result = await apiPost('/userres/v1/delete_share', { ids }); return json(response, 200, result.data || {}); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/open') { const body = await readBody(request); return json(response, 200, await openReceivedShare(body.url)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/files') { const body = await readBody(request); return json(response, 200, await listReceivedShareFiles(body.access_token, body.parent_id)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/restore') { const body = await readBody(request); return json(response, 200, await restoreReceivedShare(body)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/download') { const body = await readBody(request); return json(response, 200, await getReceivedShareDownload(body)); }
  if (request.method === 'GET' && url.pathname === '/api/offline') return json(response, 200, await apiPost('/cloudcollection/v1/list_task', { page: 0, pageSize: 100 }));
  if (request.method === 'POST' && url.pathname === '/api/offline') { const body = await readBody(request); return json(response, 200, await apiPost('/cloudcollection/v1/create_task', { url: body.url, parentId: body.parent_id || '', newName: body.new_name || '' })); }
  if (request.method === 'GET' && url.pathname === '/api/hdhive/config') return json(response, 200, state().hdhive);
  if (request.method === 'POST' && url.pathname === '/api/hdhive/config') {
    const body = await readBody(request);
    const nextBase = normalizeHdhiveBaseUrl(body.base_url ?? hdhiveBaseUrl);
    hdhiveBaseUrl = nextBase;
    if (typeof body.secret === 'string' && body.secret.trim()) hdhiveSecret = body.secret.trim();
    saveAppStateValue('hdhive_base_url', hdhiveBaseUrl);
    saveAppStateValue('hdhive_secret', hdhiveSecret);
    if (typeof body.enabled === 'boolean') setHdhiveEnabled(body.enabled);
    publishState();
    return json(response, 200, state().hdhive);
  }
  if (request.method === 'POST' && /^\/api\/auto-share\/events\/[^/]+\/retry$/.test(url.pathname)) { const body = await readBody(request); const eventId = decodeURIComponent(url.pathname.split('/')[4]); return json(response, 202, await retryAutoShareEvent(eventId, body)); }
  if (request.method === 'POST' && url.pathname === '/api/share-links') { const body = await readBody(request); const value = { id: crypto.randomUUID(), label: String(body.label || '未命名分享').trim() || '未命名分享', url: String(body.url || '').trim(), created_at: Math.floor(Date.now() / 1000) }; if (!/^https?:\/\//i.test(value.url)) throw new Error('分享链接必须以 http:// 或 https:// 开头'); savedShares.unshift(value); await saveConfig(); publishState(); return json(response, 200, value); }
  if (request.method === 'DELETE' && url.pathname.startsWith('/api/share-links/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); savedShares = savedShares.filter((item) => item.id !== id); await saveConfig(); publishState(); return json(response, 200, {}); }
  if (request.method === 'POST' && url.pathname === '/api/mappings') { const body = await readBody(request); const localPath = allowedPath(body.local_path); const sourcePolicy = ['keep', 'archive', 'delete'].includes(body.source_policy) ? body.source_policy : 'keep'; const archivePath = sourcePolicy === 'archive' ? allowedArchivePath(body.archive_path || archiveRoot) : null; if (archivePath && (archivePath === localPath || archivePath.startsWith(`${localPath}${path.sep}`))) throw new Error('归档目录不能位于被监控目录内部'); if (body.auto_share && (!hdhiveBaseUrl || !hdhiveSecret)) throw new Error('开启自动分享前请先配置 Hdhive 地址和密钥'); const mapping = { id: crypto.randomUUID(), local_path: localPath, remote_path: normalizeRemote(body.remote_path), remote_parent_id: String(body.remote_parent_id || ''), enabled: true, source_policy: sourcePolicy, archive_path: archivePath, scan_existing: body.scan_existing !== false, sync_types: normalizeSyncTypes(body.sync_types), monitor_mode: normalizeMonitorMode(body.monitor_mode), auto_share: body.auto_share === true, watch_error: null }; const stat = await fsp.stat(mapping.local_path); if (!stat.isDirectory()) throw new Error('监控路径不是目录'); mappings.push(mapping); await fsp.mkdir(archiveRoot, { recursive: true }); await saveConfig(); try { await startWatcher(mapping); } catch (error) { mappings = mappings.filter((item) => item.id !== mapping.id); await saveConfig(); throw new Error(`创建目录监控失败：${error.message}`); } publishState(); return json(response, 200, mapping); }
  if (request.method === 'DELETE' && url.pathname.startsWith('/api/mappings/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); await watchers.get(id)?.close(); watchers.delete(id); mappings = mappings.filter((item) => item.id !== id); for (const [key, item] of queue) if (item.mapping_id === id) queue.delete(key); for (const [key, item] of waitingFiles) if (item.mapping_id === id) waitingFiles.delete(key); for (const key of flashPreflightCache.keys()) if (key.startsWith(`${id}::`)) flashPreflightCache.delete(key); for (const key of history.keys()) if (key.startsWith(`${id}::`)) history.delete(key); for (const key of inflight.keys()) if (key.startsWith(`${id}::`)) inflight.delete(key); deleteMappingTransientUploads(id); await saveConfig(); publishState(); return json(response, 200, {}); }
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
      const existing = await collectExistingFiles(mapping, mapping.local_path);
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

await fsp.mkdir(dataDir, { recursive: true }); await fsp.mkdir(manualUploadRoot, { recursive: true }); await cleanupUnreferencedManualUploads(); await fsp.mkdir(watchRoot, { recursive: true }); await fsp.mkdir(archiveRoot, { recursive: true });
try { const config = JSON.parse(await fsp.readFile(configFile, 'utf8')); mappings = Array.isArray(config.mappings) ? config.mappings.map((item) => ({ source_policy: 'keep', archive_path: null, scan_existing: true, remote_parent_id: '', sync_types: DEFAULT_SYNC_TYPES, monitor_mode: 'native', auto_share: false, watch_error: null, ...item, local_path: allowedPath(item.local_path), archive_path: item.archive_path ? allowedArchivePath(item.archive_path) : null, sync_types: normalizeSyncTypes(item.sync_types), monitor_mode: normalizeMonitorMode(item.monitor_mode), auto_share: item.auto_share === true })) : []; savedShares = Array.isArray(config.saved_shares) ? config.saved_shares : []; } catch { mappings = []; savedShares = []; }
restoreUploadCheckpoints();
await restartWatchers();
restorePendingAutoShares();
resumeHdhiveReceiptPolling();
pump();
const server = http.createServer(async (request, response) => {
  const url = new URL(request.url, `http://${request.headers.host || 'localhost'}`);
  if (!enforceLoopbackHost(request, response) || !enforceMutationOrigin(request, response)) return;
  try {
    if (request.method === 'GET' && url.pathname === '/api/access/status') {
      response.setHeader('cache-control', 'no-store');
      return json(response, 200, accessControl.status(request));
    }
    if (request.method === 'POST' && url.pathname === '/api/access/unlock') {
      const body = await readBody(request, { maxBytes: 4 * 1024 });
      const result = await accessControl.unlock(request, body.code ?? body.access_code);
      const headers = { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' };
      if (result.cookie) headers['set-cookie'] = result.cookie;
      if (result.retryAfterSeconds) headers['retry-after'] = String(result.retryAfterSeconds);
      response.writeHead(result.status, headers);
      response.end(JSON.stringify(result.payload));
      return;
    }
    const authorization = await accessControl.authenticate(request);
    if (!authorization.ok) {
      const acceptsHtml = String(request.headers.accept || '').includes('text/html');
      if (request.method === 'GET' && !url.pathname.startsWith('/api/') && acceptsHtml) return accessControl.serveGate(response);
      return accessControl.reject(response, authorization);
    }
    if (url.pathname.startsWith('/api/')) await routeApiV2(request, response, url);
    else await serveStatic(response, url);
  }
  catch (error) {
    const statusCode = Number.isInteger(error.statusCode) ? error.statusCode : 400;
    json(response, statusCode, { error: error.message }, error.headers || {});
  }
});
server.requestTimeout = requestTimeoutMs;
server.headersTimeout = Math.min(requestTimeoutMs, 15_000);
server.listen(port, listenHost, async () => {
  const displayHost = listenHost.includes(':') ? `[${listenHost}]` : listenHost;
  console.log(`Guangya Web listening on http://${displayHost}:${port}, file roots: ${fileRoots.join(', ')}, uploads: ${uploadConcurrency}, multipart: ${multipartMode}, OSS timeout: ${ossTimeoutMs}ms, retries: ${ossRetryMax}, parallel: ${ossParallel}, cloud confirm timeout: ${cloudConfirmTimeoutMs}ms, admin auth: ${accessControl.required() ? `enabled (${adminUsername})` : 'disabled (loopback only)'}`);
  if (refreshToken) {
    try { await refreshSavedSession(); }
    catch (error) { status('warning', `已恢复上次登录，但刷新会话失败：${error.message}`); }
  }
  if (token) schedulePendingUploadRecovery(0);
  if (process.env.SELF_TEST === '1') {
    const selfTestHeaders = adminPassword ? { authorization: `Basic ${Buffer.from(`${adminUsername}:${adminPassword}`).toString('base64')}` } : {};
    const response = await fetch(`http://127.0.0.1:${port}/api/state`, { headers: selfTestHeaders });
    console.log(`SELF_TEST ${response.status} ${await response.text()}`);
    await new Promise((resolve) => server.close(resolve));
    for (const watcher of watchers.values()) await watcher.close();
    process.exit(0);
  }
});

setInterval(() => {
  if (!refreshToken) return;
  void refreshSavedSession().catch((error) => {
    status('warning', `自动续期失败，将稍后重试：${error.message}`);
  });
}, tokenRefreshIntervalMs);
