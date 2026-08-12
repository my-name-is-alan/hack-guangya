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
import { fetch as undiciFetch } from 'undici';
import { autoShareTargetFor, shareFilePayload, signHdhiveRequest } from './auto-share.mjs';
import { createAccessControl } from './access-control.mjs';
import {
  accountIdFromAuthPayload,
  createAuthSessionScopeStore,
  jwtAccountIdentity,
} from './auth-session-scope.mjs';
import { createDirectoryCache } from './directory-cache.mjs';
import {
  createGcidExportDiagnostics,
  gcidDiagnosticLogPath,
  readGcidExportDiagnosticLog,
  sanitizeGcidDiagnosticText,
} from './gcid-export-diagnostics.mjs';
import { createRecycleClearTaskCoordinator } from './recycle-clear-task.mjs';
import {
  calculateGuangyaCidSamples,
  calculateGuangyaFileHashes,
  cidByteRanges,
} from './guangya-file-hashes.mjs';
import {
  GCID_EXPORT_FILE_CONCURRENCY,
  GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY,
  GCID_EXPORT_READ_IDLE_TIMEOUT_MS,
  GCID_EXPORT_RANGE_CONCURRENCY,
  GCID_EXPORT_RANGE_ATTEMPTS,
  GCID_EXPORT_REQUEST_TIMEOUT_MS,
  GCID_EXPORT_SCAN_CONCURRENCY,
  GCID_EXPORT_SCAN_ATTEMPTS,
  GcidExportRangeError,
  createGcidExportRangeGate,
  readGcidExportRangeBody,
  retryableGcidExportRangeStatus,
  retryGcidExportRange,
  retryGcidExportScan,
} from './gcid-export-retry.mjs';
import {
  buildAccountHeaders,
  buildBusinessHeaders,
  businessResponseCode,
  isAuthExpiredBusinessCode,
  isUploadSecurityTokenExpired,
  resolveGuangyaProfile,
  uploadCredentialsExpired,
} from './guangya-protocol.mjs';
import { createGuangyaDeveloperClient, DeveloperApiError } from './guangya-developer.mjs';
import { createNativeMountManager, normalizeNativeMountOptions } from './native-mount.mjs';
import {
  createProxiedFetch,
  networkPreferencesPublic,
  normalizeNetworkPreferences,
  testNetworkTarget,
} from './network-preferences.mjs';
import { createOrganizerService } from './organizer.mjs';
import {
  invalidateRemoteDirectoryIds as invalidateRemoteDirectoryIdsFromCache,
  reconcileRemoteDirectoryCache as reconcileRemoteDirectoryCacheEntries,
} from './remote-directory-cache.mjs';
import { uploadPartSize } from './upload-parts.mjs';
import {
  createUploadReplacementContext,
  restorePreviousUploadRecord,
  safelyReplaceUploadedFile,
  uploadRemoteName,
} from './upload-replacement.mjs';
import { createVirtualLibraryService } from './virtual-library.mjs';
import { createWebDavHandler, normalizeWebDavEntry, WebDavError } from './webdav.mjs';
import { parseGuangyaShareLink } from '../ui/shareLink.js';
import {
  chunkDeveloperPreAuditFileIds,
  createDeveloperPreAuditBatch,
  decodeDeveloperPreAuditPlan,
  encodeDeveloperPreAuditPlan,
  finalizeDeveloperPreAuditSummary,
} from '../shared/developer-pre-audit.mjs';

const here = path.dirname(fileURLToPath(import.meta.url));
const uiRoot = path.resolve(here, '..', 'dist');
const port = Number(process.env.PORT || 8080);
const adminUsername = String(process.env.GUANGYA_ADMIN_USERNAME || 'admin');
const adminPassword = String(process.env.GUANGYA_ADMIN_PASSWORD || '');
const requestedListenHost = String(process.env.LISTEN_HOST || process.env.HOST || (adminPassword ? '0.0.0.0' : '127.0.0.1')).trim();
const loopbackHosts = new Set(['127.0.0.1', '::1', 'localhost']);
if (!adminPassword && !loopbackHosts.has(requestedListenHost.toLowerCase())) throw new Error('未配置 GUANGYA_ADMIN_PASSWORD 时只允许监听回环地址');
const listenHost = requestedListenHost;
const defaultWebDavPort = process.env.NODE_TEST_CONTEXT ? 0 : 19090;
const webdavPort = Number(process.env.GUANGYA_WEBDAV_PORT ?? defaultWebDavPort);
if (!Number.isInteger(webdavPort) || webdavPort < 0 || webdavPort > 65535) throw new Error('GUANGYA_WEBDAV_PORT 必须是 0 到 65535 的整数');
const webdavPublicPort = Number(process.env.GUANGYA_WEBDAV_PUBLIC_PORT || webdavPort);
if (!Number.isInteger(webdavPublicPort) || webdavPublicPort < 0 || webdavPublicPort > 65535) throw new Error('GUANGYA_WEBDAV_PUBLIC_PORT 必须是 0 到 65535 的整数');
const requestedWebDavHost = String(process.env.GUANGYA_WEBDAV_HOST || '127.0.0.1').trim();
const allowWebDavNonLoopback = process.env.GUANGYA_WEBDAV_ALLOW_NON_LOOPBACK === '1';
if (!loopbackHosts.has(requestedWebDavHost.toLowerCase()) && !allowWebDavNonLoopback) {
  throw new Error('WebDAV 默认只允许监听回环地址；容器内部监听需显式设置 GUANGYA_WEBDAV_ALLOW_NON_LOOPBACK=1');
}
const webdavHost = requestedWebDavHost;
const webdavEndpoint = `http://127.0.0.1:${webdavPublicPort}/dav/`;
const defaultEmbyProxyPort = process.env.NODE_TEST_CONTEXT ? 0 : 18096;
const embyProxyPort = Number(process.env.GUANGYA_EMBY_PROXY_PORT ?? defaultEmbyProxyPort);
if (!Number.isInteger(embyProxyPort) || embyProxyPort < 0 || embyProxyPort > 65535) throw new Error('GUANGYA_EMBY_PROXY_PORT 必须是 0 到 65535 的整数');
const embyProxyPublicPort = Number(process.env.GUANGYA_EMBY_PROXY_PUBLIC_PORT || embyProxyPort);
if (!Number.isInteger(embyProxyPublicPort) || embyProxyPublicPort < 0 || embyProxyPublicPort > 65535) throw new Error('GUANGYA_EMBY_PROXY_PUBLIC_PORT 必须是 0 到 65535 的整数');
const requestedEmbyProxyHost = String(process.env.GUANGYA_EMBY_PROXY_HOST || '127.0.0.1').trim();
const allowEmbyProxyNonLoopback = process.env.GUANGYA_EMBY_PROXY_ALLOW_NON_LOOPBACK === '1';
if (!loopbackHosts.has(requestedEmbyProxyHost.toLowerCase()) && !allowEmbyProxyNonLoopback) {
  throw new Error('Emby 代理端口默认只允许监听回环地址；容器内部监听需显式设置 GUANGYA_EMBY_PROXY_ALLOW_NON_LOOPBACK=1');
}
const embyProxyHost = requestedEmbyProxyHost;
const embyUpstream = String(process.env.GUANGYA_EMBY_UPSTREAM || 'http://127.0.0.1:8096').trim();
const configuredDataDir = path.resolve(process.env.DATA_DIR || path.join(here, '..', '.web-data'));
const configuredWatchRoot = path.resolve(process.env.GUANGYA_WATCH_ROOT || path.join(here, '..', 'watch'));
const configuredArchiveRoot = path.resolve(process.env.GUANGYA_ARCHIVE_ROOT || path.join(here, '..', 'archive'));
const configuredVirtualLibraryRoot = path.resolve(process.env.GUANGYA_VIRTUAL_LIBRARY_ROOT || path.join(here, '..', 'virtual-library'));
for (const directory of [configuredDataDir, configuredWatchRoot, configuredArchiveRoot, configuredVirtualLibraryRoot]) fs.mkdirSync(directory, { recursive: true });
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
const virtualLibraryRoot = canonicalizePathSync(configuredVirtualLibraryRoot);
const fileRoots = (process.env.GUANGYA_FILE_ROOTS || watchRoot).split(',').map((value) => value.trim()).filter(Boolean).map(canonicalizePathSync);
const configFile = path.join(dataDir, 'config.json');
const databaseFile = path.join(dataDir, 'state.sqlite3');
const gcidExportDiagnosticFile = gcidDiagnosticLogPath(dataDir);
const manualUploadRoot = path.join(dataDir, 'manual-uploads');
const apiBase = process.env.GUANGYA_API_BASE || 'https://api.guangyapan.com';
const accountBase = process.env.GUANGYA_ACCOUNT_BASE || 'https://account.guangyapan.com';
const guangyaProfile = resolveGuangyaProfile();
const oauthClientId = guangyaProfile.clientId;
const oauthClientSecret = guangyaProfile.clientSecret;
function envInteger(name, fallback, minimum, maximum) { const parsed = Number(process.env[name]); return Number.isFinite(parsed) ? Math.max(minimum, Math.min(maximum, Math.round(parsed))) : fallback; }
function normalizeDeviceId(value) {
  const compact = String(value || '').trim().toLowerCase().replaceAll('-', '');
  return /^[a-f0-9]{32}$/.test(compact) ? compact : crypto.randomBytes(16).toString('hex');
}
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
const maxShareTrafficBytes = 1024 * 1024 ** 4;
const requestTimeoutMs = envInteger('GUANGYA_REQUEST_TIMEOUT_MS', 30_000, 5_000, 120_000);
const gcidExportSnapshotFreshMs = 10 * 60_000;
const gcidExportInventoryPageSize = 1_000;
const gcidExportInventoryThreshold = 500;
const fileListRequestTimeoutMs = Math.min(requestTimeoutMs, 12_000);
const recycleClearDeadlineMs = envInteger('GUANGYA_RECYCLE_CLEAR_DEADLINE_MS', 120_000, 1_000, 300_000);
const recycleClearPollMs = envInteger('GUANGYA_RECYCLE_CLEAR_POLL_MS', 1_000, 10, 5_000);
const recycleClearUnknownGuardMs = envInteger('GUANGYA_RECYCLE_CLEAR_UNKNOWN_GUARD_MS', 120_000, 1_000, 600_000);
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
    cid TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL
  );
  CREATE TABLE IF NOT EXISTS gcid_export_snapshots (
    account_scope TEXT NOT NULL,
    selection_key TEXT NOT NULL,
    root_signatures_json TEXT NOT NULL,
    export_json TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL,
    PRIMARY KEY (account_scope, selection_key)
  );
  CREATE TABLE IF NOT EXISTS gcid_export_file_hashes (
    account_scope TEXT NOT NULL,
    file_id TEXT NOT NULL,
    file_size TEXT NOT NULL,
    gcid TEXT NOT NULL,
    cid TEXT NOT NULL,
    last_used_at INTEGER NOT NULL,
    PRIMARY KEY (account_scope, file_id, file_size, gcid)
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
    error_code TEXT,
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
  CREATE TABLE IF NOT EXISTS developer_targets (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    token_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
  );
  CREATE TABLE IF NOT EXISTS developer_transfer_jobs (
    id TEXT PRIMARY KEY,
    target_id TEXT NOT NULL,
    target_name TEXT NOT NULL,
    file_ids_json TEXT NOT NULL,
    file_names_json TEXT NOT NULL,
    status TEXT NOT NULL,
    phase TEXT NOT NULL,
    pre_task_id TEXT,
    upload_task_id TEXT,
    total_count INTEGER NOT NULL DEFAULT 0,
    passed_count INTEGER NOT NULL DEFAULT 0,
    rejected_count INTEGER NOT NULL DEFAULT 0,
    pending_count INTEGER NOT NULL DEFAULT 0,
    success_count INTEGER NOT NULL DEFAULT 0,
    skipped_count INTEGER NOT NULL DEFAULT 0,
    work_total_count INTEGER NOT NULL DEFAULT 0,
    processed_count INTEGER NOT NULL DEFAULT 0,
    current_path TEXT NOT NULL DEFAULT '',
    error_code INTEGER,
    message TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
  );
  CREATE TABLE IF NOT EXISTS developer_transfer_name_restores (
    job_id TEXT NOT NULL,
    file_id TEXT NOT NULL,
    original_name TEXT NOT NULL,
    temporary_name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    PRIMARY KEY (job_id, file_id)
  );
  CREATE TABLE IF NOT EXISTS offline_name_restores (
    task_id TEXT PRIMARY KEY,
    original_name TEXT NOT NULL,
    temporary_name TEXT NOT NULL,
    file_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
  );
  CREATE INDEX IF NOT EXISTS developer_transfer_jobs_status
    ON developer_transfer_jobs(status, updated_at);
  CREATE INDEX IF NOT EXISTS developer_transfer_name_restores_status
    ON developer_transfer_name_restores(status, file_id, updated_at);
  CREATE INDEX IF NOT EXISTS offline_name_restores_status
    ON offline_name_restores(status, updated_at);
`);
if (!database.prepare("PRAGMA table_info(auth_session)").all().some((column) => column.name === 'refresh_token')) database.exec('ALTER TABLE auth_session ADD COLUMN refresh_token TEXT');
if (!database.prepare("PRAGMA table_info(auto_share_events)").all().some((column) => column.name === 'notification_status')) database.exec('ALTER TABLE auto_share_events ADD COLUMN notification_status TEXT');
if (!database.prepare("PRAGMA table_info(auto_share_events)").all().some((column) => column.name === 'error_code')) database.exec('ALTER TABLE auto_share_events ADD COLUMN error_code TEXT');
if (!database.prepare("PRAGMA table_info(file_fingerprints)").all().some((column) => column.name === 'cid')) database.exec("ALTER TABLE file_fingerprints ADD COLUMN cid TEXT NOT NULL DEFAULT ''");
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
function deleteAppStateValue(key) {
  database.prepare('DELETE FROM app_state WHERE key = ?').run(key);
}
for (const [column, definition] of [
  ['work_total_count', 'INTEGER NOT NULL DEFAULT 0'],
  ['processed_count', 'INTEGER NOT NULL DEFAULT 0'],
  ['current_path', "TEXT NOT NULL DEFAULT ''"],
]) {
  if (!database.prepare("PRAGMA table_info(developer_transfer_jobs)").all().some((entry) => entry.name === column)) {
    database.exec(`ALTER TABLE developer_transfer_jobs ADD COLUMN ${column} ${definition}`);
  }
}
let networkPreferences = normalizeNetworkPreferences({
  proxy_url: appStateValue('network_proxy'),
  github_proxy: appStateValue('network_proxy_github'),
  tmdb_proxy: appStateValue('network_proxy_tmdb'),
  tg_proxy: appStateValue('network_proxy_tg'),
}, {});
function publicNetworkPreferences() {
  return networkPreferencesPublic(networkPreferences);
}
function updateNetworkPreferences(input = {}) {
  const next = normalizeNetworkPreferences(input, networkPreferences);
  networkPreferences = next;
  // Keep the legacy keys in sync so an older binary can still start with the
  // same proxy after an upgrade or rollback. All new requests use this one
  // canonical value.
  saveAppStateValue('network_proxy', next.proxy_url);
  saveAppStateValue('network_proxy_github', next.proxy_url);
  saveAppStateValue('network_proxy_tmdb', next.proxy_url);
  saveAppStateValue('network_proxy_tg', next.proxy_url);
  return publicNetworkPreferences();
}
function normalizeWebDavUsername(value) {
  const normalized = String(value || '').trim();
  if (normalized.length < 3 || normalized.length > 64 || normalized.includes(':') || /[\u0000-\u001f\u007f]/.test(normalized)) {
    throw new Error('WebDAV 用户名必须为 3 到 64 个字符，且不能包含冒号或控制字符');
  }
  return normalized;
}
function normalizeWebDavPassword(value) {
  const normalized = String(value ?? '');
  if (normalized.length < 12 || normalized.length > 256) throw new Error('WebDAV 密码必须为 12 到 256 个字符');
  return normalized;
}
let webdavUsername = normalizeWebDavUsername(
  appStateValue('webdav_username') || process.env.GUANGYA_WEBDAV_USERNAME || 'guangya',
);
const initialWebDavPassword = String(process.env.GUANGYA_WEBDAV_PASSWORD || '');
if (initialWebDavPassword) normalizeWebDavPassword(initialWebDavPassword);
const webdavAccessControl = createAccessControl({
  database,
  initialCode: initialWebDavPassword,
  username: webdavUsername,
  tableName: 'webdav_access_control',
  realm: 'Guangya WebDAV',
});
saveAppStateValue('webdav_username', webdavUsername);
let storedNativeMountOptions = {};
try {
  storedNativeMountOptions = JSON.parse(appStateValue('native_mount_options') || '{}');
} catch {}
if (!storedNativeMountOptions.target && process.env.GUANGYA_NATIVE_MOUNT_TARGET) {
  storedNativeMountOptions.target = String(process.env.GUANGYA_NATIVE_MOUNT_TARGET);
}
const nativeMountEnabled = process.env.GUANGYA_NATIVE_MOUNT_ENABLED === '1' || !fs.existsSync('/.dockerenv');
const nativeMountManager = createNativeMountManager({
  dataDir,
  initialOptions: storedNativeMountOptions,
  enabled: nativeMountEnabled,
});
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
let offlineFilenameObfuscationEnabled = appStateValue('offline_filename_obfuscation') === 'true';
let hdhiveGeneration = 0;
saveAppStateValue('transfer_upload_concurrency', uploadConcurrency);
saveAppStateValue('transfer_download_concurrency', downloadConcurrency);
saveAppStateValue('transfer_multipart', multipartMode);
saveAppStateValue('cache_enabled', cacheEnabled);
saveAppStateValue('cache_max_entries', cacheMaxEntries);
saveAppStateValue('hdhive_enabled', hdhiveEnabled);
const storedDevice = database.prepare("SELECT value FROM app_state WHERE key = 'device_id'").get();
const deviceId = normalizeDeviceId(storedDevice?.value);
database.prepare("INSERT INTO app_state (key, value, updated_at) VALUES ('device_id', ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at").run(deviceId, Math.floor(Date.now() / 1000));
const clients = new Set();
const developerTransferRunning = new Set();
let developerNameMutationChain = Promise.resolve();
const watchers = new Map();
const queue = new Map();
const flashPreflightCache = new Map();
const history = new Map(database.prepare("SELECT mapping_id, file_path, size, modified_ms FROM uploaded_files WHERE status = 'cloud_confirmed'").all().map((row) => [`${row.mapping_id}::${path.resolve(row.file_path)}`, `${row.size}:${row.modified_ms}`]));
const pendingUploads = new Map(database.prepare("SELECT mapping_id, file_path, size, modified_ms, task_id, item_json, remote_parent_id, remote_dir, relative_path FROM uploaded_files WHERE status = 'oss_complete'").all().map((row) => [`${row.mapping_id}::${path.resolve(row.file_path)}`, row]));
const activeUploadReplacements = new Map();
const inflight = new Map();
const inflightItems = new Map();
const waitingFiles = new Map();
const failedUploads = new Map();
const cancelledUploads = new Map();
const pausedUploads = new Set();
const queuePauseRequests = new Set();
const activeUploadClients = new Map();
const remoteCache = new Map([['', '']]);
const remoteCacheValidatedAt = new Map([['', Number.POSITIVE_INFINITY]]);
const remoteCacheGates = new Map();
let remoteCacheGeneration = 0;
const remoteDirectoryFreshMs = 15_000;
function resetRemoteDirectoryCache() {
  remoteCacheGeneration += 1;
  remoteCache.clear();
  remoteCache.set('', '');
  remoteCacheValidatedAt.clear();
  remoteCacheValidatedAt.set('', Number.POSITIVE_INFINITY);
}
function cleanupRemoteDirectoryCacheMetadata() {
  for (const key of remoteCacheValidatedAt.keys()) {
    if (!remoteCache.has(key)) remoteCacheValidatedAt.delete(key);
  }
}
function invalidateRemoteDirectoryIds(fileIds) {
  const removed = invalidateRemoteDirectoryIdsFromCache(remoteCache, fileIds);
  // 只有真的移除了条目才推进 generation：无条件自增会让每一次后台目录刷新
  // 都打断正在进行的 ensureRemote 路径解析（上限 8 次后直接报错）。
  if (removed > 0) remoteCacheGeneration += 1;
  cleanupRemoteDirectoryCacheMetadata();
  return removed;
}
function reconcileRemoteDirectoryCache(parentId, records, { complete = false } = {}) {
  const checkedAt = Date.now();
  const removed = reconcileRemoteDirectoryCacheEntries(remoteCache, parentId, records, {
    complete,
    onConfirmed: (key) => remoteCacheValidatedAt.set(key, checkedAt),
  });
  if (removed > 0) remoteCacheGeneration += 1;
  cleanupRemoteDirectoryCacheMetadata();
  return removed;
}
if (cacheEnabled) trimManagedCaches();
else clearManagedCaches();
const pendingAutoShares = new Map();
let mappings = [];
let savedShares = [];
const storedAuth = database.prepare('SELECT access_token, refresh_token FROM auth_session WHERE id = 1').get();
let token = process.env.GUANGYA_TOKEN || storedAuth?.access_token || null;
let refreshToken = storedAuth?.refresh_token || null;
let refreshPromise = null;
const authSessionScope = createAuthSessionScopeStore({
  loadValue: () => appStateValue('auth_session_scope_v1'),
  saveValue: (value) => saveAppStateValue('auth_session_scope_v1', value),
  issuer: accountBase,
});
authSessionScope.initialize(token);
const smsChallenges = new Map();
let paused = false;
let active = 0;
let activeFlashPreflights = 0;
const flashPreflightConcurrency = 1;
const flashPreflightTokenMaxAgeMs = 10 * 60 * 1000;
const fileStabilityMs = Math.max(200, Number(process.env.GUANGYA_FILE_STABILITY_MS || 5000));
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
function autoShareReceipts() { return database.prepare('SELECT event_id, mapping_id, target_key, share_url, status, action, error_code, message, resource_url, notification_status, updated_at FROM auto_share_events ORDER BY updated_at DESC LIMIT 50').all(); }
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
function pendingOfflineNameRestoreCount() {
  return Number(database.prepare("SELECT COUNT(*) AS count FROM offline_name_restores WHERE status = 'pending'").get()?.count || 0);
}
function offlineSettings() {
  return {
    filename_obfuscation_enabled: offlineFilenameObfuscationEnabled,
    pending_restores: pendingOfflineNameRestoreCount(),
  };
}
function updateOfflineSettings(body) {
  const requested = body.filename_obfuscation_enabled ?? body.filenameObfuscationEnabled;
  if (typeof requested !== 'boolean') throw new Error('文件名混淆开关必须是布尔值');
  offlineFilenameObfuscationEnabled = requested;
  saveAppStateValue('offline_filename_obfuscation', String(requested));
  return offlineSettings();
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
    remoteCacheValidatedAt.delete(key);
    excess -= 1;
  }
}
function trimGcidExportSnapshotCache() {
  database.prepare(`DELETE FROM gcid_export_snapshots
    WHERE rowid IN (
      SELECT rowid FROM gcid_export_snapshots
      ORDER BY last_used_at DESC, rowid DESC
      LIMIT -1 OFFSET ?
    )`).run(cacheMaxEntries);
}
function trimGcidExportFileHashCache() {
  database.prepare(`DELETE FROM gcid_export_file_hashes
    WHERE rowid IN (
      SELECT rowid FROM gcid_export_file_hashes
      ORDER BY last_used_at DESC, rowid DESC
      LIMIT -1 OFFSET ?
    )`).run(cacheMaxEntries);
}
function trimManagedCaches() {
  trimFileFingerprintCache();
  trimGcidExportSnapshotCache();
  trimGcidExportFileHashCache();
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
  const fingerprints = database.prepare('SELECT file_path, modified_ms, gcid, cid FROM file_fingerprints').all();
  const fingerprintBytes = fingerprints.reduce((total, row) => total
    + Buffer.byteLength(String(row.file_path))
    + Buffer.byteLength(String(row.modified_ms))
    + Buffer.byteLength(String(row.gcid))
    + Buffer.byteLength(String(row.cid)) + 24, 0);
  const remoteEntries = [...remoteCache.entries()].filter(([key]) => key !== '');
  const remoteBytes = remoteEntries.reduce((total, [key, value]) => total
    + Buffer.byteLength(String(key)) + Buffer.byteLength(String(value)), 0);
  const snapshots = database.prepare('SELECT root_signatures_json, export_json FROM gcid_export_snapshots').all();
  const snapshotBytes = snapshots.reduce((total, row) => total
    + Buffer.byteLength(String(row.root_signatures_json))
    + Buffer.byteLength(String(row.export_json)) + 64, 0);
  const cloudHashes = database.prepare('SELECT file_id, file_size, gcid, cid FROM gcid_export_file_hashes').all();
  const cloudHashBytes = cloudHashes.reduce((total, row) => total
    + Buffer.byteLength(String(row.file_id))
    + Buffer.byteLength(String(row.file_size))
    + Buffer.byteLength(String(row.gcid))
    + Buffer.byteLength(String(row.cid)) + 64, 0);
  const remoteEntryCount = remoteEntries.length + snapshots.length + cloudHashes.length;
  const remoteTotalBytes = remoteBytes + snapshotBytes + cloudHashBytes;
  return {
    file_fingerprints: { entries: fingerprints.length, size_bytes: fingerprintBytes },
    remote_cache: { entries: remoteEntryCount, size_bytes: remoteTotalBytes },
    gcid_export_snapshots: { entries: snapshots.length, size_bytes: snapshotBytes },
    gcid_export_file_hashes: { entries: cloudHashes.length, size_bytes: cloudHashBytes },
    entries: fingerprints.length + remoteEntryCount,
    bytes: fingerprintBytes + remoteTotalBytes,
    file_fingerprints_entries: fingerprints.length,
    file_fingerprints_bytes: fingerprintBytes,
    remote_cache_entries: remoteEntryCount,
    remote_cache_bytes: remoteTotalBytes,
    total_size_bytes: fingerprintBytes + remoteTotalBytes,
    policy: cacheSettings(),
  };
}
function clearManagedCaches() {
  database.exec(`DELETE FROM file_fingerprints;
    DELETE FROM gcid_export_snapshots;
    DELETE FROM gcid_export_file_hashes;`);
  resetRemoteDirectoryCache();
  // "清理缓存"必须同时打掉挂载端目录快照并通知前端，否则用户看不到效果。
  // 模块初始化早期（webDavDirectoryCache 尚未创建）调用时静默跳过。
  try {
    webDavDirectoryCache.clear();
    publishCloudDirectoryInvalidated([], { all: true, source: 'cache-clear' });
  } catch { /* 启动阶段无快照可清 */ }
  return cacheState();
}

function maskDeveloperValue(value) {
  const normalized = String(value || '');
  if (!normalized) return '';
  if (normalized.length <= 8) return '••••••••';
  return `${normalized.slice(0, 4)}••••${normalized.slice(-4)}`;
}

function normalizeDeveloperSetting(value, label, maximum = 256) {
  const normalized = String(value || '').trim();
  if (!normalized) throw new Error(`${label}不能为空`);
  if (normalized.length > maximum) throw new Error(`${label}不能超过 ${maximum} 个字符`);
  if (/[^\x21-\x7e]/.test(normalized)) throw new Error(`${label}只能包含可见 ASCII 字符`);
  return normalized;
}

function normalizeDeveloperTargetName(value) {
  const normalized = String(value || '').trim();
  if (!normalized) throw new Error('小号名称不能为空');
  if (normalized.length > 64 || /[\u0000-\u001f\u007f]/.test(normalized)) throw new Error('小号名称不能超过 64 个字符或包含控制字符');
  return normalized;
}

function developerCredentials() {
  const environmentClientId = String(process.env.GUANGYA_DEVELOPER_CLIENT_ID || '').trim();
  const environmentClientSecret = String(process.env.GUANGYA_DEVELOPER_CLIENT_SECRET || '').trim();
  return {
    clientId: environmentClientId || String(appStateValue('developer_client_id') || '').trim(),
    clientSecret: environmentClientSecret || String(appStateValue('developer_client_secret') || '').trim(),
    clientIdFromEnvironment: Boolean(environmentClientId),
    clientSecretFromEnvironment: Boolean(environmentClientSecret),
  };
}

function accountIdFromProfile(payload) {
  const data = payload?.data ?? payload;
  const profile = data?.user ?? data?.profile ?? data;
  for (const key of ['sub', 'userId', 'user_id', 'id']) {
    const value = String(profile?.[key] ?? '').trim();
    if (value) return value;
  }
  return '';
}

async function currentDeveloperAccountId() {
  if (!token) throw new Error('请先登录当前光鸭账号');
  const accountId = accountIdFromProfile(await accountGet('/v1/user/me'));
  if (!accountId) throw new Error('当前登录态没有返回可识别的账号 ID，无法绑定开发者模式');
  return accountId;
}

function developerBinding() {
  return {
    requestedEnabled: appStateValue('developer_mode_enabled') === '1',
    accountId: String(appStateValue('developer_account_id') || '').trim(),
    verifiedClientId: String(appStateValue('developer_verified_client_id') || '').trim(),
    verifiedAt: Number(appStateValue('developer_account_verified_at') || 0) || 0,
  };
}

function developerTargetFromRow(row) {
  return {
    id: String(row.id),
    name: String(row.name),
    token_masked: maskDeveloperValue(row.token_id),
    created_at: Number(row.created_at),
    updated_at: Number(row.updated_at),
  };
}

function developerSettingsState(currentAccountId = '') {
  const credentials = developerCredentials();
  const binding = developerBinding();
  const targets = database.prepare('SELECT id, name, token_id, created_at, updated_at FROM developer_targets ORDER BY updated_at DESC, name').all();
  const accountMatchesCurrent = Boolean(currentAccountId && binding.accountId && currentAccountId === binding.accountId);
  const accountVerified = Boolean(binding.accountId && binding.verifiedAt > 0 && binding.verifiedClientId === credentials.clientId);
  return {
    configured: Boolean(credentials.clientId && credentials.clientSecret),
    enabled: Boolean(binding.requestedEnabled && accountVerified && accountMatchesCurrent && credentials.clientId && credentials.clientSecret),
    requested_enabled: binding.requestedEnabled,
    client_id: credentials.clientId,
    client_secret_set: Boolean(credentials.clientSecret),
    account_id: binding.accountId,
    current_account_id: currentAccountId,
    account_verified: accountVerified,
    account_matches_current: accountMatchesCurrent,
    verified_at: binding.verifiedAt,
    managed_by_environment: credentials.clientIdFromEnvironment || credentials.clientSecretFromEnvironment,
    client_id_managed_by_environment: credentials.clientIdFromEnvironment,
    client_secret_managed_by_environment: credentials.clientSecretFromEnvironment,
    targets: targets.map(developerTargetFromRow),
  };
}

async function developerSettingsForCurrentAccount() {
  let currentAccountId = '';
  try { currentAccountId = await currentDeveloperAccountId(); } catch {}
  return developerSettingsState(currentAccountId);
}

function updateDeveloperCredentials(body) {
  const current = developerCredentials();
  if (body.clear === true) {
    if (current.clientIdFromEnvironment || current.clientSecretFromEnvironment) throw new Error('开发者凭据由环境变量托管，不能在页面中清除');
    saveAppStateValue('developer_client_id', '');
    saveAppStateValue('developer_client_secret', '');
    saveAppStateValue('developer_mode_enabled', '0');
    saveAppStateValue('developer_account_id', '');
    saveAppStateValue('developer_verified_client_id', '');
    saveAppStateValue('developer_account_verified_at', '0');
    return developerSettingsState();
  }
  const clientId = normalizeDeveloperSetting(body.client_id ?? body.clientId ?? current.clientId, '开发者 client_id');
  const requestedSecret = body.client_secret ?? body.clientSecret;
  const clientSecret = requestedSecret == null || String(requestedSecret).trim() === ''
    ? current.clientSecret
    : normalizeDeveloperSetting(requestedSecret, '开发者 client_secret');
  if (!clientSecret) throw new Error('首次配置时必须填写开发者 client_secret');
  if (current.clientIdFromEnvironment && clientId !== current.clientId) throw new Error('client_id 由 GUANGYA_DEVELOPER_CLIENT_ID 托管');
  if (current.clientSecretFromEnvironment && requestedSecret != null && String(requestedSecret).trim()) throw new Error('client_secret 由 GUANGYA_DEVELOPER_CLIENT_SECRET 托管');
  const credentialsChanged = clientId !== current.clientId || clientSecret !== current.clientSecret;
  if (!current.clientIdFromEnvironment) saveAppStateValue('developer_client_id', clientId);
  if (!current.clientSecretFromEnvironment) saveAppStateValue('developer_client_secret', clientSecret);
  if (credentialsChanged) {
    saveAppStateValue('developer_mode_enabled', '0');
    saveAppStateValue('developer_account_id', '');
    saveAppStateValue('developer_verified_client_id', '');
    saveAppStateValue('developer_account_verified_at', '0');
  }
  return developerSettingsState();
}

function upsertDeveloperTarget(body) {
  const id = body.id == null || String(body.id).trim() === '' ? crypto.randomUUID() : validateIdentifier(body.id, '小号配置 ID');
  const existing = database.prepare('SELECT id, token_id, created_at FROM developer_targets WHERE id = ?').get(id);
  const name = normalizeDeveloperTargetName(body.name);
  const requestedToken = body.token_id ?? body.tokenId;
  const tokenId = requestedToken == null || String(requestedToken).trim() === ''
    ? String(existing?.token_id || '')
    : normalizeDeveloperSetting(requestedToken, '接收 TOKEN');
  if (!tokenId) throw new Error('首次添加小号时必须填写接收 TOKEN');
  const now = Math.floor(Date.now() / 1000);
  database.prepare(`INSERT INTO developer_targets (id, name, token_id, created_at, updated_at)
    VALUES (?, ?, ?, ?, ?)
    ON CONFLICT(id) DO UPDATE SET name = excluded.name, token_id = excluded.token_id, updated_at = excluded.updated_at`)
    .run(id, name, tokenId, Number(existing?.created_at || now), now);
  return developerTargetFromRow(database.prepare('SELECT id, name, token_id, created_at, updated_at FROM developer_targets WHERE id = ?').get(id));
}

function deleteDeveloperTarget(idValue) {
  const id = validateIdentifier(idValue, '小号配置 ID');
  const active = database.prepare("SELECT 1 FROM developer_transfer_jobs WHERE target_id = ? AND status IN ('queued', 'direct', 'auditing', 'copying', 'running') LIMIT 1").get(id);
  if (active) throw new Error('这个小号仍有进行中的互传任务，暂时不能删除');
  const result = database.prepare('DELETE FROM developer_targets WHERE id = ?').run(id);
  if (!result.changes) throw httpError(404, '小号配置不存在');
  return {};
}

function parseJobArray(value) {
  try { return JSON.parse(String(value || '[]')); } catch { return []; }
}

function developerTransferJob(row) {
  if (!row) return null;
  return {
    id: String(row.id),
    target_id: String(row.target_id),
    target_name: String(row.target_name),
    file_ids: parseJobArray(row.file_ids_json),
    file_names: parseJobArray(row.file_names_json),
    status: String(row.status),
    phase: String(row.phase),
    pre_task_id: row.pre_task_id == null ? null : String(row.pre_task_id),
    upload_task_id: row.upload_task_id == null ? null : String(row.upload_task_id),
    total_count: Number(row.total_count || 0),
    passed_count: Number(row.passed_count || 0),
    rejected_count: Number(row.rejected_count || 0),
    pending_count: Number(row.pending_count || 0),
    success_count: Number(row.success_count || 0),
    skipped_count: Number(row.skipped_count || 0),
    work_total_count: Number(row.work_total_count || 0),
    processed_count: Number(row.processed_count || 0),
    current_path: String(row.current_path || ''),
    error_code: row.error_code == null ? null : Number(row.error_code),
    message: row.message == null ? null : String(row.message),
    created_at: Number(row.created_at),
    updated_at: Number(row.updated_at),
  };
}

function loadDeveloperTransferJob(id) {
  return developerTransferJob(database.prepare('SELECT * FROM developer_transfer_jobs WHERE id = ?').get(id));
}

function listDeveloperTransfers(limit = 50) {
  const normalizedLimit = Math.max(1, Math.min(100, Math.floor(Number(limit) || 50)));
  return database.prepare('SELECT * FROM developer_transfer_jobs ORDER BY created_at DESC LIMIT ?').all(normalizedLimit).map(developerTransferJob);
}

function updateDeveloperTransferJob(id, patch) {
  const allowed = new Set([
    'status', 'phase', 'pre_task_id', 'upload_task_id', 'total_count', 'passed_count',
    'rejected_count', 'pending_count', 'success_count', 'skipped_count', 'work_total_count',
    'processed_count', 'current_path', 'error_code', 'message',
  ]);
  const entries = Object.entries(patch).filter(([key, value]) => allowed.has(key) && value !== undefined);
  if (entries.length) {
    const assignments = entries.map(([key]) => `${key} = ?`).join(', ');
    database.prepare(`UPDATE developer_transfer_jobs SET ${assignments}, updated_at = ? WHERE id = ?`)
      .run(...entries.map(([, value]) => value), Math.floor(Date.now() / 1000), id);
  }
  const job = loadDeveloperTransferJob(id);
  if (job) publish({ type: 'developer-transfer', job });
  return job;
}

function developerClient() {
  const credentials = developerCredentials();
  if (!credentials.clientId || !credentials.clientSecret) throw new Error('请先在设置中填写开发者 client_id 和 client_secret');
  return createGuangyaDeveloperClient({
    clientId: credentials.clientId,
    clientSecret: credentials.clientSecret,
    timeoutMs: requestTimeoutMs,
  });
}

async function verifyDeveloperAccountOwnership(probeFileIdValue = '') {
  const currentAccountId = await currentDeveloperAccountId();
  let probeFileId = String(probeFileIdValue || '').trim();
  if (probeFileId) {
    probeFileId = validateIdentifier(probeFileId, '账号校验文件 ID');
    await apiPost('/userres/v1/file/get_file_detail', { fileId: probeFileId });
  } else {
    const payload = await apiPost('/userres/v1/file/get_file_list', {
      parentId: '',
      page: 0,
      pageSize: 1,
      dirType: 0,
      orderBy: 0,
      sortType: 0,
    });
    probeFileId = String(payload?.data?.list?.[0]?.fileId || '').trim();
  }
  if (!probeFileId) {
    throw new Error('当前账号没有可用于所有权校验的文件或目录，请先在根目录创建一个文件夹后重试');
  }
  await developerPostWithRetry(developerClient(), '/userres/v1/file/get_file_detail', { fileId: probeFileId }, 0);
  return { accountId: currentAccountId, probeFileId };
}

async function ensureDeveloperModeForCurrentAccount(probeFileId = '') {
  const binding = developerBinding();
  if (!binding.requestedEnabled) throw new Error('请先在“设置 → 账号”中开启开发者模式');
  const credentials = developerCredentials();
  if (!binding.accountId || binding.verifiedAt <= 0 || binding.verifiedClientId !== credentials.clientId) {
    throw new Error('开发者凭据尚未通过当前账号所有权校验');
  }
  const currentAccountId = await currentDeveloperAccountId();
  if (currentAccountId !== binding.accountId) {
    throw new Error('开发者模式绑定的账号与当前登录账号不一致，请切回原账号或重新验证凭据');
  }
  const client = developerClient();
  if (probeFileId) {
    await developerPostWithRetry(client, '/userres/v1/file/get_file_detail', {
      fileId: validateIdentifier(probeFileId, '账号校验文件 ID'),
    }, 0);
  }
  return client;
}

async function updateDeveloperMode(enabledValue) {
  const enabled = enabledValue === true;
  if (!enabled) {
    saveAppStateValue('developer_mode_enabled', '0');
    return developerSettingsForCurrentAccount();
  }
  developerClient();
  const credentials = developerCredentials();
  const binding = developerBinding();
  const currentAccountId = await currentDeveloperAccountId();
  if (!binding.accountId || binding.verifiedAt <= 0) throw new Error('请先验证 client_id 确实属于当前账号');
  if (binding.verifiedClientId !== credentials.clientId) throw new Error('开发者 client_id 已变化，请重新验证当前账号');
  if (binding.accountId !== currentAccountId) throw new Error('这套开发者凭据绑定的不是当前登录账号，请重新配置并验证');
  saveAppStateValue('developer_mode_enabled', '1');
  resumeDeveloperTransfers();
  return developerSettingsState(currentAccountId);
}

async function apiFileReadWithDeveloperFallback(endpoint, body, timeoutMs = requestTimeoutMs) {
  try {
    return await apiPost(endpoint, body, [], true, timeoutMs);
  } catch (primaryError) {
    if (!developerBinding().requestedEnabled) throw primaryError;
    try {
      const client = await ensureDeveloperModeForCurrentAccount();
      return await developerPostWithRetry(client, endpoint, body, 0);
    } catch (fallbackError) {
      const error = new Error(`主接口读取失败：${primaryError?.message || primaryError}；开发者接口兜底失败：${fallbackError?.message || fallbackError}`);
      error.statusCode = Number(fallbackError?.statusCode || primaryError?.statusCode || 502);
      throw error;
    }
  }
}

async function developerPostWithRetry(client, endpoint, body, retries = 2) {
  for (let attempt = 0; ; attempt += 1) {
    try { return await client.post(endpoint, body); }
    catch (error) {
      if (!(error instanceof DeveloperApiError) || !error.retryable || attempt >= retries) throw error;
      const delay = error.apiCode === 18010 ? 60_000 : Math.min(2_000 * (attempt + 1), 5_000);
      await new Promise((resolve) => setTimeout(resolve, delay));
    }
  }
}

function developerTaskId(payload) {
  return String(payload?.data?.task_id ?? payload?.data?.taskId ?? payload?.task_id ?? payload?.taskId ?? '').trim();
}

function developerCounts(data = {}) {
  const count = (...keys) => {
    const value = keys.map((key) => data[key]).find((entry) => entry != null && entry !== '');
    if (value == null) return undefined;
    const parsed = Number(value);
    return Number.isFinite(parsed) ? Math.max(0, Math.round(parsed)) : undefined;
  };
  return {
    total_count: count('total_count', 'totalCount'),
    passed_count: count('passed_count', 'passedCount'),
    rejected_count: count('rejected_count', 'rejectedCount'),
    pending_count: count('pending_count', 'pendingCount'),
    success_count: count('success_count', 'successCount', 'use_count', 'useCount'),
    skipped_count: count('skipped_count', 'skippedCount'),
  };
}

async function withDeveloperNameMutationLock(callback) {
  const previous = developerNameMutationChain;
  let release;
  developerNameMutationChain = new Promise((resolve) => { release = resolve; });
  await previous;
  try { return await callback(); }
  finally { release(); }
}

async function mapConcurrent(values, concurrency, callback) {
  const list = Array.from(values || []);
  const results = new Array(list.length);
  const errors = [];
  let cursor = 0;
  const worker = async () => {
    while (cursor < list.length && !errors.length) {
      const index = cursor;
      cursor += 1;
      try { results[index] = await callback(list[index], index); }
      catch (error) { errors.push(error); }
    }
  };
  await Promise.all(Array.from({ length: Math.min(Math.max(1, concurrency), list.length) }, worker));
  if (errors.length) throw errors[0];
  return results;
}

function cloudDetailRecord(data) {
  const value = data && typeof data === 'object' ? data : {};
  return value.fileInfo || value.file_info || value.resourceInfo || value.resource_info || value.file || value;
}

function cloudEntryFromRecord(record, fallbackId = '', fallbackName = '') {
  const value = cloudDetailRecord(record);
  const fileId = String(value.fileId ?? value.file_id ?? value.id ?? fallbackId ?? '').trim();
  const name = String(value.fileName ?? value.file_name ?? value.name ?? fallbackName ?? '').trim();
  const resType = Number(value.resType ?? value.res_type ?? value.type ?? 0);
  const folder = resType === 2 || value.isFolder === true || value.is_folder === true || value.isDir === true;
  const rawSize = value.fileSize ?? value.file_size ?? value.totalSize ?? value.total_size ?? value.size ?? 0;
  const size = Number(rawSize || 0);
  const gcid = String(value.gcid ?? value.GCID ?? value.gCid ?? '').trim();
  const ancestorIds = String(value.fullParentIds ?? value.full_parent_ids ?? '')
    .split('/')
    .map((item) => item.trim())
    .filter(Boolean);
  const modifiedAt = Number(value.utime ?? value.updatedAt ?? value.updateTime ?? value.modifiedAt ?? value.modifyTime ?? 0) || 0;
  const sizeInfo = record?.sizeInfo ?? record?.size_info ?? {};
  const optionalCount = (...values) => {
    const found = values.find((entry) => entry !== undefined && entry !== null && entry !== '');
    if (found === undefined) return null;
    const parsed = Number(found);
    return Number.isSafeInteger(parsed) && parsed >= 0 ? parsed : null;
  };
  if (!fileId || !name) throw new Error('光鸭返回的文件详情缺少文件 ID 或名称');
  if (!folder && (!Number.isSafeInteger(size) || size < 0)) throw new Error(`文件大小无效：${name}`);
  return {
    fileId,
    name,
    folder,
    size: folder ? 0 : size,
    gcid,
    modifiedAt,
    subtreeSize: optionalCount(sizeInfo.size, sizeInfo.totalSize, sizeInfo.total_size),
    subtreeFolders: optionalCount(sizeInfo.subDirCount, sizeInfo.sub_dir_count, sizeInfo.folderCount),
    subtreeFiles: optionalCount(sizeInfo.subFileCount, sizeInfo.sub_file_count, sizeInfo.fileCount),
    ancestorIds,
  };
}

function safeCloudPathSegment(value) {
  const segment = String(value || '').replace(/[\\/]/g, '_').replace(/[\u0000-\u001f\u007f]/g, '').trim();
  return segment || '未命名文件';
}

async function cloudEntryDetail(fileId, fallbackName = '') {
  const response = await apiPost('/userres/v1/file/get_file_detail', { fileId });
  return cloudEntryFromRecord(response.data || {}, fileId, fallbackName);
}

async function loadGcidExportRoots(fileIds, fallbackNames = [], diagnostics = null) {
  const roots = await mapConcurrent(fileIds, 8, async (fileId, index) => {
    const fields = { root_index: index, file_id_suffix: String(fileId || '').slice(-8) };
    diagnostics?.write('info', 'scan_root_detail_started', fields);
    try {
      const entry = await retryGcidExportScan(async (attempt) => {
        const startedAt = Date.now();
        try {
          return await cloudEntryDetail(fileId, fallbackNames[index]);
        }
        catch (error) {
          diagnostics?.write(error?.retryable === true ? 'warn' : 'error', 'scan_root_detail_attempt_failed', {
            ...fields,
            attempt: attempt + 1,
            max_attempts: GCID_EXPORT_SCAN_ATTEMPTS,
            retrying: error?.retryable === true && attempt + 1 < GCID_EXPORT_SCAN_ATTEMPTS,
            elapsed_ms_request: Date.now() - startedAt,
            path: fallbackNames[index] || '',
            error: error?.message || error,
          });
          throw error;
        }
      });
      diagnostics?.write('info', 'scan_root_detail_succeeded', {
        ...fields,
        path: entry.name,
        is_folder: entry.folder,
      });
      return entry;
    }
    catch (error) {
      diagnostics?.write('error', 'scan_root_detail_failed', {
        ...fields,
        path: fallbackNames[index] || '',
        error: error?.message || error,
      });
      throw error;
    }
  });
  return roots;
}

async function collectCloudSelectionEntries(
  fileIds,
  fallbackNames = [],
  includeFolders = false,
  diagnostics = null,
  preloadedRoots = null,
) {
  const roots = preloadedRoots || await loadGcidExportRoots(fileIds, fallbackNames, diagnostics);
  const entries = [];
  const visited = new Set();
  const queue = roots.map((entry) => ({ entry, relativePath: safeCloudPathSegment(entry.name) }));
  let queueIndex = 0;
  let scannedFolders = 0;
  while (queueIndex < queue.length) {
    const folders = [];
    while (folders.length < GCID_EXPORT_SCAN_CONCURRENCY && queueIndex < queue.length) {
      const current = queue[queueIndex];
      queueIndex += 1;
      if (visited.has(current.entry.fileId)) continue;
      visited.add(current.entry.fileId);
      if (current.entry.folder) folders.push(current);
      else entries.push({ ...current.entry, path: current.relativePath });
    }
    const loaded = await mapConcurrent(folders, GCID_EXPORT_SCAN_CONCURRENCY, async (current) => ({
      current,
      children: await organizerListCloudChildren(current.entry.fileId, (pageEvent) => {
        diagnostics?.write(pageEvent.level, pageEvent.event, {
          path: current.relativePath,
          file_id_suffix: String(current.entry.fileId || '').slice(-8),
          ...pageEvent.fields,
        });
      }),
    }));
    for (const { current, children } of loaded) {
      scannedFolders += 1;
      if (includeFolders) entries.push({ ...current.entry, path: current.relativePath });
      for (const child of children) {
        const entry = cloudEntryFromRecord(child);
        queue.push({
          entry,
          relativePath: `${current.relativePath}/${safeCloudPathSegment(entry.name)}`,
        });
      }
    }
    if (visited.size + queue.length - queueIndex > 100_000) throw new Error('一次最多处理 100000 个云端文件或文件夹');
  }
  return { entries, roots, scannedFolders };
}

function shouldUseGcidExportInventory(roots) {
  return roots.some((entry) => entry.folder
    && (entry.subtreeFolders == null
      || Number(entry.subtreeFolders) + 1 > gcidExportInventoryThreshold));
}

async function loadGcidExportInventoryPage(resType, page, diagnostics) {
  const fields = { inventory_type: resType === 1 ? 'file' : 'folder', page };
  const startedAt = Date.now();
  const response = await retryGcidExportScan(async (attempt) => {
    const attemptStartedAt = Date.now();
    try {
      return await apiPost('/userres/v1/file/get_file_list', {
        parentId: '*',
        page,
        pageSize: gcidExportInventoryPageSize,
        orderBy: 0,
        sortType: 0,
        resType,
      });
    }
    catch (error) {
      diagnostics.write(error?.retryable === true ? 'warn' : 'error', 'scan_inventory_page_attempt_failed', {
        ...fields,
        attempt: attempt + 1,
        max_attempts: GCID_EXPORT_SCAN_ATTEMPTS,
        retrying: error?.retryable === true && attempt + 1 < GCID_EXPORT_SCAN_ATTEMPTS,
        elapsed_ms_request: Date.now() - attemptStartedAt,
        error: error?.message || error,
      });
      throw error;
    }
  });
  const list = Array.isArray(response.data?.list) ? response.data.list : [];
  const total = Number.isFinite(Number(response.data?.total))
    ? Math.max(0, Number(response.data.total))
    : list.length;
  diagnostics.write('info', 'scan_inventory_page_succeeded', {
    ...fields,
    page_entries: list.length,
    reported_total: total,
    elapsed_ms_page: Date.now() - startedAt,
  });
  return { resType, page, list, total };
}

function gcidExportInventoryPath(entry, roots, folderNames) {
  const exactRoot = roots.find((root) => !root.folder && root.fileId === entry.fileId);
  if (exactRoot) return safeCloudPathSegment(exactRoot.name);
  for (const root of roots) {
    if (!root.folder) continue;
    const rootIndex = entry.ancestorIds.indexOf(root.fileId);
    if (rootIndex < 0) continue;
    const parts = [safeCloudPathSegment(root.name)];
    for (const ancestorId of entry.ancestorIds.slice(rootIndex + 1)) {
      const name = folderNames.get(ancestorId);
      if (!name) return '';
      parts.push(safeCloudPathSegment(name));
    }
    parts.push(safeCloudPathSegment(entry.name));
    return parts.join('/');
  }
  return '';
}

async function collectGcidExportEntries(fileIds, fallbackNames, diagnostics, roots) {
  if (!shouldUseGcidExportInventory(roots)) {
    diagnostics.write('info', 'scan_strategy_selected', { strategy: 'directory' });
    return collectCloudSelectionEntries(fileIds, fallbackNames, false, diagnostics, roots);
  }
  diagnostics.write('info', 'scan_strategy_selected', {
    strategy: 'global_inventory',
    page_size: gcidExportInventoryPageSize,
    concurrency: GCID_EXPORT_SCAN_CONCURRENCY,
  });
  const [firstFiles, firstFolders] = await Promise.all([
    loadGcidExportInventoryPage(1, 0, diagnostics),
    loadGcidExportInventoryPage(2, 0, diagnostics),
  ]);
  const filePages = Math.max(1, Math.ceil(firstFiles.total / gcidExportInventoryPageSize));
  const folderPages = Math.max(1, Math.ceil(firstFolders.total / gcidExportInventoryPageSize));
  const totalPages = filePages + folderPages;
  let completedPages = 2;
  let scannedEntries = firstFiles.list.length + firstFolders.list.length;
  const publishScan = () => publish({
    type: 'gcid-export-progress',
    phase: 'scan',
    stage: '正在加载云端文件索引',
    current_path: `已读取 ${scannedEntries} 条云端索引`,
    completed_files: 0,
    total_files: 0,
    scanned_pages: completedPages,
    total_pages: totalPages,
    scanned_entries: scannedEntries,
    percent: Math.max(0, Math.min(100, Math.floor(completedPages * 100 / totalPages))),
    diagnostic_run_id: diagnostics.runId,
  });
  publishScan();
  const jobs = [];
  for (let page = 1; page < filePages; page += 1) jobs.push({ resType: 1, page });
  for (let page = 1; page < folderPages; page += 1) jobs.push({ resType: 2, page });
  const pages = await mapConcurrent(jobs, GCID_EXPORT_SCAN_CONCURRENCY, async (job) => {
    const result = await loadGcidExportInventoryPage(job.resType, job.page, diagnostics);
    scannedEntries += result.list.length;
    completedPages += 1;
    publishScan();
    return result;
  });
  pages.sort((left, right) => left.resType - right.resType || left.page - right.page);
  const fileRecords = [...firstFiles.list];
  const folderRecords = [...firstFolders.list];
  for (const page of pages) {
    if (page.resType === 1) fileRecords.push(...page.list);
    else folderRecords.push(...page.list);
  }
  if (fileRecords.length < firstFiles.total || folderRecords.length < firstFolders.total) {
    throw new Error('光鸭全库文件索引返回不完整，请稍后重试');
  }
  if (fileRecords.length + folderRecords.length > 100_000) {
    throw new Error('一次最多处理 100000 个云端文件或文件夹');
  }
  const folders = folderRecords.map((record) => cloudEntryFromRecord(record));
  const folderNames = new Map(folders.map((entry) => [entry.fileId, entry.name]));
  for (const root of roots) if (root.folder) folderNames.set(root.fileId, root.name);
  const selectedFolderIds = new Set(folders
    .filter((entry) => roots.some((root) => root.folder
      && (entry.fileId === root.fileId || entry.ancestorIds.includes(root.fileId))))
    .map((entry) => entry.fileId));
  const entries = [];
  const seenFiles = new Set();
  for (const record of fileRecords) {
    const entry = cloudEntryFromRecord(record);
    const selectedPath = gcidExportInventoryPath(entry, roots, folderNames);
    if (!selectedPath || seenFiles.has(entry.fileId)) continue;
    seenFiles.add(entry.fileId);
    entries.push({ ...entry, path: selectedPath });
  }
  for (const root of roots) {
    if (root.folder || seenFiles.has(root.fileId)) continue;
    seenFiles.add(root.fileId);
    entries.push({ ...root, path: safeCloudPathSegment(root.name) });
  }
  if (roots.length === 1 && roots[0].folder && roots[0].subtreeFiles != null
    && entries.length !== Number(roots[0].subtreeFiles)) {
    throw new Error(`光鸭全库索引与目录统计不一致（索引 ${entries.length} / 目录 ${roots[0].subtreeFiles}），请稍后重试`);
  }
  diagnostics.write('info', 'scan_inventory_filtered', {
    account_files: firstFiles.total,
    account_folders: firstFolders.total,
    selected_files: entries.length,
    selected_folders: selectedFolderIds.size,
  });
  return { entries, roots, scannedFolders: selectedFolderIds.size };
}

async function renameDeveloperNameWithRetry(fileId, newName) {
  const attempts = 5;
  let lastError;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    try {
      await renameRemote(fileId, newName);
      return;
    } catch (error) {
      lastError = error;
      try {
        if ((await cloudEntryDetail(fileId, newName)).name === newName) return;
      } catch {}
    }
    if (attempt + 1 < attempts) {
      const delay = [400, 800, 1_600, 3_000][Math.min(attempt, 3)];
      await new Promise((resolve) => setTimeout(resolve, delay));
    }
  }
  throw new Error(`${lastError?.message || lastError || '文件名改写失败'}（文件名改写已重试 ${attempts} 次）`);
}

async function releaseDeveloperNameObfuscation(jobId) {
  return withDeveloperNameMutationLock(async () => {
    const rows = database.prepare("SELECT file_id, original_name, temporary_name FROM developer_transfer_name_restores WHERE job_id = ? AND status IN ('active', 'released', 'restore_failed')").all(jobId);
    if (!rows.length) return { restored: 0, deferred: 0, failed: 0 };
    const now = Math.floor(Date.now() / 1000);
    database.prepare("UPDATE developer_transfer_name_restores SET status = 'released', updated_at = ? WHERE job_id = ? AND status = 'active'").run(now, jobId);
    const restorable = [];
    let deferred = 0;
    for (const row of rows) {
      const active = Number(database.prepare("SELECT COUNT(*) AS count FROM developer_transfer_name_restores WHERE file_id = ? AND status = 'active'").get(row.file_id)?.count || 0);
      if (active) deferred += 1;
      else restorable.push(row);
    }
    const previous = loadDeveloperTransferJob(jobId);
    const total = restorable.length + deferred;
    let processed = deferred;
    updateDeveloperTransferJob(jobId, {
      phase: 'restoring',
      work_total_count: total,
      processed_count: processed,
      current_path: String(restorable[0]?.original_name || ''),
      message: `正在恢复源文件名 ${processed}/${total}`,
    });
    const outcomes = await mapConcurrent(restorable, 2, async (row) => {
      let restored = false;
      try {
        let currentName = '';
        try { currentName = (await cloudEntryDetail(String(row.file_id))).name; } catch {}
        if (currentName !== String(row.original_name)) {
          await renameDeveloperNameWithRetry(String(row.file_id), String(row.original_name));
        }
        database.prepare("UPDATE developer_transfer_name_restores SET status = 'completed', last_error = NULL, updated_at = ? WHERE file_id = ? AND status <> 'active'").run(Math.floor(Date.now() / 1000), row.file_id);
        restored = true;
      } catch (error) {
        database.prepare("UPDATE developer_transfer_name_restores SET status = 'restore_failed', last_error = ?, updated_at = ? WHERE file_id = ? AND status <> 'active'")
          .run(String(error?.message || error || '恢复原文件名失败').slice(0, 500), Math.floor(Date.now() / 1000), row.file_id);
      }
      processed += 1;
      updateDeveloperTransferJob(jobId, {
        processed_count: processed,
        current_path: String(row.original_name || ''),
        message: `正在恢复源文件名 ${processed}/${total}`,
      });
      return restored;
    });
    database.prepare("DELETE FROM developer_transfer_name_restores WHERE status = 'completed' AND updated_at < ?").run(now - 30 * 86_400);
    resetRemoteDirectoryCache();
    webDavDirectoryCache.clear();
    publishCloudDirectoryInvalidated([], { all: true, source: 'developer-name-restore' });
    updateDeveloperTransferJob(jobId, {
      phase: previous?.phase || 'completed',
      processed_count: total,
      current_path: '',
      message: previous?.message ?? null,
    });
    return {
      restored: outcomes.filter(Boolean).length,
      deferred,
      failed: outcomes.filter((value) => !value).length,
    };
  });
}

async function finishDeveloperUpload(client, jobId, taskId) {
  const initial = loadDeveloperTransferJob(jobId);
  updateDeveloperTransferJob(jobId, {
    status: 'running', phase: 'upload', upload_task_id: taskId,
    work_total_count: initial?.total_count || 0,
    processed_count: Math.min(initial?.total_count || 0, (initial?.success_count || 0) + (initial?.skipped_count || 0)),
    current_path: '', message: '小号正在接收文件',
  });
  for (let index = 0; index < 400; index += 1) {
    const payload = await developerPostWithRetry(client, '/developer/v1/upload_status', { task_id: taskId });
    const data = payload?.data || {};
    const stateValue = String(data.status || payload.status || '').toLowerCase();
    const counts = developerCounts(data);
    const current = loadDeveloperTransferJob(jobId);
    const total = counts.total_count ?? current?.total_count ?? 0;
    const successCount = counts.success_count ?? current?.success_count ?? 0;
    const skippedCount = counts.skipped_count ?? current?.skipped_count ?? 0;
    if (stateValue === 'success' || (stateValue === 'failed' && successCount > 0)) {
      const rejectedCount = current?.rejected_count || 0;
      const message = rejectedCount > 0
        ? `已秒传 ${successCount || current?.passed_count || 0} 个，${rejectedCount} 个未通过预审`
        : '文件已秒传到小号授权目录';
      return updateDeveloperTransferJob(jobId, {
        ...counts, status: 'success', phase: 'completed',
        work_total_count: total, processed_count: total,
        current_path: '', error_code: null, message,
      });
    }
    if (stateValue === 'failed') {
      updateDeveloperTransferJob(jobId, {
        ...counts, status: 'running', phase: 'upload', work_total_count: total,
        processed_count: Math.min(total, successCount + skippedCount), current_path: '',
      });
      throw new Error(String(data.message || data.msg || '小号秒传任务失败'));
    }
    updateDeveloperTransferJob(jobId, {
      ...counts, status: 'running', phase: 'upload', work_total_count: total,
      processed_count: Math.min(total, successCount + skippedCount),
      current_path: '', message: '小号正在接收文件',
    });
    await new Promise((resolve) => setTimeout(resolve, 1_500));
  }
  throw new Error('小号秒传任务长时间未完成，请稍后在任务记录中重试');
}

async function submitDeveloperUpload(client, job, targetToken) {
  updateDeveloperTransferJob(job.id, {
    status: 'copying', phase: 'upload', work_total_count: job.total_count,
    processed_count: 0, current_path: '', message: '正在提交小号秒传',
  });
  const payload = await developerPostWithRetry(client, '/developer/v1/upload_by_fileid', {
    token_id: targetToken,
    file_ids: job.file_ids,
  });
  const taskId = developerTaskId(payload);
  if (!taskId) throw new Error('开发者接口没有返回秒传任务 ID');
  return finishDeveloperUpload(client, job.id, taskId);
}

async function finishDeveloperPreAudit(client, job, targetToken, taskState) {
  const plan = decodeDeveloperPreAuditPlan(taskState, job.total_count);
  updateDeveloperTransferJob(job.id, {
    status: 'auditing', phase: 'pre_upload', pre_task_id: encodeDeveloperPreAuditPlan(plan.batches),
    work_total_count: job.total_count,
    processed_count: Math.min(job.total_count, job.passed_count + job.rejected_count),
    current_path: '', message: '文件正在预审，通过后会自动秒传',
  });
  for (let pollIndex = 0; pollIndex < 7_200; pollIndex += 1) {
    for (const batch of plan.batches) {
      if (batch.done) continue;
      try {
        const payload = await developerPostWithRetry(client, '/developer/v1/pre_upload_status', { task_id: batch.task_id });
        const data = payload?.data || {};
        const auditStatus = Number(data.status ?? payload.status ?? 0);
        const counts = developerCounts(data);
        batch.file_count = Math.max(batch.file_count, counts.total_count || 0, (counts.passed_count || 0) + (counts.rejected_count || 0));
        batch.passed_count = counts.passed_count ?? batch.passed_count;
        batch.rejected_count = counts.rejected_count ?? batch.rejected_count;
        if (auditStatus === 4) {
          batch.rejected_count = Math.max(batch.rejected_count, batch.file_count - batch.passed_count);
          batch.done = true;
          batch.failed = true;
        } else if (auditStatus === 3) {
          batch.done = true;
        }
      } catch {
        batch.rejected_count = Math.max(batch.rejected_count, batch.file_count - batch.passed_count);
        batch.done = true;
        batch.failed = true;
      }
    }
    const summary = finalizeDeveloperPreAuditSummary(plan);
    const suffix = summary.failed_batches ? `；${summary.failed_batches} 个预审批次失败，已跳过` : '';
    updateDeveloperTransferJob(job.id, {
      status: 'auditing', phase: 'pre_upload', pre_task_id: encodeDeveloperPreAuditPlan(plan.batches),
      total_count: summary.total_count, passed_count: summary.passed_count,
      rejected_count: summary.rejected_count, pending_count: summary.pending_count,
      work_total_count: summary.total_count,
      processed_count: Math.min(summary.total_count, summary.passed_count + summary.rejected_count),
      current_path: '', message: `文件正在预审，通过后会自动秒传${suffix}`,
    });
    if (summary.done) {
      const completed = loadDeveloperTransferJob(job.id);
      try {
        return await submitDeveloperUpload(client, completed, targetToken);
      } catch (error) {
        if (error instanceof DeveloperApiError && error.apiCode === 18014) {
          return updateDeveloperTransferJob(job.id, {
            status: 'success', phase: 'completed',
            skipped_count: Math.max(completed.skipped_count, completed.passed_count),
            work_total_count: completed.total_count, processed_count: completed.total_count,
            current_path: '', error_code: null,
            message: `通过的 ${completed.passed_count} 个文件此前已传给该小号；${completed.rejected_count} 个未通过预审`,
          });
        }
        if (!(error instanceof DeveloperApiError) || error.apiCode !== 18011) throw error;
        const detail = completed.passed_count > 0
          ? `预审显示通过 ${completed.passed_count} 个，但平台正式秒传时未返回可上传文件`
          : `预审完成：${completed.rejected_count} 个文件均未通过，未开始秒传`;
        throw new DeveloperApiError(detail, { code: 18011 });
      }
    }
    await new Promise((resolve) => setTimeout(resolve, 3_000));
  }
  throw new Error('文件预审超过 6 小时仍未完成');
}

async function startDeveloperPreAudit(client, job, targetToken) {
  const collected = await collectCloudSelectionEntries(job.file_ids, job.file_names, false);
  const files = collected.entries.filter((entry) => !entry.folder);
  if (!files.length) throw new Error('所选内容中没有可预审的文件');
  const chunks = chunkDeveloperPreAuditFileIds(files.map((entry) => entry.fileId));
  const batches = [];
  let submitted = 0;
  updateDeveloperTransferJob(job.id, {
    status: 'auditing', phase: 'pre_upload', total_count: files.length,
    passed_count: 0, rejected_count: 0, pending_count: files.length,
    work_total_count: files.length, processed_count: 0,
    current_path: files[0]?.path || '', message: '正在按原文件分批提交预审',
  });
  for (const chunk of chunks) {
    try {
      const payload = await developerPostWithRetry(client, '/developer/v1/pre_upload', {
        token_id: targetToken,
        file_ids: chunk,
      });
      const taskId = developerTaskId(payload);
      if (!taskId) throw new Error('开发者接口没有返回预审任务 ID');
      batches.push(createDeveloperPreAuditBatch(taskId, chunk.length));
    } catch {
      batches.push(createDeveloperPreAuditBatch('', chunk.length, {
        rejected_count: chunk.length, done: true, failed: true,
      }));
    }
    submitted += chunk.length;
    const currentPath = files[Math.min(submitted, files.length - 1)]?.path || '';
    updateDeveloperTransferJob(job.id, {
      status: 'auditing', phase: 'pre_upload', pre_task_id: encodeDeveloperPreAuditPlan(batches),
      work_total_count: files.length, processed_count: 0, current_path: currentPath,
      message: `正在按原文件分批提交预审 ${submitted}/${files.length}`,
    });
  }
  const prepared = loadDeveloperTransferJob(job.id);
  return finishDeveloperPreAudit(client, prepared, targetToken, encodeDeveloperPreAuditPlan(batches));
}

async function runDeveloperTransferJob(jobId) {
  if (developerTransferRunning.has(jobId)) return;
  developerTransferRunning.add(jobId);
  try {
    let job = loadDeveloperTransferJob(jobId);
    if (!job || ['success', 'failed'].includes(job.status)) return;
    const target = database.prepare('SELECT token_id FROM developer_targets WHERE id = ?').get(job.target_id);
    if (!target?.token_id) throw new Error('小号接收 TOKEN 配置已不存在');
    const client = developerClient();
    if (job.upload_task_id) {
      await finishDeveloperUpload(client, job.id, job.upload_task_id);
      return;
    }
    if (job.pre_task_id) {
      await finishDeveloperPreAudit(client, job, target.token_id, job.pre_task_id);
      return;
    }
    updateDeveloperTransferJob(job.id, {
      status: 'direct', phase: 'direct', work_total_count: job.total_count,
      processed_count: 0, current_path: '', message: '正在尝试直接秒传',
    });
    try {
      await submitDeveloperUpload(client, job, target.token_id);
    } catch (error) {
      if (error instanceof DeveloperApiError && error.apiCode === 18014) {
        updateDeveloperTransferJob(job.id, {
          status: 'success', phase: 'completed', skipped_count: job.total_count,
          work_total_count: job.total_count, processed_count: job.total_count,
          current_path: '', error_code: null, message: '这些文件此前已传给该小号，无需重复传输',
        });
        return;
      }
      if (!(error instanceof DeveloperApiError) || error.apiCode !== 18011) throw error;
      await startDeveloperPreAudit(client, job, target.token_id);
    }
  } catch (error) {
    const code = error instanceof DeveloperApiError ? error.apiCode : null;
    updateDeveloperTransferJob(jobId, {
      status: 'failed',
      phase: 'failed',
      current_path: '',
      error_code: code,
      message: String(error?.message || error || '小号互传失败'),
    });
  } finally {
    try {
      const restored = await releaseDeveloperNameObfuscation(jobId);
      if (restored.failed) {
        const current = loadDeveloperTransferJob(jobId);
        updateDeveloperTransferJob(jobId, {
          message: `${current?.message || '小号互传已结束'}；${restored.failed} 个源文件名恢复失败，请稍后重试`,
        });
      } else if (restored.restored) {
        const current = loadDeveloperTransferJob(jobId);
        updateDeveloperTransferJob(jobId, {
          message: `${current?.message || '小号互传已结束'}，源文件名已恢复`,
        });
      }
    } catch (restoreError) {
      const current = loadDeveloperTransferJob(jobId);
      updateDeveloperTransferJob(jobId, {
        message: `${current?.message || '小号互传已结束'}；恢复源文件名时出错：${restoreError?.message || restoreError}`,
      });
    }
    developerTransferRunning.delete(jobId);
  }
}

function resumeDeveloperTransfers() {
  const binding = developerBinding();
  const credentials = developerCredentials();
  if (!binding.requestedEnabled || !binding.accountId || binding.verifiedAt <= 0 || binding.verifiedClientId !== credentials.clientId) return;
  const restoreJobs = database.prepare(`SELECT DISTINCT restores.job_id
    FROM developer_transfer_name_restores AS restores
    JOIN developer_transfer_jobs AS jobs ON jobs.id = restores.job_id
    WHERE restores.status IN ('active', 'released', 'restore_failed')
      AND jobs.status IN ('success', 'failed')`).all();
  for (const row of restoreJobs) {
    setImmediate(() => void releaseDeveloperNameObfuscation(String(row.job_id)).catch(() => {}));
  }
  const rows = database.prepare("SELECT id FROM developer_transfer_jobs WHERE status IN ('queued', 'direct', 'auditing', 'copying', 'running') ORDER BY created_at").all();
  for (const row of rows) setImmediate(() => void runDeveloperTransferJob(String(row.id)));
}

async function startDeveloperTransfer(body) {
  const targetId = validateIdentifier(body.target_id ?? body.targetId, '小号配置 ID');
  const target = database.prepare('SELECT id, name FROM developer_targets WHERE id = ?').get(targetId);
  if (!target) throw new Error('请选择有效的小号接收 TOKEN');
  const fileIds = validateFileIds(body.file_ids ?? body.fileIds);
  if (fileIds.length > 20) throw new Error('开发者接口一次最多互传 20 项');
  await ensureDeveloperModeForCurrentAccount(fileIds[0]);
  const fileNames = Array.isArray(body.file_names ?? body.fileNames)
    ? (body.file_names ?? body.fileNames).slice(0, fileIds.length).map((value) => String(value || '').slice(0, 255))
    : [];
  const pairs = fileIds.map((fileId, index) => ({ fileId, fileName: fileNames[index] || '' }))
    .sort((left, right) => left.fileId.localeCompare(right.fileId));
  const sortedFileIds = pairs.map((item) => item.fileId);
  const sortedFileNames = pairs.map((item) => item.fileName);
  const identity = JSON.stringify(sortedFileIds);
  const duplicate = database.prepare("SELECT * FROM developer_transfer_jobs WHERE target_id = ? AND file_ids_json = ? AND status IN ('queued', 'direct', 'auditing', 'copying', 'running') ORDER BY created_at DESC LIMIT 1").get(targetId, identity);
  if (duplicate) return { ...developerTransferJob(duplicate), reused: true };
  const id = crypto.randomUUID();
  const now = Math.floor(Date.now() / 1000);
  database.prepare(`INSERT INTO developer_transfer_jobs
    (id, target_id, target_name, file_ids_json, file_names_json, status, phase, total_count, work_total_count, message, created_at, updated_at)
    VALUES (?, ?, ?, ?, ?, 'direct', 'direct', ?, ?, ?, ?, ?)`)
    .run(id, targetId, target.name, identity, JSON.stringify(sortedFileNames), sortedFileIds.length, sortedFileIds.length, '正在并发启动小号秒传', now, now);
  const job = loadDeveloperTransferJob(id);
  setImmediate(() => void runDeveloperTransferJob(id));
  return job;
}

async function testDeveloperCredentials(probeFileId = '') {
  const verified = await verifyDeveloperAccountOwnership(probeFileId);
  saveAppStateValue('developer_account_id', verified.accountId);
  saveAppStateValue('developer_verified_client_id', developerCredentials().clientId);
  saveAppStateValue('developer_account_verified_at', String(Math.floor(Date.now() / 1000)));
  saveAppStateValue('developer_mode_enabled', '0');
  return {
    ok: true,
    account_id: verified.accountId,
    settings: developerSettingsState(verified.accountId),
  };
}

function state() { return { logged_in: Boolean(token), paused, pending: queue.size + waitingFiles.size + pendingUploads.size, active_uploads: active, upload_concurrency: uploadConcurrency, download_concurrency: downloadConcurrency, multipart: multipartMode, multipart_part_size: multipartMode, mappings, saved_shares: savedShares, hdhive: { enabled: hdhiveEnabled, configured: Boolean(hdhiveBaseUrl && hdhiveSecret), base_url: hdhiveBaseUrl, instance_id: hdhiveInstanceId }, auto_share_receipts: autoShareReceipts() }; }
function publish(payload) { const line = `data: ${JSON.stringify(payload)}\n\n`; for (const response of clients) response.write(line); }
function publishState() { publish({ type: 'state', state: state() }); }
function status(level, message) { publish({ type: 'status', level, message }); }
function publishCloudDirectoryInvalidated(parentIds = [], {
  all = false,
  source = 'cloud-write',
} = {}) {
  const normalized = [...new Set((Array.isArray(parentIds) ? parentIds : [parentIds])
    .filter((value) => value !== undefined && value !== null)
    .map((value) => String(value)))];
  publish({
    type: 'cloud-directory-invalidated',
    parent_ids: normalized,
    all: Boolean(all),
    source: String(source),
  });
}
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
function invalidateAuthSession() {
  token = null;
  refreshToken = null;
  authSessionScope.clearCurrent();
  resetRemoteDirectoryCache();
  synchronizeWebDavCacheScope();
  replaceAuthSession(null, null);
  publishState();
}
function uploadHistoryPath(item) { return item.history_path || item.file_path; }
function uploadEventPath(item) { return item.event_path || item.file_path; }
class UploadCancelledError extends Error {
  constructor() {
    super('上传已取消');
    this.name = 'UploadCancelledError';
  }
}
class UploadPausedError extends Error {
  constructor() {
    super('上传已暂停');
    this.name = 'UploadPausedError';
  }
}
function isUploadCancelled(key) { return cancelledUploads.has(key); }
function assertUploadNotCancelled(key) { if (isUploadCancelled(key)) throw new UploadCancelledError(); }
function isUploadCancellationError(error) { return error instanceof UploadCancelledError || error?.name === 'cancel' || error?.message === '上传已取消'; }
function isUploadPauseRequested(key) { return pausedUploads.has(key) || queuePauseRequests.has(key); }
function assertUploadRunnable(key) {
  assertUploadNotCancelled(key);
  if (isUploadPauseRequested(key)) throw new UploadPausedError();
}
function isUploadPauseError(error) { return error instanceof UploadPausedError || error?.message === '上传已暂停'; }
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
    fullEndPoint: params?.fullEndPoint,
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
  return {
    response,
    // Some resume responses omit static endpoint fields. Retain only the
    // non-sensitive location metadata from the original checkpoint.
    params: { ...params, ...(response.data || {}) },
  };
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
  const previous = restorePreviousUploadRecord(row);
  if (previous) {
    database.prepare("UPDATE uploaded_files SET size = ?, modified_ms = ?, task_id = NULL, remote_file_id = ?, status = 'cloud_confirmed', item_json = NULL, uploaded_at = ? WHERE mapping_id = ? AND file_path = ? AND status = 'oss_complete'")
      .run(previous.size, previous.modifiedMs, previous.remoteFileId, Math.floor(Date.now() / 1000), row.mapping_id, row.file_path);
    history.set(key, `${previous.size}:${previous.modifiedMs}`);
  } else {
    database.prepare("DELETE FROM uploaded_files WHERE mapping_id = ? AND file_path = ? AND status = 'oss_complete'").run(row.mapping_id, row.file_path);
  }
  pendingUploads.delete(key);
}
function pendingUploadItem(row) {
  let stored = {};
  try { stored = JSON.parse(row?.item_json || '{}'); } catch {}
  return {
    ...stored,
    mapping_id: stored.mapping_id || row.mapping_id,
    file_path: stored.file_path || row.file_path,
    size: Number(stored.size ?? row.size ?? 0),
    mtime: Number(stored.mtime ?? row.modified_ms ?? 0),
  };
}
async function cancelUploadTask(filePath, mappingId = '') {
  const targetPath = String(filePath || '').trim();
  const targetMapping = String(mappingId || '').trim();
  if (!targetPath) throw new Error('缺少要取消的上传路径');
  const matches = (item) => uploadEventPath(item) === targetPath
    && (!targetMapping || String(item.mapping_id || '') === targetMapping);
  const matched = new Map();
  for (const [key, item] of queue) if (matches(item)) matched.set(key, item);
  for (const [key, item] of waitingFiles) if (matches(item)) matched.set(key, item);
  for (const [key, item] of inflightItems) if (matches(item)) matched.set(key, item);
  for (const [key, item] of failedUploads) if (matches(item)) matched.set(key, item);
  for (const [key, row] of pendingUploads) {
    const item = pendingUploadItem(row);
    if (matches(item)) matched.set(key, item);
  }
  if ([...matched.keys()].some((key) => activeUploadReplacements.has(key))) {
    throw httpError(409, '新版本已经入库，正在安全替换旧文件，此阶段不能取消');
  }

  const cleanup = [];
  for (const [key, item] of matched) {
    cancelledUploads.set(key, uploadStamp(item));
    pausedUploads.delete(key);
    queuePauseRequests.delete(key);
    queue.delete(key);
    waitingFiles.delete(key);
    failedUploads.delete(key);
    flashPreflightCache.delete(key);
    try { activeUploadClients.get(key)?.cancel(); } catch {}
    clearUploadCheckpoint(item);
    clearPendingUpload(key);
    if (!inflightItems.has(key) && item.cleanup_path && isWithinRoot(manualUploadRoot, item.cleanup_path)) {
      cleanup.push(fsp.rm(item.cleanup_path, { recursive: true, force: true }));
    }
    publish({
      type: 'file',
      state: 'cancelled',
      file_path: uploadEventPath(item),
      mapping_id: item.mapping_id,
      uploaded_bytes: 0,
      total_bytes: Number(item.size || 0),
      stage: '已取消',
    });
  }
  await Promise.allSettled(cleanup);
  publishState();
  pump();
  return { cancelled: matched.size > 0, count: matched.size };
}
async function retryUploadTask(filePath, mappingId = '') {
  const targetPath = String(filePath || '').trim();
  const targetMapping = String(mappingId || '').trim();
  if (!targetPath) throw new Error('缺少要重试的上传路径');
  const matched = [...failedUploads.entries()].filter(([, item]) => uploadEventPath(item) === targetPath
    && (!targetMapping || String(item.mapping_id || '') === targetMapping));
  if (!matched.length) throw httpError(404, '失败的上传任务已失效，请重新选择文件上传');

  let retried = 0;
  for (const [key, item] of matched) {
    let stat;
    try { stat = await fsp.stat(item.file_path); }
    catch { failedUploads.delete(key); continue; }
    if (!stat.isFile()) { failedUploads.delete(key); continue; }
    const refreshed = { ...item, size: stat.size, mtime: item.history_path ? item.mtime : stat.mtimeMs };
    failedUploads.delete(key);
    cancelledUploads.delete(key);
    pausedUploads.delete(key);
    queuePauseRequests.delete(key);
    waitingFiles.delete(key);
    flashPreflightCache.delete(key);
    queue.delete(key);
    queue.set(key, refreshed);
    retried += 1;
    publish({
      type: 'file', state: token ? 'queued' : 'waiting-login',
      file_path: uploadEventPath(refreshed), mapping_id: refreshed.mapping_id,
      uploaded_bytes: 0, total_bytes: refreshed.size,
      stage: token ? '已重新加入上传队列' : '等待登录后重试',
    });
  }
  if (!retried) throw httpError(410, '本地源文件已不存在，请重新选择文件上传');
  publishState();
  pump();
  return { retried: true, count: retried };
}

async function pauseUploadTask(filePath, mappingId = '') {
  const targetPath = String(filePath || '').trim();
  const targetMapping = String(mappingId || '').trim();
  if (!targetPath) throw new Error('缺少要暂停的上传路径');
  const matches = (item) => uploadEventPath(item) === targetPath
    && (!targetMapping || String(item.mapping_id || '') === targetMapping);
  const matched = new Map();
  for (const [key, item] of queue) if (matches(item)) matched.set(key, item);
  for (const [key, item] of waitingFiles) if (matches(item)) matched.set(key, item);
  for (const [key, item] of inflightItems) if (matches(item)) matched.set(key, item);
  if (!matched.size) return { paused: false, count: 0 };

  for (const [key, item] of matched) {
    pausedUploads.add(key);
    const checkpoint = loadUploadCheckpoint(item);
    if (inflightItems.has(key)) {
      try { activeUploadClients.get(key)?.cancel(); } catch {}
      publish({
        type: 'file', state: 'pausing', file_path: uploadEventPath(item), mapping_id: item.mapping_id,
        uploaded_bytes: Number(checkpoint?.uploadedBytes || 0), total_bytes: Number(item.size || 0),
        stage: '正在暂停并保存上传断点',
      });
      continue;
    }
    publish({
      type: 'file', state: 'paused', file_path: uploadEventPath(item), mapping_id: item.mapping_id,
      uploaded_bytes: Number(checkpoint?.uploadedBytes || 0), total_bytes: Number(item.size || 0),
      stage: '已暂停，可从当前断点继续',
    });
  }
  publishState();
  return { paused: true, count: matched.size };
}

async function resumeUploadTask(filePath, mappingId = '') {
  const targetPath = String(filePath || '').trim();
  const targetMapping = String(mappingId || '').trim();
  if (!targetPath) throw new Error('缺少要继续的上传路径');
  if (paused) throw httpError(409, '上传队列已暂停，请先恢复队列');
  const matches = (item) => uploadEventPath(item) === targetPath
    && (!targetMapping || String(item.mapping_id || '') === targetMapping);
  const matched = new Map();
  for (const [key, item] of queue) if (pausedUploads.has(key) && matches(item)) matched.set(key, item);
  for (const [key, item] of waitingFiles) if (pausedUploads.has(key) && matches(item)) matched.set(key, item);
  for (const [key, item] of inflightItems) if (pausedUploads.has(key) && matches(item)) matched.set(key, item);
  if (!matched.size) return { resumed: false, count: 0 };
  if ([...matched.keys()].some((key) => inflightItems.has(key))) {
    throw httpError(409, '上传任务正在进入暂停状态，请稍后继续');
  }

  for (const [key, item] of matched) {
    pausedUploads.delete(key);
    const waiting = waitingFiles.has(key);
    publish({
      type: 'file', state: waiting ? 'waiting-file' : token ? 'queued' : 'waiting-login',
      file_path: uploadEventPath(item), mapping_id: item.mapping_id,
      uploaded_bytes: 0, total_bytes: Number(item.size || 0),
      stage: waiting ? '另外的程序正在使用该文件，释放后将自动上传' : token ? '已恢复，等待上传通道' : '等待登录后继续上传',
    });
  }
  publishState();
  pump();
  return { resumed: true, count: matched.size };
}
function deleteMappingTransientUploads(mappingId) { database.prepare("DELETE FROM uploaded_files WHERE mapping_id = ? AND status <> 'cloud_confirmed'").run(mappingId); database.prepare('DELETE FROM upload_checkpoints WHERE mapping_id = ?').run(mappingId); for (const key of pendingUploads.keys()) if (key.startsWith(`${mappingId}::`)) pendingUploads.delete(key); for (const key of failedUploads.keys()) if (key.startsWith(`${mappingId}::`)) failedUploads.delete(key); }
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
  const matched = candidates.find((candidate) => {
    if (path.resolve(candidate.file_path) === filePath) return true;
    try { return path.relative(canonicalizePathSync(candidate.file_path), canonicalizePathSync(filePath)) === ''; }
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
const organizer = createOrganizerService({
  database,
  publish,
  env: process.env,
  fetchImpl: undiciFetch,
  getNetworkPreferences: () => networkPreferences,
  cloud: {
    isAuthenticated: () => Boolean(token),
    listChildren: organizerListCloudChildren,
    createDirectory: organizerCreateCloudDirectory,
    copyEntry: (fileId, parentId) => executeFileTask('/userres/v1/file/copy_file', { fileIds: [String(fileId)], parentId: String(parentId) }, { parentIds: [parentId] }),
    copyEntries: (fileIds, parentId) => executeFileTask('/userres/v1/file/copy_file', { fileIds: fileIds.map(String), parentId: String(parentId) }, { parentIds: [parentId] }),
    moveEntry: (fileId, parentId) => executeFileTask('/userres/v1/file/move_file', { fileIds: [String(fileId)], parentId: String(parentId) }, { parentIds: [parentId], entryIds: [fileId] }),
    moveEntries: (fileIds, parentId) => executeFileTask('/userres/v1/file/move_file', { fileIds: fileIds.map(String), parentId: String(parentId) }, { parentIds: [parentId], entryIds: fileIds }),
    renameEntry: (fileId, name) => organizerRenameCloudEntry(String(fileId), String(name)),
    deleteEntry: (fileId) => executeFileTask('/userres/v1/file/delete_file', { fileIds: [String(fileId)] }, { entryIds: [fileId] }),
    uploadBuffer: organizerUploadBuffer,
    getDownloadUrl: async (fileId) => (await getCloudDownload({ file_ids: [String(fileId)], packaged: false })).download_url,
    shareAfterOrganize: createOrganizerShare,
  },
});
const virtualLibrary = createVirtualLibraryService({
  database,
  publish,
  root: virtualLibraryRoot,
  proxyPort: embyProxyPublicPort,
  embyUpstream,
  fetchImpl: (...args) => createProxiedFetch(networkPreferences.proxy_url, undiciFetch)(...args),
  cloud: {
    listChildren: organizerListCloudChildren,
    getDownloadUrl: async (fileId) => (await getCloudDownload({ file_ids: [String(fileId)], packaged: false })).download_url,
  },
});
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
    failedUploads.delete(key);
    const stamp = `${item.size}:${item.mtime}`;
    cancelledUploads.delete(key);
    if (history.get(key) === stamp || pendingUploads.has(key) || inflight.get(key) === stamp || (queue.has(key) && `${queue.get(key).size}:${queue.get(key).mtime}` === stamp) || waitingFiles.has(key)) { skipped += 1; continue; }
    queue.set(key, item);
    queued += 1;
    publish({ type: 'file', state: token ? 'queued' : 'waiting-login', file_path: item.file_path, mapping_id: mappingId, uploaded_bytes: 0, total_bytes: item.size });
  }
  pump();
  return { queued, skipped, total: files.length };
}
function ignore(file) { const base = path.basename(file).toLowerCase(); return base.startsWith('~$') || ['.tmp', '.part', '.crdownload', '.download', '.swp', '.ds_store'].some((suffix) => base.endsWith(suffix)); }
function headers() {
  const trace = `${crypto.randomBytes(16).toString('hex')}-${crypto.randomBytes(8).toString('hex')}`;
  return buildBusinessHeaders({
    token,
    deviceId,
    profile: guangyaProfile,
    traceparent: `00-${trace}-01`,
  });
}
async function parseResponse(response, endpoint) {
  const raw = await response.text();
  if (!raw.trim() && response.ok) return { code: 0, data: {} };
  try { return JSON.parse(raw.replace(/^\uFEFF/, '')); } catch (error) { throw httpError(502, `光鸭接口 ${endpoint} 返回了非 JSON 响应（HTTP ${response.status}）：${raw.slice(0, 240)}（${error.message}）`); }
}
// 与 Rust 端 endpoint_idempotency 对齐的只读端点分类：只读接口可以安全地
// 全量重试；写接口只重试"请求肯定没被服务端受理"的失败（连接失败、429/503），
// 避免复制/分享等操作被重复执行。
const READ_ONLY_ENDPOINT_MARKERS = Object.freeze([
  '/file/get_',
  '/file/search_files',
  '/userres/v1/get_',
  '/userres/v1/check_can_flash_upload',
  '/cloudcollection/v1/list_task',
  '/cloudcollection/v1/resolve_res',
  '/developer/v1/pre_upload_status',
  '/developer/v1/upload_status',
  '/scheduler/v1/query_packaging_task',
  '/misc/v1/',
  '/assets/v1/',
  '/user/v1/',
]);
function endpointIsReadOnly(endpoint) {
  return READ_ONLY_ENDPOINT_MARKERS.some((marker) => String(endpoint || '').includes(marker));
}
const API_RETRY_ATTEMPTS = 4;
function apiRetryDelayMs(attempt) {
  const base = Math.min(300 * 2 ** attempt, 5_000);
  const jitter = base / 4;
  return Math.round(base - jitter + Math.random() * jitter * 2);
}

// 业务 POST 的有界退避重试外壳。旧实现只给错误打 retryable 标记但从不消费，
// 一次 502/429/网络抖动就把错误直接抛给 UI。
async function apiPost(
  endpoint,
  body,
  allowed = [],
  allowRefresh = true,
  timeoutMs = requestTimeoutMs,
  invalidateOnAuthFailure = true,
) {
  const readOnly = endpointIsReadOnly(endpoint);
  const attempts = readOnly ? API_RETRY_ATTEMPTS : 3;
  let lastError = null;
  for (let attempt = 0; attempt < attempts; attempt += 1) {
    if (attempt > 0) await new Promise((resolve) => setTimeout(resolve, apiRetryDelayMs(attempt - 1)));
    try {
      return await apiPostOnce(endpoint, body, allowed, allowRefresh, timeoutMs, invalidateOnAuthFailure);
    } catch (error) {
      const mutationSafe = error?.notDelivered === true || error?.httpStatus === 429 || error?.httpStatus === 503;
      const retryable = readOnly ? error?.retryable === true : mutationSafe;
      if (!retryable || attempt + 1 >= attempts) throw error;
      lastError = error;
    }
  }
  throw lastError;
}

async function apiPostOnce(
  endpoint,
  body,
  allowed = [],
  allowRefresh = true,
  timeoutMs = requestTimeoutMs,
  invalidateOnAuthFailure = true,
) {
  let response;
  try {
    response = await fetch(`${apiBase}${endpoint}`, { method: 'POST', headers: headers(), body: JSON.stringify(body || {}), signal: AbortSignal.timeout(timeoutMs) });
  } catch (cause) {
    const timedOut = ['AbortError', 'TimeoutError'].includes(cause?.name);
    const error = httpError(timedOut ? 504 : 502, timedOut ? `光鸭接口 ${endpoint} 请求超时` : `无法连接光鸭接口 ${endpoint}：${cause.message}`);
    error.httpStatus = error.statusCode;
    error.retryable = true;
    // 连接根本没有建立时请求肯定未被受理，写接口也可以安全重试。
    error.notDelivered = ['ECONNREFUSED', 'ENOTFOUND', 'EAI_AGAIN'].includes(cause?.cause?.code || cause?.code);
    error.cause = cause;
    throw error;
  }
  const payload = await parseResponse(response, endpoint);
  const code = businessResponseCode(payload);
  if (response.status === 401 || isAuthExpiredBusinessCode(code)) {
    if (allowRefresh && refreshToken) {
      await refreshSavedSession();
      return apiPost(endpoint, body, allowed, false, timeoutMs, invalidateOnAuthFailure);
    }
    if (invalidateOnAuthFailure) invalidateAuthSession();
    throw httpError(401, '登录态已失效，且自动续期失败，请重新扫码登录');
  }
  if (!response.ok || code === null || (code !== 0 && !allowed.includes(code))) {
    const error = new Error(payload.msg || (code === null
      ? `光鸭接口 ${endpoint} 返回了未标明成功状态的响应`
      : `光鸭接口失败 ${response.status}/${code}`));
    error.httpStatus = response.status;
    error.statusCode = response.status === 429 ? 429 : response.status >= 500 ? 502 : response.status >= 400 ? response.status : 400;
    error.apiCode = code;
    error.retryable = response.status >= 500 || response.status === 429 || [100, 101, 102, 103].includes(code);
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
  const parentId = String(body.parent_id || '');
  const response = await apiPost('/userres/v1/restore_share', { accessToken, fileIds: validateFileIds(body.file_ids), parentId });
  await waitOperation(response.data?.taskId);
  resetRemoteDirectoryCache();
  webDavDirectoryCache.invalidate(parentId);
  publishCloudDirectoryInvalidated([parentId], { source: 'received-share-restore' });
  return response.data || {};
}

function assertPackagingTaskActive(data) {
  const status = String(data?.status || data?.state || '').trim().toLowerCase();
  const errorCode = Number(data?.errorCode ?? data?.error_code ?? 0);
  if (['failed', 'failure', 'error', 'cancelled', 'canceled', 'expired'].includes(status)
    || (Number.isFinite(errorCode) && errorCode !== 0)) {
    throw new Error(String(data?.message || data?.msg || data?.error || '光鸭文件打包失败'));
  }
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
    const downloadUrl = String(response.data?.signedURL || response.data?.signedUrl
      || response.data?.downloadUrl || response.data?.downloadURL || response.data?.url || '');
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
    assertPackagingTaskActive(result.data);
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
    const downloadUrl = String(response.data?.signedURL || response.data?.signedUrl
      || response.data?.downloadUrl || response.data?.downloadURL || response.data?.url || '');
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
    assertPackagingTaskActive(result.data);
    await new Promise((resolve) => setTimeout(resolve, 1_000));
  }
  throw new Error('光鸭打包超过 10 分钟仍未完成，请稍后重试');
}

function formatExportBytes(value) {
  const bytes = typeof value === 'bigint' ? value : BigInt(value || 0);
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  let unit = 0;
  let divisor = 1n;
  while (unit < units.length - 1 && bytes >= divisor * 1024n) {
    divisor *= 1024n;
    unit += 1;
  }
  if (!unit) return `${bytes} B`;
  const hundredths = (bytes * 100n + divisor / 2n) / divisor;
  return `${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')} ${units[unit]}`;
}

function exportJsonFileName(names) {
  const raw = names.length === 1 ? safeCloudPathSegment(names[0]) : `光鸭秒传_${names.length}项`;
  const stem = raw.replace(/\.[^.]+$/, '').replace(/[\\/:*?"<>|]/g, '_').slice(0, 120) || '光鸭秒传';
  return `${stem}_秒传.json`;
}

function gcidExportRootSignatures(roots) {
  return roots.map((entry) => ({
    fileId: entry.fileId,
    name: entry.name,
    folder: entry.folder,
    size: entry.size,
    gcid: String(entry.gcid || '').toLowerCase(),
    modifiedAt: entry.modifiedAt,
    subtreeSize: entry.subtreeSize,
    subtreeFolders: entry.subtreeFolders,
    subtreeFiles: entry.subtreeFiles,
  }));
}

function gcidExportSelectionKey(fileIds) {
  return crypto.createHash('sha256').update(fileIds.join('\0')).digest('hex');
}

function loadGcidExportSnapshot(accountScope, selectionKey) {
  const row = database.prepare(`SELECT root_signatures_json, export_json, created_at
    FROM gcid_export_snapshots WHERE account_scope = ? AND selection_key = ?`)
    .get(accountScope, selectionKey);
  if (!row) return null;
  return {
    rootSignatures: JSON.parse(row.root_signatures_json),
    exportData: JSON.parse(row.export_json),
    createdAt: Number(row.created_at),
  };
}

function saveGcidExportSnapshot(accountScope, selectionKey, rootSignatures, exportData) {
  const now = Math.floor(Date.now() / 1000);
  database.prepare(`INSERT INTO gcid_export_snapshots
    (account_scope, selection_key, root_signatures_json, export_json, created_at, last_used_at)
    VALUES (?, ?, ?, ?, ?, ?)
    ON CONFLICT(account_scope, selection_key) DO UPDATE SET
      root_signatures_json=excluded.root_signatures_json,
      export_json=excluded.export_json,
      created_at=excluded.created_at,
      last_used_at=excluded.last_used_at`)
    .run(accountScope, selectionKey, JSON.stringify(rootSignatures), JSON.stringify(exportData), now, now);
  trimGcidExportSnapshotCache();
}

function loadGcidExportFileHash(accountScope, entry) {
  const gcid = String(entry?.gcid || '').trim().toLowerCase();
  if (!/^[0-9a-f]{40}$/.test(gcid)) return '';
  const row = database.prepare(`SELECT cid FROM gcid_export_file_hashes
    WHERE account_scope = ? AND file_id = ? AND file_size = ? AND gcid = ?`)
    .get(accountScope, String(entry.fileId), String(entry.size), gcid);
  let cid = String(row?.cid || '').trim().toUpperCase();
  if (!/^[0-9A-F]{40}$/.test(cid)) {
    const local = database.prepare(`SELECT cid FROM file_fingerprints
      WHERE size = ? AND LOWER(gcid) = ? AND LENGTH(cid) = 40
      ORDER BY updated_at DESC LIMIT 1`).get(Number(entry.size), gcid);
    cid = String(local?.cid || '').trim().toUpperCase();
  }
  if (!/^[0-9A-F]{40}$/.test(cid)) return '';
  if (row) {
    database.prepare(`UPDATE gcid_export_file_hashes SET last_used_at = ?
      WHERE account_scope = ? AND file_id = ? AND file_size = ? AND gcid = ?`)
      .run(Math.floor(Date.now() / 1000), accountScope, String(entry.fileId), String(entry.size), gcid);
  }
  return cid;
}

function saveGcidExportFileHash(accountScope, entry, file) {
  const gcid = String(file?.gcid || '').trim().toLowerCase();
  const cid = String(file?.cid || '').trim().toUpperCase();
  if (!/^[0-9a-f]{40}$/.test(gcid) || !/^[0-9A-F]{40}$/.test(cid)) return;
  database.prepare(`INSERT INTO gcid_export_file_hashes
    (account_scope, file_id, file_size, gcid, cid, last_used_at)
    VALUES (?, ?, ?, ?, ?, ?)
    ON CONFLICT(account_scope, file_id, file_size, gcid) DO UPDATE SET
      cid=excluded.cid, last_used_at=excluded.last_used_at`)
    .run(accountScope, String(entry.fileId), String(entry.size), gcid, cid, Math.floor(Date.now() / 1000));
}

async function exportGcidJson(body) {
  const diagnostics = createGcidExportDiagnostics(gcidExportDiagnosticFile, 'docker-web');
  diagnostics.write('info', 'run_started', {
    selected_roots: Array.isArray(body?.file_ids ?? body?.fileIds) ? (body.file_ids ?? body.fileIds).length : 0,
    scan_concurrency: GCID_EXPORT_SCAN_CONCURRENCY,
    file_concurrency: GCID_EXPORT_FILE_CONCURRENCY,
    range_concurrency_per_file: GCID_EXPORT_RANGE_CONCURRENCY,
    scan_attempts: GCID_EXPORT_SCAN_ATTEMPTS,
    range_attempts: GCID_EXPORT_RANGE_ATTEMPTS,
    global_range_concurrency: GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY,
    request_timeout_ms: GCID_EXPORT_REQUEST_TIMEOUT_MS,
    read_idle_timeout_ms: GCID_EXPORT_READ_IDLE_TIMEOUT_MS,
    proxy_configured: Boolean(networkPreferences.proxy_url),
  });
  try {
    const result = await exportGcidJsonWithDiagnostics(body, diagnostics);
    diagnostics.write('info', 'run_completed', {
      total_files: result.total_files,
      skipped_files: result.skipped_files_count,
      source_total_bytes: result.total_size,
    });
    return { ...result, diagnostic_run_id: diagnostics.runId };
  }
  catch (error) {
    diagnostics.write('error', 'run_failed', { error: error?.message || error });
    const safeMessage = sanitizeGcidDiagnosticText(error?.message || error || '秒传 JSON 生成失败');
    if (error instanceof Error) error.message = safeMessage;
    throw error instanceof Error ? error : new Error(safeMessage);
  }
}

async function exportGcidJsonWithDiagnostics(body, diagnostics) {
  const fileIds = validateFileIds(body.file_ids ?? body.fileIds);
  const fallbackNames = Array.isArray(body.file_names ?? body.fileNames)
    ? (body.file_names ?? body.fileNames).slice(0, fileIds.length).map((value) => String(value || '').slice(0, 255))
    : [];
  diagnostics.write('info', 'scan_started', { selected_roots: fileIds.length });
  const roots = await loadGcidExportRoots(fileIds, fallbackNames, diagnostics);
  const rootSignatures = gcidExportRootSignatures(roots);
  const selectionKey = gcidExportSelectionKey(fileIds);
  const accountScope = authSessionScope.current();
  const cachedSnapshotFiles = new Map();
  if (cacheEnabled && accountScope !== 'logged-out') {
    try {
      const snapshot = loadGcidExportSnapshot(accountScope, selectionKey);
      if (snapshot) {
        const ageMs = Math.max(0, Date.now() - snapshot.createdAt * 1000);
        for (const file of Array.isArray(snapshot.exportData?.files) ? snapshot.exportData.files : []) {
          cachedSnapshotFiles.set(String(file.path || ''), file);
        }
        const signaturesMatch = JSON.stringify(snapshot.rootSignatures) === JSON.stringify(rootSignatures);
        if (ageMs <= gcidExportSnapshotFreshMs
          && signaturesMatch
          && Number(snapshot.exportData?.skippedFilesCount || 0) === 0) {
          const exportData = { ...snapshot.exportData, generatedAt: Math.floor(Date.now() / 1000) };
          database.prepare(`UPDATE gcid_export_snapshots SET last_used_at = ?
            WHERE account_scope = ? AND selection_key = ?`)
            .run(Math.floor(Date.now() / 1000), accountScope, selectionKey);
          diagnostics.write('info', 'snapshot_cache_hit', {
            cache_age_seconds: Math.floor(ageMs / 1000),
            total_files: Number(exportData.totalFilesCount || 0),
            fresh_window_seconds: Math.floor(gcidExportSnapshotFreshMs / 1000),
          });
          publish({
            type: 'gcid-export-progress',
            stage: '已命中缓存，秒传 JSON 已生成',
            current_path: '',
            completed_files: Number(exportData.totalFilesCount || 0),
            total_files: Number(exportData.totalFilesCount || 0),
            percent: 100,
            diagnostic_run_id: diagnostics.runId,
          });
          return {
            cancelled: false,
            file_name: exportJsonFileName(roots.map((entry) => entry.name)),
            total_files: Number(exportData.totalFilesCount || 0),
            skipped_files_count: 0,
            total_size: String(exportData.totalSize || 0),
            export: exportData,
          };
        }
        diagnostics.write('info', 'snapshot_cache_miss', {
          reason: ageMs > gcidExportSnapshotFreshMs
            ? 'expired'
            : !signaturesMatch ? 'root_signature_changed' : 'partial_snapshot',
          cache_age_seconds: Math.floor(ageMs / 1000),
          fresh_window_seconds: Math.floor(gcidExportSnapshotFreshMs / 1000),
        });
      }
      else diagnostics.write('info', 'snapshot_cache_miss', { reason: 'not_found' });
    }
    catch (error) {
      diagnostics.write('warn', 'snapshot_cache_read_failed', { error: error?.message || error });
    }
  }
  const collected = await collectGcidExportEntries(fileIds, fallbackNames, diagnostics, roots);
  diagnostics.write('info', 'scan_completed', {
    roots: collected.roots.length,
    folders: collected.scannedFolders,
    discovered_entries: collected.entries.length,
  });
  const singleFolder = collected.roots.length === 1 && collected.roots[0].folder ? collected.roots[0] : null;
  const rootPrefix = singleFolder ? `${safeCloudPathSegment(singleFolder.name)}/` : '';
  const files = collected.entries
    .filter((entry) => !entry.folder)
    .map((entry) => ({
      ...entry,
      path: rootPrefix && entry.path.startsWith(rootPrefix) ? entry.path.slice(rootPrefix.length) : entry.path,
    }));
  if (!files.length) throw new Error('所选内容中没有可生成秒传 JSON 的文件');
  const totalSize = files.reduce((total, entry) => total + BigInt(entry.size), 0n);
  const plannedSampleBytes = files.reduce((total, entry) => total + cidByteRanges(entry.size)
    .reduce((rangeTotal, range) => rangeTotal + BigInt(range.end - range.start), 0n), 0n);
  let readBytes = 0n;
  let completedFiles = 0;
  let lastPublishAt = 0;
  const publishProgress = (stage, currentPath, force = false) => {
    const now = Date.now();
    if (!force && now - lastPublishAt < 250) return;
    lastPublishAt = now;
    const percent = Math.floor(completedFiles * 100 / files.length);
    publish({
      type: 'gcid-export-progress',
      phase: 'hash',
      stage,
      current_path: currentPath,
      completed_files: completedFiles,
      total_files: files.length,
      sampled_bytes: readBytes.toString(),
      planned_sample_bytes: plannedSampleBytes.toString(),
      source_total_bytes: totalSize.toString(),
      downloaded_bytes: readBytes.toString(),
      total_bytes: plannedSampleBytes.toString(),
      percent: Math.max(0, Math.min(100, percent)),
      diagnostic_run_id: diagnostics.runId,
    });
  };
  const fetchCloud = createProxiedFetch(networkPreferences.proxy_url, undiciFetch);
  const acquireRangeSlot = createGcidExportRangeGate();
  const rangeHash = async (entry, initialDownloadUrl) => {
    const ranges = cidByteRanges(entry.size);
    const parts = await mapConcurrent(ranges, GCID_EXPORT_RANGE_CONCURRENCY, async (range, rangeIndex) => retryGcidExportRange(async (attempt) => {
      if (range.start === range.end) return Buffer.alloc(0);
      const attemptStartedAt = Date.now();
      const requestFields = {
        path: entry.path,
        file_id_suffix: String(entry.fileId || '').slice(-8),
        range_index: rangeIndex,
        range_start: range.start,
        range_end_exclusive: range.end,
        expected_bytes: range.end - range.start,
        attempt: attempt + 1,
        max_attempts: GCID_EXPORT_RANGE_ATTEMPTS,
      };
      diagnostics.write('info', 'range_request_started', requestFields);
      let downloadUrl;
      try {
        downloadUrl = attempt === 0
          ? initialDownloadUrl
          : (await getCloudDownload({ file_ids: [entry.fileId], packaged: false })).download_url;
      }
      catch (error) {
        diagnostics.write('error', 'range_download_url_failed', {
          ...requestFields,
          elapsed_ms_request: Date.now() - attemptStartedAt,
          error: error?.message || error,
        });
        throw error;
      }
      const releaseRangeSlot = await acquireRangeSlot();
      try {
      const controller = new AbortController();
      const requestTimeout = setTimeout(() => {
        controller.abort(new GcidExportRangeError(`分段读取 ${entry.path} 请求超时`));
      }, GCID_EXPORT_REQUEST_TIMEOUT_MS);
      let response;
      try {
        response = await fetchCloud(downloadUrl, {
          method: 'GET',
          headers: {
            'accept-encoding': 'identity',
            range: `bytes=${range.start}-${range.end - 1}`,
          },
          signal: controller.signal,
        });
      }
      catch (error) {
        const reason = controller.signal.reason instanceof GcidExportRangeError ? controller.signal.reason : error;
        diagnostics.write('error', 'range_request_failed', {
          ...requestFields,
          elapsed_ms_request: Date.now() - attemptStartedAt,
          error: reason?.message || reason,
        });
        throw reason;
      }
      finally {
        clearTimeout(requestTimeout);
      }
      const partial = response.status === 206;
      const wholeFile = range.start === 0 && range.end === entry.size && response.ok;
      if (!partial && !wholeFile) {
        const error = new GcidExportRangeError(`云端未接受分段读取（HTTP ${response.status}）`, {
          retryable: retryableGcidExportRangeStatus(response.status),
        });
        diagnostics.write('error', 'range_response_rejected', {
          ...requestFields,
          elapsed_ms_request: Date.now() - attemptStartedAt,
          http_status: response.status,
          retryable: error.retryable,
          error: error.message,
        });
        controller.abort(error);
        throw error;
      }
      if (partial && String(response.headers.get('content-range') || '').toLowerCase() !== `bytes ${range.start}-${range.end - 1}/${entry.size}`) {
        const error = new GcidExportRangeError('云端返回的分段范围与请求不一致');
        diagnostics.write('error', 'range_content_range_mismatch', {
          ...requestFields,
          elapsed_ms_request: Date.now() - attemptStartedAt,
          http_status: response.status,
          returned_content_range: response.headers.get('content-range') || '',
          error: error.message,
        });
        controller.abort(error);
        throw error;
      }
      let bytes;
      try {
        bytes = await readGcidExportRangeBody(response.body, range.end - range.start, {
          timeoutMs: GCID_EXPORT_READ_IDLE_TIMEOUT_MS,
          abort: (error) => controller.abort(error),
        });
      }
      catch (error) {
        const reason = controller.signal.reason instanceof GcidExportRangeError ? controller.signal.reason : error;
        diagnostics.write('error', 'range_stream_failed', {
          ...requestFields,
          elapsed_ms_request: Date.now() - attemptStartedAt,
          http_status: response.status,
          error: reason?.message || reason,
        });
        throw reason;
      }
      diagnostics.write('info', 'range_request_succeeded', {
        ...requestFields,
        elapsed_ms_request: Date.now() - attemptStartedAt,
        http_status: response.status,
        received_bytes: bytes.length,
      });
      return bytes;
      }
      finally {
        releaseRangeSlot();
      }
    }, { baseDelayMs: 400 + rangeIndex * 125 }));
    return {
      cid: calculateGuangyaCidSamples(parts, entry.size),
      sampled: parts.reduce((total, bytes) => total + BigInt(bytes.length), 0n),
    };
  };
  const rangeHashWithRetry = async (entry) => {
    const download = await retryGcidExportRange(
      async (attempt) => {
        const startedAt = Date.now();
        const fields = {
          path: entry.path,
          file_id_suffix: String(entry.fileId || '').slice(-8),
          attempt: attempt + 1,
          max_attempts: GCID_EXPORT_RANGE_ATTEMPTS,
        };
        diagnostics.write('info', 'sample_download_url_started', fields);
        try {
          const result = await getCloudDownload({ file_ids: [entry.fileId], packaged: false });
          diagnostics.write('info', 'sample_download_url_succeeded', {
            ...fields,
            elapsed_ms_request: Date.now() - startedAt,
          });
          return result;
        }
        catch (error) {
          diagnostics.write('error', 'sample_download_url_failed', {
            ...fields,
            elapsed_ms_request: Date.now() - startedAt,
            error: error?.message || error,
          });
          throw error;
        }
      },
    );
    return rangeHash(entry, download.download_url);
  };
  publishProgress('正在生成秒传指纹（Range 采样）', files[0].path, true);
  const outcomes = await mapConcurrent(files, GCID_EXPORT_FILE_CONCURRENCY, async (entry, index) => {
    const fileStartedAt = Date.now();
    const fileFields = {
      file_index: index,
      path: entry.path,
      file_id_suffix: String(entry.fileId || '').slice(-8),
      size: entry.size,
      gcid_available_from_scan: /^[0-9a-f]{40}$/i.test(String(entry.gcid || '').trim()),
    };
    diagnostics.write('info', 'file_started', fileFields);
    try {
      publishProgress('正在生成秒传指纹（Range 采样）', entry.path, true);
      let gcid = String(entry.gcid || '').trim();
      if (!/^[0-9a-f]{40}$/i.test(gcid)) {
        try {
          gcid = String((await cloudEntryDetail(entry.fileId, entry.name)).gcid || '').trim();
        }
        catch (error) {
          diagnostics.write('warn', 'file_detail_refresh_failed', {
            ...fileFields,
            error: error?.message || error,
          });
        }
      }
      let cachedCid = '';
      if (cacheEnabled && accountScope !== 'logged-out' && /^[0-9a-f]{40}$/i.test(gcid)) {
        const cached = cachedSnapshotFiles.get(entry.path);
        if (String(cached?.size || '') === String(entry.size)
          && String(cached?.gcid || '').toLowerCase() === gcid.toLowerCase()
          && /^[0-9a-f]{40}$/i.test(String(cached?.cid || ''))) cachedCid = String(cached.cid).toUpperCase();
        if (!cachedCid) {
          try {
            cachedCid = loadGcidExportFileHash(accountScope, { ...entry, gcid });
          }
          catch (error) {
            diagnostics.write('warn', 'file_cache_read_failed', {
              ...fileFields,
              error: error?.message || error,
            });
          }
        }
      }
      if (cachedCid) {
        completedFiles += 1;
        publishProgress('正在生成秒传指纹（Range 采样）', entry.path, true);
        diagnostics.write('info', 'file_cache_hit', fileFields);
        return {
          file: {
            path: entry.path,
            size: String(entry.size),
            gcid: gcid.toLowerCase(),
            cid: cachedCid,
          },
        };
      }
      if (!/^[0-9a-f]{40}$/i.test(gcid)) {
        throw new Error('光鸭文件详情缺少有效 GCID，无法进行 Range 采样');
      }
      let sampled;
      try {
        sampled = await rangeHashWithRetry(entry);
      }
      catch (error) {
        diagnostics.write('error', 'sample_mode_failed', {
          ...fileFields,
          fallback_to_full_download: false,
          error: error?.message || error,
        });
        throw new Error(`CID Range 采样失败：${error?.message || error}`);
      }
      readBytes += sampled.sampled;
      const hashes = { gcid, cid: sampled.cid };
      completedFiles += 1;
      publishProgress('正在生成秒传指纹（Range 采样）', entry.path, true);
      diagnostics.write('info', 'file_succeeded', {
        ...fileFields,
        elapsed_ms_file: Date.now() - fileStartedAt,
        mode: 'sampled',
      });
      const file = {
        path: entry.path,
        size: String(entry.size),
        gcid: hashes.gcid.toLowerCase(),
        cid: hashes.cid,
      };
      if (cacheEnabled && accountScope !== 'logged-out') {
        try { saveGcidExportFileHash(accountScope, entry, file); }
        catch (error) {
          diagnostics.write('warn', 'file_cache_save_failed', {
            ...fileFields,
            error: error?.message || error,
          });
        }
      }
      return { file };
    } catch (error) {
      completedFiles += 1;
      publishProgress('正在生成秒传指纹（Range 采样）', entry.path, true);
      diagnostics.write('error', 'file_failed', {
        ...fileFields,
        elapsed_ms_file: Date.now() - fileStartedAt,
        error: error?.message || error,
      });
      return { skipped: `${entry.path}：${sanitizeGcidDiagnosticText(error?.message || error || '读取失败').slice(0, 500)}` };
    }
  });
  const hashed = outcomes.flatMap((outcome) => outcome.file ? [outcome.file] : []);
  const skippedFiles = outcomes.flatMap((outcome) => outcome.skipped ? [outcome.skipped] : []);
  if (!hashed.length) throw new Error(`秒传 JSON 生成失败：${skippedFiles[0] || '没有文件可导出'}`);
  const rootNames = collected.roots.map((entry) => entry.name);
  const exportData = {
    scriptVersion: 'guangya-gcid-export-2.0',
    exportVersion: '2.0',
    source: 'guangya',
    hashType: 'gcid',
    usesGcidInExport: true,
    usesCidInExport: true,
    usesBase62EtagsInExport: false,
    commonPath: singleFolder ? singleFolder.name : '',
    sourceFolderId: singleFolder ? singleFolder.fileId : '',
    sourceFolderName: singleFolder ? singleFolder.name : '',
    totalFilesCount: hashed.length,
    totalSize: totalSize <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(totalSize) : totalSize.toString(),
    formattedTotalSize: formatExportBytes(totalSize),
    generatedAt: Math.floor(Date.now() / 1000),
    scannedFoldersCount: collected.scannedFolders,
    skippedFilesCount: skippedFiles.length,
    skippedFiles,
    files: hashed,
  };
  if (cacheEnabled) trimGcidExportFileHashCache();
  if (cacheEnabled && accountScope !== 'logged-out' && skippedFiles.length === 0) {
    try {
      saveGcidExportSnapshot(accountScope, selectionKey, rootSignatures, exportData);
      diagnostics.write('info', 'snapshot_cache_saved', {
        total_files: hashed.length,
        fresh_window_seconds: Math.floor(gcidExportSnapshotFreshMs / 1000),
      });
    }
    catch (error) {
      diagnostics.write('warn', 'snapshot_cache_save_failed', { error: error?.message || error });
    }
  }
  publishProgress(skippedFiles.length ? '秒传 JSON 已生成，部分文件已跳过' : '秒传 JSON 已生成', '', true);
  return {
    cancelled: false,
    file_name: exportJsonFileName(rootNames),
    total_files: hashed.length,
    skipped_files_count: skippedFiles.length,
    total_size: totalSize.toString(),
    export: exportData,
  };
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
    ON CONFLICT(event_id) DO UPDATE SET share_url=excluded.share_url, status=excluded.status, action=excluded.action, error_code=NULL, message=excluded.message, resource_url=excluded.resource_url, payload=excluded.payload, updated_at=excluded.updated_at`)
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
  const response = await createProxiedFetch(networkPreferences.proxy_url, undiciFetch)(hdhiveTargetUrl(pathname), { method, headers: { 'content-type': 'application/json', 'X-GuangYa-Instance-Id': hdhiveInstanceId, 'X-GuangYa-Timestamp': timestamp, 'X-GuangYa-Signature': hdhiveSignature(method, pathname, bodyText, timestamp) }, body: body == null ? undefined : bodyText, redirect: 'error', signal: AbortSignal.timeout(30_000) });
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
      database.prepare('UPDATE auto_share_events SET notification_status = ?, error_code = ?, updated_at = ? WHERE event_id = ?').run(result.notification_status || null, result.error_code || null, Math.floor(Date.now() / 1000), eventId);
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
  // 光鸭分享不是不可变快照；它依赖当前云端资源关系。手动分享始终
  // 创建当前资源的新链接，避免复用可能已被移动或删除影响的旧链接。
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
async function createOrganizerShare({ mappingId, remoteTargetId, title, targetType = 'folder' }) {
  const normalizedMappingId = String(mappingId || '').trim();
  const targetId = validateIdentifier(remoteTargetId, '整理后分享目标 ID');
  const normalizedTitle = String(title || '').trim() || '整理后的媒体';
  if (!normalizedMappingId) throw new Error('整理后分享缺少任务 ID');
  // 云端移动、删除或覆盖可能使旧链接失效，因此整理完成后始终从 B
  // 目录创建新分享，不查询、不复用 A 目录或历史分享绑定。
  const response = await apiPost('/userres/v1/share_file', shareFilePayload([targetId], normalizedTitle));
  const data = response.data || response;
  const shareUrl = pickShareUrl(data);
  const shareId = String(shareIdFromUrl(shareUrl) || data.shareCode || data.share_code || data.shareId || data.shareID || data.share_id || '');
  if (!shareUrl || !shareId) throw new Error('光鸭没有返回完整分享链接');
  const eventId = crypto.randomUUID();
  const payload = {
    event_id: eventId,
    occurred_at: new Date().toISOString(),
    mapping_id: normalizedMappingId,
    target_key: normalizedTitle,
    target_type: targetType === 'file' ? 'file' : 'folder',
    remote_target_id: targetId,
    share_id: shareId,
    share_url: shareUrl,
    title: normalizedTitle,
    intent: 'new',
    change_hint: { added: [], changed: [], removed: [] },
  };
  if (!hdhiveEnabled) {
    saveAutoShareEvent(eventId, normalizedMappingId, normalizedTitle, shareUrl, 'disabled', '', 'B 目录新分享已创建，Hdhive 已关闭', '', payload);
    return { ...data, share_id: shareId, share_url: shareUrl, hdhive_event_id: eventId, hdhive_status: 'disabled', hdhive_message: 'B 目录新分享已创建，Hdhive 已关闭' };
  }
  saveAutoShareEvent(eventId, normalizedMappingId, normalizedTitle, shareUrl, 'sending', '', 'B 目录新分享已创建，正在通知 Hdhive', '', payload);
  try {
    const accepted = await hdhiveRequest('POST', '/api/integrations/guangya-sync/events', payload);
    const hdhiveStatus = accepted.status || 'accepted';
    saveAutoShareEvent(eventId, normalizedMappingId, normalizedTitle, shareUrl, hdhiveStatus, '', 'Hdhive 已接收整理后的 B 目录分享', '', payload);
    void pollHdhiveReceipt(eventId, normalizedMappingId, normalizedTitle, shareUrl, payload);
    return { ...data, share_id: shareId, share_url: shareUrl, hdhive_event_id: eventId, hdhive_status: hdhiveStatus, hdhive_message: 'Hdhive 已接收整理后的 B 目录分享' };
  } catch (error) {
    const message = `B 目录新分享已创建，但提交 Hdhive 失败：${error.message}`;
    saveAutoShareEvent(eventId, normalizedMappingId, normalizedTitle, shareUrl, 'delivery_failed', '', message, '', payload);
    return { ...data, share_id: shareId, share_url: shareUrl, hdhive_event_id: eventId, hdhive_status: 'delivery_failed', hdhive_message: message };
  }
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
  if (mapping.organizer_mapping_id) throw new Error('该任务已启用上传后自动整理；请在媒体整理页扫描 A 目录，光鸭会在整理完成后从 B 目录创建新分享');
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
function accountRequestHeaders(extraHeaders = {}) {
  return {
    ...buildAccountHeaders({ deviceId, profile: guangyaProfile }),
    ...extraHeaders,
  };
}
async function accountGet(endpoint, allowRefresh = true) {
  if (!token) throw new Error('尚未设置光鸭会话令牌');
  let response;
  try {
    response = await fetch(`${accountBase}${endpoint}`, { headers: accountRequestHeaders({ authorization: `Bearer ${token}` }), signal: AbortSignal.timeout(requestTimeoutMs) });
  } catch (cause) {
    throw httpError(['AbortError', 'TimeoutError'].includes(cause?.name) ? 504 : 502, `无法连接光鸭账号接口 ${endpoint}：${cause.message}`);
  }
  const payload = await parseResponse(response, endpoint);
  if (response.status === 401) {
    if (allowRefresh && refreshToken) {
      await refreshSavedSession();
      return accountGet(endpoint, false);
    }
    invalidateAuthSession();
    throw new Error('登录态已失效，且自动续期失败，请重新扫码登录');
  }
  if (!response.ok) throw httpError(response.status >= 500 ? 502 : response.status, payload.msg || `账号接口失败 ${response.status}`);
  return payload;
}
async function probeAccountProfileForScope(accessToken) {
  if (!String(accessToken || '').trim() || jwtAccountIdentity(accessToken, accountBase)) return null;
  try {
    const response = await fetch(`${accountBase}/v1/user/me`, {
      headers: accountRequestHeaders({ authorization: `Bearer ${accessToken}` }),
      signal: AbortSignal.timeout(Math.min(requestTimeoutMs, 5_000)),
    });
    if (!response.ok) return null;
    return await parseResponse(response, '/v1/user/me');
  } catch {
    return null;
  }
}
async function establishExplicitAuthSessionScope(accessToken, authPayload) {
  const needsProfile = !jwtAccountIdentity(accessToken, accountBase)
    && !accountIdFromAuthPayload(authPayload);
  const profile = needsProfile ? await probeAccountProfileForScope(accessToken) : authPayload;
  return authSessionScope.establish(accessToken, profile);
}
async function accountPost(endpoint, body, extraHeaders = {}) {
  let response;
  try {
    response = await fetch(`${accountBase}${endpoint}`, { method: 'POST', headers: accountRequestHeaders(extraHeaders), body: JSON.stringify(body || {}), signal: AbortSignal.timeout(requestTimeoutMs) });
  } catch (cause) {
    throw httpError(['AbortError', 'TimeoutError'].includes(cause?.name) ? 504 : 502, `无法连接光鸭账号接口 ${endpoint}：${cause.message}`);
  }
  return { status: response.status, payload: await parseResponse(response, endpoint) };
}
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
      usage: 'SIGN_IN',
      selected_channel: 'VERIFICATION_PHONE',
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
  await establishExplicitAuthSessionScope(token, credentials.payload);
  resetRemoteDirectoryCache();
  synchronizeWebDavCacheScope();
  replaceAuthSession(token, refreshToken);
  smsChallenges.delete(verificationId);
  status('success', '手机号登录成功，可以开始使用云盘和备份任务');
  publishState();
  pump();
  schedulePendingUploadRecovery(0);
  return { authenticated: true, is_user: isUser };
}
async function startDeviceLogin() {
  const { status: statusCode, payload } = await accountPost('/v1/auth/device/code', {
    scope: 'user',
    client_id: oauthClientId,
    meta: { scene: 'pc_login' },
  });
  if (statusCode >= 400) throw new Error(payload.error_description || payload.msg || '无法创建扫码登录任务');
  return payload.data || payload;
}
async function pollDeviceLogin(deviceCode) {
  if (!String(deviceCode || '').trim()) throw new Error('缺少扫码登录任务');
  const { status: statusCode, payload } = await accountPost('/v1/auth/token', {
    grant_type: 'urn:ietf:params:oauth:grant-type:device_code',
    device_code: deviceCode,
    client_id: oauthClientId,
    client_secret: oauthClientSecret,
  });
  const accessToken = authValue(payload, 'access_token');
  const nextRefreshToken = authValue(payload, 'refresh_token');
  if (accessToken) {
    token = String(accessToken);
    refreshToken = nextRefreshToken ? String(nextRefreshToken) : null;
    await establishExplicitAuthSessionScope(token, payload);
    resetRemoteDirectoryCache();
    synchronizeWebDavCacheScope();
    replaceAuthSession(token, refreshToken);
    status('success', '扫码登录成功，可以开始使用云盘和备份任务');
    publishState();
    pump();
    schedulePendingUploadRecovery(0);
    return { authenticated: true };
  }
  const oauthError = String(authValue(payload, 'error') || '').trim().toLowerCase();
  const description = String(authValue(payload, 'error_description') || authValue(payload, 'msg') || '').trim();
  const isPending = oauthError === 'authorization_pending'
    || oauthError === 'slow_down'
    || statusCode === 202
    || statusCode === 428
    || description === 'Precondition Required';
  if (isPending) {
    return {
      pending: true,
      slow_down: oauthError === 'slow_down',
      message: oauthError === 'slow_down' ? '请求过快，正在降低扫码轮询频率' : '等待扫码确认',
    };
  }
  throw new Error(description || oauthError || '扫码登录失败');
}
async function refreshSavedSession() {
  if (!refreshToken) return false;
  if (!refreshPromise) refreshPromise = (async () => {
    const { status: statusCode, payload } = await accountPost('/v1/auth/token', {
      grant_type: 'refresh_token',
      refresh_token: refreshToken,
      client_id: oauthClientId,
      client_secret: oauthClientSecret,
    });
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
    authSessionScope.retainAfterRefresh(token);
    synchronizeWebDavCacheScope();
    saveAuthSession(token, refreshToken);
    publishState();
    pump();
    schedulePendingUploadRecovery(0);
    return true;
  })().finally(() => { refreshPromise = null; });
  return refreshPromise;
}
async function findFolder(parentId, name) {
  let seen = 0;
  for (let page = 0; page < 100; page += 1) {
    const response = await apiPost('/userres/v1/file/get_file_list', {
      page,
      pageSize: 100,
      parentId,
      resType: 2,
      needSubFolderStat: true,
    });
    const list = Array.isArray(response.data?.list) ? response.data.list : [];
    const found = list.find((item) => Number(item.resType) === 2 && item.fileName === name);
    if (found?.fileId) return String(found.fileId);
    seen += list.length;
    const total = response.data?.total == null ? Number.NaN : Number(response.data.total);
    if (!list.length
      || (Number.isFinite(total) && total >= 0 && seen >= total)
      || (!Number.isFinite(total) && list.length < 100)) break;
  }
  return null;
}
const REMOTE_CACHE_INVALIDATED = Symbol('remote-cache-invalidated');
async function resolveRemoteDirectoryPart({ cacheKey, parentId, part, prefix, generation }) {
  const pending = remoteCacheGates.get(cacheKey);
  if (pending?.generation === generation) return pending.promise;
  const task = (async () => {
    if (generation !== remoteCacheGeneration) return REMOTE_CACHE_INVALIDATED;
    if (cacheEnabled && remoteCache.has(cacheKey)) {
      const cachedId = String(remoteCache.get(cacheKey));
      const age = Date.now() - Number(remoteCacheValidatedAt.get(cacheKey) || 0);
      if (age <= remoteDirectoryFreshMs) return cachedId;
      const verifiedId = await findFolder(parentId, part);
      if (generation !== remoteCacheGeneration) return REMOTE_CACHE_INVALIDATED;
      if (verifiedId) {
        if (verifiedId !== cachedId) {
          invalidateRemoteDirectoryIds([cachedId]);
          return REMOTE_CACHE_INVALIDATED;
        }
        remoteCacheValidatedAt.set(cacheKey, Date.now());
        return verifiedId;
      }
      invalidateRemoteDirectoryIds([cachedId]);
      return REMOTE_CACHE_INVALIDATED;
    }
    const response = await apiPost('/userres/v1/file/create_dir', {
      parentId,
      dirName: part,
      failIfNameExist: true,
    }, [159]);
    const fileId = response.data?.fileId
      || (response.code === 159 ? await findFolder(parentId, part) : null);
    if (!fileId) throw new Error(`无法创建远程目录 ${prefix}`);
    if (response.code !== 159) {
      await waitOperation(response.data?.taskId);
      webDavDirectoryCache.invalidate(parentId);
      publishCloudDirectoryInvalidated([parentId], { source: 'upload-create-directory' });
    }
    if (generation !== remoteCacheGeneration) return REMOTE_CACHE_INVALIDATED;
    const resolvedId = String(fileId);
    if (cacheEnabled) {
      remoteCache.delete(cacheKey);
      remoteCache.set(cacheKey, resolvedId);
      remoteCacheValidatedAt.set(cacheKey, Date.now());
      trimRemoteCache();
    }
    return resolvedId;
  })();
  const record = { generation, promise: task };
  remoteCacheGates.set(cacheKey, record);
  try {
    return await task;
  } finally {
    if (remoteCacheGates.get(cacheKey) === record) remoteCacheGates.delete(cacheKey);
  }
}
async function ensureRemote(baseParentId, remotePath) {
  const normalized = normalizeRemote(remotePath);
  if (!normalized) return String(baseParentId || '');
  for (let attempt = 0; attempt < 8; attempt += 1) {
    const generation = remoteCacheGeneration;
    let parentId = String(baseParentId || '');
    let prefix = '';
    let retry = false;
    for (const part of normalized.split('/')) {
      prefix = prefix ? `${prefix}/${part}` : part;
      const cacheKey = `${baseParentId || ''}::${prefix}`;
      const resolvedId = await resolveRemoteDirectoryPart({
        cacheKey,
        parentId,
        part,
        prefix,
        generation,
      });
      if (resolvedId === REMOTE_CACHE_INVALIDATED || generation !== remoteCacheGeneration) {
        retry = true;
        break;
      }
      parentId = resolvedId;
    }
    if (!retry) return parentId;
  }
  throw new Error('远程目录持续发生变化，请稍后重试');
}
const CLOUD_TASK_PENDING_CODES = new Set([147]);
const CLOUD_TASK_INVALID_CODES = new Set([145, 146, 152, 155, 163]);
function isExplicitPermanentCloudTaskFailure(error) {
  const code = Number(error?.apiCode);
  return CLOUD_TASK_INVALID_CODES.has(code)
    || (error?.retryable === false && (Number.isFinite(code) || Number.isFinite(error?.httpStatus)));
}
async function waitTask(taskId, eventPath, cancelled = () => false) {
  const deadline = Date.now() + cloudConfirmTimeoutMs;
  let attempt = 0;
  while (Date.now() < deadline) {
    if (cancelled()) throw new UploadCancelledError();
    let transientFailure = false;
    try {
      const response = await apiPost('/userres/v1/file/get_info_by_task_id', { taskId }, [...CLOUD_TASK_PENDING_CODES]);
      if (response.data?.fileId) return response.data;
      if (Number(response.code || 0) !== 147) {
        const error = new Error('云端入库成功响应缺少有效的 fileId，已停止轮询');
        error.apiCode = Number(response.code || 0);
        error.httpStatus = 200;
        error.retryable = false;
        throw error;
      }
    } catch (error) {
      if (isExplicitPermanentCloudTaskFailure(error)) error.permanentCloudTaskFailure = true;
      if (error?.retryable !== true) throw error;
      transientFailure = true;
    }
    attempt += 1;
    publish({
      type: 'progress',
      file_path: eventPath,
      percent: 100,
      bytes_per_second: 0,
      stage: transientFailure ? '云端确认暂时不可用，正在重试' : '文件已上传，云端正在入库',
    });
    const delayMs = Math.min(cloudConfirmPollMs * Math.max(1, Math.ceil(attempt / 5)), 5_000, Math.max(0, deadline - Date.now()));
    if (delayMs > 0) await new Promise((resolve) => setTimeout(resolve, delayMs));
  }
  if (cancelled()) throw new UploadCancelledError();
  const error = new Error(`云端入库超过 ${Math.round(cloudConfirmTimeoutMs / 1000)} 秒仍未完成，请稍后刷新云盘确认`);
  error.retryable = true;
  throw error;
}
async function waitOperation(taskId) {
  if (!taskId) return;
  // 单次查询失败不代表云端任务失败：任务仍在服务端执行，轮询接口的瞬时
  // 抖动应继续等待，只有连续多次失败才放弃（与 Rust 端 wait_operation_task
  // 保持一致；此前一次 502 就把整个移动/删除操作报错）。
  let consecutiveFailures = 0;
  for (let index = 0; index < 90; index += 1) {
    let response;
    try {
      response = await apiPost('/userres/v1/get_task_status', { taskId });
      consecutiveFailures = 0;
    } catch (error) {
      consecutiveFailures += 1;
      if (consecutiveFailures >= 5 || error?.httpStatus === 401) throw error;
      await new Promise((resolve) => setTimeout(resolve, 1000));
      continue;
    }
    const statusCode = Number(response.data?.status);
    const detail = response.data?.detail || {};
    if ([2, 3].includes(statusCode) && detail.code && Number(detail.code) !== 0) throw new Error(detail.msg || '文件操作失败');
    if (statusCode === 2) return;
    if (statusCode === 3) throw new Error(detail.msg || '文件操作失败');
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
  throw new Error('文件操作长时间未完成');
}
async function calculateFileHash(filePath, algorithm) {
  const hash = crypto.createHash(algorithm);
  const stream = fs.createReadStream(filePath, { highWaterMark: 2 * 1024 * 1024 });
  for await (const chunk of stream) hash.update(chunk);
  return hash.digest('hex');
}
async function calculateFileFlashHashes(filePath, size, modifiedMs, eventPath) {
  const resolvedPath = path.resolve(filePath);
  const modified = String(modifiedMs);
  const cached = cacheEnabled
    ? database.prepare('SELECT gcid, cid FROM file_fingerprints WHERE file_path = ? AND size = ? AND modified_ms = ?')
      .get(resolvedPath, size, modified)
    : null;
  if (/^[0-9A-F]{40}$/i.test(cached?.gcid || '') && /^[0-9A-F]{40}$/i.test(cached?.cid || '')) {
    publish({ type: 'progress', file_path: eventPath, percent: 0, bytes_per_second: 0, stage: '已复用秒传指纹' });
    return { gcid: cached.gcid.toUpperCase(), cid: cached.cid.toUpperCase() };
  }
  const hashes = await calculateGuangyaFileHashes(resolvedPath, size, (percent) => {
    publish({ type: 'progress', file_path: eventPath, percent: 0, bytes_per_second: 0, stage: `正在计算秒传指纹 ${percent}%` });
  });
  if (cacheEnabled) {
    database.prepare(`INSERT INTO file_fingerprints (file_path, size, modified_ms, gcid, cid, updated_at)
      VALUES (?, ?, ?, ?, ?, ?)
      ON CONFLICT(file_path) DO UPDATE SET size = excluded.size, modified_ms = excluded.modified_ms, gcid = excluded.gcid, cid = excluded.cid, updated_at = excluded.updated_at`)
      .run(resolvedPath, size, modified, hashes.gcid, hashes.cid, Math.floor(Date.now() / 1000));
    trimFileFingerprintCache();
  }
  return hashes;
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
  const ready = { ...item, size: second.size, mtime: item.history_path ? item.mtime : second.mtimeMs, source_dev: second.dev, source_ino: second.ino };
  if (!ready.replacement && ready.change_kind === 'changed' && ready.mapping_id && !ready.mapping_id.startsWith('__')) {
    const row = database.prepare(`
      SELECT size, modified_ms, remote_file_id
      FROM uploaded_files
      WHERE mapping_id = ? AND file_path = ? AND status = 'cloud_confirmed'
        AND remote_parent_id = ? AND remote_dir = ? AND relative_path = ?
    `).get(
      ready.mapping_id,
      path.resolve(uploadHistoryPath(ready)),
      ready.remote_parent_id || '',
      ready.remote_dir || '',
      ready.relative_path || '',
    );
    ready.replacement = createUploadReplacementContext({
      oldFileId: row?.remote_file_id,
      originalName: path.posix.basename(String(ready.relative_path || '').replaceAll('\\', '/')),
      previousSize: row?.size,
      previousModifiedMs: row?.modified_ms,
    });
  }
  return ready;
}
function scheduleBusyUploadRetry(key, item) {
  if (isUploadCancelled(key)) return;
  waitingFiles.set(key, item);
  publish({ type: 'file', state: 'waiting-file', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size, stage: '另外的程序正在使用该文件，释放后将自动上传' });
  publishState();
  setTimeout(async () => {
    waitingFiles.delete(key);
    if (isUploadCancelled(key)) { publishState(); return; }
    try {
      const stat = await fsp.stat(item.file_path);
      if (!stat.isFile()) return;
      if (!item.mapping_id.startsWith('__') && !mappings.some((mapping) => mapping.id === item.mapping_id && mapping.enabled)) return;
      const refreshed = { ...item, size: stat.size, mtime: item.history_path ? item.mtime : stat.mtimeMs };
      const stamp = `${refreshed.size}:${refreshed.mtime}`;
      if (history.get(key) === stamp || pendingUploads.has(key) || inflight.get(key) === stamp || (queue.has(key) && `${queue.get(key).size}:${queue.get(key).mtime}` === stamp)) return;
      queue.set(key, refreshed);
      const uploadPaused = pausedUploads.has(key);
      publish({ type: 'file', state: uploadPaused ? 'paused' : 'waiting-file', file_path: uploadEventPath(refreshed), uploaded_bytes: 0, total_bytes: refreshed.size, stage: uploadPaused ? '已暂停，可从当前断点继续' : '另外的程序正在使用该文件，释放后将自动上传' });
    } catch {
      // 文件暂时消失时等待后续文件系统事件重新入队。
    } finally {
      publishState();
      pump();
    }
  }, fileBusyRetryMs);
}
async function preflightFlashUpload(item) {
  const key = queueKey(item.mapping_id, uploadHistoryPath(item));
  assertUploadRunnable(key);
  const source = await validateWatchedSource(item);
  item.file_path = source.absolute;
  const stat = source.stat;
  item.size = stat.size;
  if (!item.history_path) item.mtime = stat.mtimeMs;
  const eventPath = uploadEventPath(item);
  assertUploadRunnable(key);
  if (loadUploadCheckpoint(item)) return { kind: 'skipped' };

  publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, bytes_per_second: 0, stage: '正在后台校验秒传' });
  const parentId = await ensureRemote(item.remote_parent_id || '', item.remote_dir);
  item.resolved_remote_parent_id = parentId;
  assertUploadRunnable(key);
  const res = { fileSize: stat.size };
  if (stat.size < 1024 * 1024) {
    publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, bytes_per_second: 0, stage: '正在后台计算秒传 MD5' });
    res.md5 = await calculateFileHash(item.file_path, 'md5');
    assertUploadRunnable(key);
  }
  const response = await apiPost('/userres/v1/get_res_center_token', {
    capacity: 2,
    name: uploadRemoteName(item),
    res,
    parentId,
  }, [156]);
  assertUploadRunnable(key);
  const data = response.data;
  if (!data?.taskId) throw new Error('光鸭没有返回上传任务 ID');
  let taskId = data.taskId;
  let instantUpload = response.code === 156;
  if (!instantUpload && stat.size >= 1024 * 1024) {
    try {
      const { gcid, cid } = await calculateFileFlashHashes(item.file_path, stat.size, stat.mtimeMs, eventPath);
      assertUploadRunnable(key);
      const flash = await apiPost('/userres/v1/check_can_flash_upload', { taskId, gcid, cid });
      instantUpload = flash.data?.canFlashUpload === true;
      if (instantUpload && flash.data?.taskId) taskId = String(flash.data.taskId);
    } catch (error) {
      status('warning', `后台秒传校验失败，稍后继续普通上传：${error.message}`);
    }
  }
  if (!instantUpload) return { kind: 'miss', data };

  assertUploadNotCancelled(key);
  clearUploadCheckpoint(item);
  publish({ type: 'progress', file_path: eventPath, percent: 100, uploaded_bytes: stat.size, total_bytes: stat.size, bytes_per_second: 0, stage: '已命中秒传' });
  if (item.mapping_id) savePendingUploadRecord(item, { taskId, remoteFileId: null });
  publish({ type: 'file', state: 'processing', file_path: eventPath, uploaded_bytes: stat.size, total_bytes: stat.size, stage: '已秒传，正在等待云端入库' });
  schedulePendingUploadRecovery(0);
  return { kind: 'accepted' };
}
async function upload(item) {
  const key = queueKey(item.mapping_id, uploadHistoryPath(item));
  assertUploadRunnable(key);
  const source = await validateWatchedSource(item);
  item.file_path = source.absolute;
  const stat = source.stat;
  item.size = stat.size;
  if (!item.history_path) item.mtime = stat.mtimeMs;
  const eventPath = uploadEventPath(item);
  assertUploadRunnable(key);
  publish({ type: 'progress', file_path: eventPath, percent: 0, uploaded_bytes: 0, total_bytes: stat.size, stage: '正在准备云端目录' });
  const parentId = await ensureRemote(item.remote_parent_id || '', item.remote_dir);
  item.resolved_remote_parent_id = parentId;
  assertUploadRunnable(key);
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
    response = await apiPost('/userres/v1/get_res_center_token', { capacity: 2, name: uploadRemoteName(item), res, parentId }, [156]);
    data = response.data;
  }
  if (!data?.taskId) throw new Error('光鸭没有返回上传任务 ID');
  let taskId = data.taskId;
  let instantUpload = response?.code === 156;
  if (!instantUpload && !checkpoint && !flashPrechecked && stat.size >= 1024 * 1024) {
    try {
      const { gcid, cid } = await calculateFileFlashHashes(item.file_path, stat.size, stat.mtimeMs, eventPath);
      const flash = await apiPost('/userres/v1/check_can_flash_upload', { taskId, gcid, cid });
      instantUpload = flash.data?.canFlashUpload === true;
      if (instantUpload && flash.data?.taskId) taskId = String(flash.data.taskId);
    } catch (error) {
      status('warning', `秒传校验失败，继续普通上传：${error.message}`);
    }
  }
  if (!instantUpload) {
    if (uploadCredentialsExpired(data)) {
      publish({ type: 'progress', file_path: eventPath, uploaded_bytes: Number(checkpoint?.uploadedBytes || 0), total_bytes: stat.size, bytes_per_second: 0, stage: '上传凭证已过期，正在刷新后续传' });
      const resumed = await resumeUploadParams(data, stat.size);
      data = resumed.params;
      taskId = data.taskId || taskId;
      if (checkpoint?.checkpoint) {
        checkpoint.params = data;
        saveUploadCheckpoint(item, data, checkpoint.checkpoint, checkpoint.uploadedBytes);
      }
    }
    if (!data.creds || !data.objectPath) throw new Error('光鸭没有返回完整上传凭证');
    let currentParams = data;
    let multipartCheckpoint = checkpoint?.checkpoint
      ? { ...checkpoint.checkpoint, file: item.file_path }
      : undefined;
    const uploadedAtStart = Math.max(0, Number(checkpoint?.uploadedBytes || 0));
    let lastUploadedBytes = uploadedAtStart;
    let credentialRefreshes = 0;
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
        endpoint: currentParams.fullEndPoint || currentParams.endPoint,
        cname: Boolean(currentParams.fullEndPoint),
        secure: true,
        timeout: ossTimeoutMs,
        retryMax: ossRetryMax,
        requestErrorRetryHandle: () => {
          publish({ type: 'progress', file_path: eventPath, uploaded_bytes: lastUploadedBytes, total_bytes: stat.size, stage: 'OSS 分片超时，正在自动重试', bytes_per_second: 0 });
          return true;
        },
      });
      activeUploadClients.set(key, client);
      try {
        assertUploadRunnable(key);
        await client.multipartUpload(currentParams.objectPath, item.file_path, {
          checkpoint: multipartCheckpoint,
          partSize: uploadPartSize(stat.size, multipartMode),
          parallel: ossParallel,
          timeout: ossTimeoutMs,
          progress: (fraction, nextCheckpoint) => {
            if (isUploadCancelled(key) || isUploadPauseRequested(key)) {
              client.cancel();
              return;
            }
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
        assertUploadNotCancelled(key);
        break;
      } catch (error) {
        if (isUploadPauseRequested(key)) throw new UploadPausedError();
        if (isUploadCancelled(key) || isUploadCancellationError(error)) throw new UploadCancelledError();
        if (isUploadSecurityTokenExpired(error) && credentialRefreshes < 3) {
          credentialRefreshes += 1;
          publish({ type: 'progress', file_path: eventPath, uploaded_bytes: lastUploadedBytes, total_bytes: stat.size, bytes_per_second: 0, stage: 'OSS 上传凭证已过期，正在刷新后续传' });
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
      } finally {
        if (activeUploadClients.get(key) === client) activeUploadClients.delete(key);
      }
    }
    clearUploadCheckpoint(item);
  } else {
    clearUploadCheckpoint(item);
    publish({ type: 'progress', file_path: eventPath, percent: 100, uploaded_bytes: stat.size, total_bytes: stat.size, stage: '已命中秒传' });
  }
  if (item.mapping_id) {
    assertUploadNotCancelled(key);
    const pendingTask = { taskId, remoteFileId: null };
    savePendingUploadRecord(item, pendingTask);
  }
  publish({ type: 'progress', file_path: eventPath, percent: 100, uploaded_bytes: stat.size, total_bytes: stat.size, bytes_per_second: 0, stage: '已上传，正在等待云端入库' });
  publish({ type: 'file', state: 'processing', file_path: eventPath, uploaded_bytes: stat.size, total_bytes: stat.size, stage: '已上传，正在等待云端入库' });
  let taskData;
  try { taskData = await waitTask(taskId, eventPath, () => isUploadCancelled(key)); }
  catch (error) {
    if (isUploadCancellationError(error)) throw error;
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
  if (isUploadCancelled(key)) {
    clearPendingUpload(key);
    return false;
  }
  if (!pendingUploads.has(key)) return false;
  if (item.replacement) {
    let replacementTask = activeUploadReplacements.get(key);
    if (!replacementTask) {
      replacementTask = (async () => {
        const resolvedParentId = String(item.resolved_remote_parent_id ?? await ensureRemote(item.remote_parent_id || '', item.remote_dir));
        publish({ type: 'progress', file_path: uploadEventPath(item), percent: 100, uploaded_bytes: item.size, total_bytes: item.size, bytes_per_second: 0, stage: '新版本已入库，正在安全替换旧文件' });
        await safelyReplaceUploadedFile({
          replacement: item.replacement,
          newFileId: taskData.remoteFileId,
          listEntries: () => fetchWebDavChildren(resolvedParentId),
          renameEntry: renameRemote,
          deleteEntry: (fileId) => deleteWebDavEntry({
            entry: { id: fileId, parentId: resolvedParentId, name: item.replacement.backupName, isDirectory: false },
          }),
        });
      })();
      activeUploadReplacements.set(key, replacementTask);
    }
    try {
      await replacementTask;
    } catch (error) {
      error.replacementPending = true;
      schedulePendingUploadRecovery();
      throw error;
    } finally {
      if (activeUploadReplacements.get(key) === replacementTask) activeUploadReplacements.delete(key);
    }
  }
  if (!confirmPendingUploadRecord(key, taskData.taskId, taskData.remoteFileId)) return false;
  const hasResolvedParent = item.resolved_remote_parent_id !== undefined
    && item.resolved_remote_parent_id !== null;
  if (hasResolvedParent || !item.remote_dir) {
    const resolvedParentId = String(item.resolved_remote_parent_id ?? item.remote_parent_id ?? '');
    webDavDirectoryCache.invalidate(resolvedParentId);
    publishCloudDirectoryInvalidated([resolvedParentId], { source: 'upload-confirmed' });
  } else {
    // Older persisted upload records did not retain the resolved leaf directory.
    // Clear safely instead of announcing an incorrect parent to mounted clients.
    webDavDirectoryCache.clear();
    publishCloudDirectoryInvalidated([], { all: true, source: 'upload-confirmed-legacy' });
  }
  const mapping = mappings.find((entry) => entry.id === item.mapping_id && entry.enabled);
  if (mapping) {
    try {
      const current = await fsp.stat(item.file_path);
      const sourceChanged = current.isFile() && (
        current.size !== item.size
        || current.mtimeMs !== item.mtime
        || (item.source_dev != null && Number(current.dev) !== Number(item.source_dev))
        || (item.source_ino != null && Number(current.ino) !== Number(item.source_ino))
      );
      if (sourceChanged) {
        publish({
          type: 'file',
          state: 'waiting-file',
          file_path: uploadEventPath(item),
          uploaded_bytes: 0,
          total_bytes: current.size,
          stage: '检测到源文件仍在写入，等待完整后重新上传',
        });
        await enqueue(mapping, item.file_path);
        return true;
      }
    } catch (error) {
      if (error.code !== 'ENOENT') status('warning', `核对上传源文件失败：${uploadEventPath(item)}：${error.message}`);
    }
  }
  clearAutoShareFailure(item);
  if (mapping?.organizer_mapping_id) {
    try {
      await organizer.notifyUpload({
        mappingId: mapping.organizer_mapping_id,
        remoteFileId: taskData.remoteFileId,
        relativePath: item.relative_path,
        shareAfter: mapping.auto_share === true,
      });
      status('success', mapping.auto_share
        ? '文件已确认入库，已进入“先整理 B 目录、再重新分享”流程'
        : '文件已确认入库，已进入云盘自动整理流程');
    } catch (error) {
      status('error', `文件已确认入库，但上传后自动整理排队失败；为避免分享 A 目录后失效，本次没有执行原自动分享：${error.message}`);
    }
  } else {
    try { await scheduleAutoShare(item, taskData); }
    catch (error) { status('error', `文件已确认入库，但自动分享排队失败：${error.message}`); }
  }
  try {
    const action = await applySourcePolicy(item);
    if (action) status('success', action);
  } catch (error) {
    if (error.code !== 'ENOENT') status('warning', `文件已确认入库，但上传后策略执行失败：${error.message}`);
  }
  if (recovered && item.cleanup_path && isWithinRoot(manualUploadRoot, item.cleanup_path)) await fsp.rm(item.cleanup_path, { recursive: true, force: true });
  publish({ type: 'file', state: 'done', file_path: uploadEventPath(item), uploaded_bytes: item.size, total_bytes: item.size });
  return true;
}
function scheduleCloudUploadRetry(key, item, reason) {
  if (isUploadCancelled(key)) return;
  if (!item?.file_path) {
    status('error', `云端明确拒绝上传任务，且本地源文件信息不足，无法自动重传：${reason}`);
    return;
  }
  waitingFiles.set(key, item);
  publish({ type: 'file', state: 'waiting-file', file_path: uploadEventPath(item), stage: '云端入库失败，稍后将重新上传' });
  setTimeout(async () => {
    waitingFiles.delete(key);
    if (isUploadCancelled(key)) { publishState(); return; }
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
function isTransientUploadError(error) {
  if (error?.retryable === true) return true;
  if (['AbortError', 'TimeoutError'].includes(error?.name)) return true;
  if (error instanceof TypeError) return true;
  const code = String(error?.code || error?.cause?.code || '');
  return /^(?:ECONNRESET|ECONNREFUSED|EHOSTUNREACH|ENETUNREACH|ETIMEDOUT|UND_ERR_)/.test(code);
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
      if (!token || inflight.has(key) || !pendingUploads.has(key) || isUploadCancelled(key)) continue;
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
        const data = await waitTask(row.task_id, eventPath, () => isUploadCancelled(key));
        if (!item) {
          if (!confirmPendingUploadRecord(key, row.task_id, data.fileId)) continue;
          status('warning', `已恢复云端入库确认，但旧记录缺少任务上下文，未执行自动分享和源文件策略：${eventPath}`);
        } else {
          await finalizeConfirmedUpload(key, item, { taskId: row.task_id, remoteFileId: data.fileId }, true);
        }
      } catch (error) {
        if (isUploadCancellationError(error) || isUploadCancelled(key)) {
          clearPendingUpload(key);
          continue;
        }
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
  const candidate = [...queue.entries()].find(([key, item]) => !inflight.has(key) && !isUploadCancelled(key) && !pausedUploads.has(key) && !flashPreflightCached(key, item));
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
    assertUploadRunnable(key);
    return preflightFlashUpload(item);
  }).then(async (result) => {
    assertUploadNotCancelled(key);
    if (result.kind === 'miss') {
      if (!mappingAcceptsUpload(item)) {
        flashPreflightCache.delete(key);
        return;
      }
      flashPreflightCache.set(key, { stamp: uploadStamp(item), data: result.data, createdAt: Date.now() });
      prependQueuedItem(key, item);
      publish({ type: 'file', state: isUploadPauseRequested(key) ? 'paused' : 'queued', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size, stage: isUploadPauseRequested(key) ? '已暂停，可从当前断点继续' : '秒传未命中，等待上传通道' });
      return;
    }
    if (result.kind === 'skipped') {
      if (!mappingAcceptsUpload(item)) {
        flashPreflightCache.delete(key);
        return;
      }
      flashPreflightCache.set(key, { stamp: uploadStamp(item), data: null, createdAt: Date.now() });
      prependQueuedItem(key, item);
      publish({ type: 'file', state: isUploadPauseRequested(key) ? 'paused' : 'queued', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size, stage: isUploadPauseRequested(key) ? '已暂停，可从当前断点继续' : '已有上传断点，等待上传通道' });
      return;
    }
    flashPreflightCache.delete(key);
    if (result.kind === 'accepted') {
      pausedUploads.delete(key);
      queuePauseRequests.delete(key);
      return;
    }
  }).catch((error) => {
    if (isUploadCancelled(key)) {
      preserveSource = false;
      clearUploadCheckpoint(item);
      publish({ type: 'file', state: 'cancelled', file_path: uploadEventPath(item), mapping_id: item.mapping_id, total_bytes: item.size, stage: '已取消' });
      return;
    }
    if (isUploadPauseError(error) || isUploadPauseRequested(key)) {
      if (mappingAcceptsUpload(item)) prependQueuedItem(key, item);
      const checkpoint = loadUploadCheckpoint(item);
      publish({ type: 'file', state: 'paused', file_path: uploadEventPath(item), mapping_id: item.mapping_id, uploaded_bytes: Number(checkpoint?.uploadedBytes || 0), total_bytes: item.size, stage: pausedUploads.has(key) ? '已暂停，可从当前断点继续' : '队列已暂停，可从当前断点继续' });
      return;
    }
    if (isUploadCancellationError(error)) {
      preserveSource = false;
      clearUploadCheckpoint(item);
      publish({ type: 'file', state: 'cancelled', file_path: uploadEventPath(item), mapping_id: item.mapping_id, total_bytes: item.size, stage: '已取消' });
      return;
    }
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
    queuePauseRequests.delete(key);
    activeFlashPreflights = Math.max(0, activeFlashPreflights - 1);
    publishState();
    pump();
  });
}
function pump() {
  if (paused || !token) { publishState(); return; }
  while (active < uploadConcurrency && queue.size) {
    const candidate = [...queue.entries()].find(([key]) => !inflight.has(key) && !isUploadCancelled(key) && !pausedUploads.has(key));
    if (!candidate) break;
    const [key, item] = candidate;
    queue.delete(key);
    failedUploads.delete(key);
    inflight.set(key, `${item.size}:${item.mtime}`);
    inflightItems.set(key, item);
    active += 1;
    publish({ type: 'file', state: 'preparing', file_path: uploadEventPath(item), uploaded_bytes: 0, total_bytes: item.size });
    let preserveSource = false;
    prepareUploadItem(item).then((ready) => {
      Object.assign(item, ready);
      assertUploadRunnable(key);
      return upload(item);
    }).then(async (taskData) => {
      assertUploadNotCancelled(key);
      pausedUploads.delete(key);
      queuePauseRequests.delete(key);
      if (taskData.pending) {
        preserveSource = true;
        status('warning', `文件已上传到 OSS，云端尚未确认入库；已保留记录并会自动重试：${uploadEventPath(item)}：${taskData.pendingError}`);
        publish({ type: 'file', state: 'processing', file_path: uploadEventPath(item), stage: '等待云端入库，下次将自动恢复确认' });
        return;
      }
      await finalizeConfirmedUpload(key, item, taskData);
    }).catch((error) => {
      if (isUploadCancelled(key)) {
        clearUploadCheckpoint(item);
        clearPendingUpload(key);
        publish({ type: 'file', state: 'cancelled', file_path: uploadEventPath(item), mapping_id: item.mapping_id, total_bytes: item.size, stage: '已取消' });
        return;
      }
      if (isUploadPauseError(error) || isUploadPauseRequested(key)) {
        preserveSource = true;
        if (mappingAcceptsUpload(item)) prependQueuedItem(key, item);
        const checkpoint = loadUploadCheckpoint(item);
        publish({ type: 'file', state: 'paused', file_path: uploadEventPath(item), mapping_id: item.mapping_id, uploaded_bytes: Number(checkpoint?.uploadedBytes || 0), total_bytes: item.size, stage: pausedUploads.has(key) ? '已暂停，可从当前断点继续' : '队列已暂停，可从当前断点继续' });
        return;
      }
      if (isUploadCancellationError(error)) {
        clearUploadCheckpoint(item);
        clearPendingUpload(key);
        publish({ type: 'file', state: 'cancelled', file_path: uploadEventPath(item), mapping_id: item.mapping_id, total_bytes: item.size, stage: '已取消' });
        return;
      }
      if (isFileBusyError(error)) {
        preserveSource = true;
        scheduleBusyUploadRetry(key, item);
        return;
      }
      if (error.replacementPending) {
        preserveSource = true;
        status('warning', `新版本已入库，但安全替换尚未完成，将在后台继续：${uploadEventPath(item)}：${error.message}`);
        publish({ type: 'file', state: 'processing', file_path: uploadEventPath(item), uploaded_bytes: item.size, total_bytes: item.size, stage: '正在等待安全替换旧文件' });
        return;
      }
      if (error.requeueUpload || isTransientUploadError(error)) {
        preserveSource = true;
        scheduleCloudUploadRetry(key, item, error.message);
        return;
      }
      recordAutoShareFailure(item, error);
      console.error(`上传失败：${item.file_path}：${error.stack || error.message}`);
      preserveSource = true;
      failedUploads.set(key, item);
      publish({ type: 'file', state: 'error', file_path: uploadEventPath(item), total_bytes: item.size, error: error.message });
    }).finally(async () => {
      if (!preserveSource && item.cleanup_path && isWithinRoot(manualUploadRoot, item.cleanup_path)) await fsp.rm(item.cleanup_path, { recursive: true, force: true });
      inflight.delete(key);
      inflightItems.delete(key);
      queuePauseRequests.delete(key);
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
  failedUploads.delete(key);
  const mark = `${stat.size}:${stat.mtimeMs}`;
  if (cancelledUploads.get(key) === mark) return;
  if (cancelledUploads.has(key)) cancelledUploads.delete(key);
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
    if (mapping.organizer_mapping_id) {
      try {
        await organizer.notifyUpload({ mappingId: mapping.organizer_mapping_id, remoteFileId: reused.remoteFileId, relativePath: item.relative_path, shareAfter: mapping.auto_share === true });
      } catch (error) {
        status('error', `历史文件无需重复上传，但上传后自动整理排队失败；本次没有分享 A 目录：${error.message}`);
      }
    } else if (mapping.auto_share && !reuseAutoShareBinding(item, reused.sourceMappingId)) {
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
    awaitWriteFinish: { stabilityThreshold: fileStabilityMs, pollInterval: polling ? 1000 : 200 },
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
async function apiOverview() { const assets = await apiPost('/assets/v1/get_assets', {}); let profile = {}; try { profile = await accountGet('/v1/user/me'); } catch {} return { assets: assets.data || {}, profile: profile?.data || profile || {} }; }
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
  async function fetchRemotePage(remotePage) {
    if (!query && (requestedType || requestedExtension)) {
      const folder = requestedType === 'folder';
      const fileType = !folder && (SEARCH_FILE_TYPES[requestedType] || fileTypeForExtension(requestedExtension));
      const body = { parentId: '*', pageSize: 100, page: remotePage, resType: folder ? 2 : 1, orderBy: 3, sortType: 1 };
      if (fileType) body.fileTypes = [fileType];
      return apiPost('/userres/v1/file/get_file_list', body);
    }
    return apiPost('/userres/v1/file/search_files', { name: query, pageSize: 100, page: remotePage });
  }

  const requiresLocalPagination = Boolean(requestedExtension || (query && requestedType));
  if (requiresLocalPagination) {
    const start = page * 100;
    const end = start + 100;
    const filtered = [];
    let result = null;
    let remoteCount = 0;
    let remoteTotal = 0;
    let exhausted = false;
    for (let remotePage = 0; remotePage < 1000; remotePage += 1) {
      const current = await fetchRemotePage(remotePage);
      result ||= current;
      const data = current.data || {};
      const remoteList = Array.isArray(data.list) ? data.list : [];
      remoteCount += remoteList.length;
      const reportedTotal = Number(data.total);
      if (Number.isFinite(reportedTotal)) remoteTotal = Math.max(remoteTotal, reportedTotal);
      filtered.push(...remoteList.filter((record) => matchesSearchType(record, requestedType)
        && (!requestedExtension || (Number(record?.resType) !== 2 && cloudFileExtension(record) === requestedExtension))));
      if (!remoteList.length || (Number.isFinite(reportedTotal) && remoteCount >= reportedTotal)) {
        exhausted = true;
        break;
      }
      if (filtered.length > end) break;
    }
    const list = filtered.slice(start, end);
    return {
      ...(result || { code: 0, msg: 'success' }),
      data: {
        ...(result?.data || {}),
        list,
        total: exhausted ? filtered.length : Math.max(end + 1, filtered.length),
        remote_total: remoteTotal || remoteCount,
        remote_count: remoteCount,
        page,
        pageSize: 100,
        page_size: 100,
      },
    };
  }

  const result = await fetchRemotePage(page);
  const data = result.data || {};
  const remoteList = Array.isArray(data.list) ? data.list : [];
  const list = remoteList.filter((record) => matchesSearchType(record, requestedType));
  return {
    ...result,
    data: {
      ...data,
      list,
      total: Number(data.total ?? list.length),
      remote_total: Number(data.total ?? remoteList.length),
      remote_count: remoteList.length,
      page,
      pageSize: 100,
      page_size: 100,
    },
  };
}

function validateIdentifiers(values, label) {
  if (!Array.isArray(values) || !values.length) throw new Error(`请至少提供一个${label}`);
  const unique = [];
  const seen = new Set();
  for (const value of values) {
    if (!['string', 'number', 'bigint'].includes(typeof value)) throw new Error(`${label}无效，请刷新后重试`);
    if (typeof value === 'number' && (!Number.isSafeInteger(value) || value < 0)) throw new Error(`${label}无效，请刷新后重试`);
    const normalized = String(value).trim();
    if (!normalized || normalized.length > 256 || !/^[A-Za-z0-9._:-]+$/.test(normalized)) {
      throw new Error(`${label}无效，请刷新后重试`);
    }
    if (seen.has(normalized)) continue;
    seen.add(normalized);
    unique.push(normalized);
  }
  return unique;
}
function validateFileIds(fileIds) { return validateIdentifiers(fileIds, '文件 ID'); }
function validateTaskIds(taskIds) { return validateIdentifiers(taskIds, '离线任务 ID'); }
function validateShareIds(shareIds) { return validateIdentifiers(shareIds, '分享记录 ID'); }
function validateIdentifier(value, label) { return validateIdentifiers([value], label)[0]; }
function validateOptionalIdentifier(value, label) {
  if (value == null || String(value).trim() === '') return '';
  return validateIdentifier(value, label);
}
function validateInteger(value, label, { minimum = 0, maximum = Number.MAX_SAFE_INTEGER } = {}) {
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    throw new Error(`${label}必须是 ${minimum} 到 ${maximum} 之间的整数`);
  }
  return parsed;
}
function queryInteger(url, name, fallback, options) {
  const value = url.searchParams.get(name);
  return value == null || value === '' ? fallback : validateInteger(value, name, options);
}
function queryIntegerList(url, camelName, snakeName, { maximum = 255 } = {}) {
  const values = [...url.searchParams.getAll(camelName), ...url.searchParams.getAll(snakeName)]
    .flatMap((value) => String(value).split(','))
    .map((value) => value.trim())
    .filter(Boolean);
  if (!values.length) return [];
  return [...new Set(values.map((value) => validateInteger(value, camelName, { minimum: 0, maximum })))];
}
function validateCloudName(value, label) {
  const name = String(value || '').trim();
  if (!name || name === '.' || name === '..' || name.length > 255 || /[\\/:*?"<>|\u0000-\u001f\u007f]/.test(name)) {
    throw new Error(`${label}无效`);
  }
  return name;
}
function validateOfflineUrl(value) {
  const raw = String(value ?? '');
  if (!raw || raw.length > 8192 || /[\u0000-\u001f\u007f]/.test(raw)) throw new Error('离线资源地址无效');
  const source = raw.trim();
  if (!source || !/^(?:https?|magnet|ed2k):/i.test(source)) throw new Error('离线资源地址只支持 HTTP、HTTPS、Magnet 或 ED2K');
  if (/^https?:/i.test(source)) {
    let parsed;
    try { parsed = new URL(source); } catch { throw new Error('HTTP 离线资源地址无效'); }
    if (!parsed.hostname || !['http:', 'https:'].includes(parsed.protocol)) throw new Error('HTTP 离线资源地址无效');
  } else if (/^magnet:/i.test(source) && !/^magnet:\?/i.test(source)) {
    throw new Error('Magnet 离线资源地址无效');
  } else if (/^ed2k:/i.test(source) && !/^ed2k:\/\//i.test(source)) {
    throw new Error('ED2K 离线资源地址无效');
  }
  return source;
}
function validateOfflineFileIndexes(value, source) {
  if (value == null) return [];
  if (!Array.isArray(value)) throw new Error('file_indexes 必须是非负整数数组');
  const fileIndexes = [...new Set(value.map((item) => {
    if (typeof item !== 'number' || !Number.isSafeInteger(item) || item < 0) throw new Error('file_indexes 必须是非负整数数组');
    return item;
  }))];
  if (fileIndexes.length > 1000) throw new Error('单次最多选择 1000 个资源文件');
  if (fileIndexes.length && !/^magnet:/i.test(source)) throw new Error('只有 Magnet 资源支持选择文件序号');
  return fileIndexes;
}
function offlineResolvedName(payload) {
  const data = payload?.data || payload || {};
  const info = data.urlResInfo || data.btResInfo || data.emuleResInfo || data.resourceInfo || data;
  return String(info?.fileName || info?.name || info?.title || '').trim();
}
function ed2kFileName(source) {
  if (!/^ed2k:\/\/\|file\|/i.test(source)) return '';
  const encoded = String(source).split('|')[2] || '';
  try { return decodeURIComponent(encoded.replaceAll('+', '%20')).trim(); }
  catch { return encoded.trim(); }
}
function magnetDisplayName(source) {
  if (!/^magnet:\?/i.test(source)) return '';
  try { return String(new URLSearchParams(source.slice(source.indexOf('?') + 1)).get('dn') || '').trim(); }
  catch { return ''; }
}
function offlineSourceName(source) {
  return magnetDisplayName(source) || ed2kFileName(source);
}
function offlineTemporaryName(originalName, source) {
  const extension = /^ed2k:/i.test(source) ? path.extname(originalName) : '';
  const safeExtension = /^\.[A-Za-z0-9]{1,16}$/.test(extension) ? extension : '';
  return `gy_${crypto.randomUUID().replaceAll('-', '').slice(0, 20)}${safeExtension}`;
}
function protectedOfflineSource(source, temporaryName) {
  if (/^magnet:\?/i.test(source)) {
    const separator = source.indexOf('?');
    const base = source.slice(0, separator);
    const parameters = source.slice(separator + 1).split('&').filter((parameter) => {
      const rawKey = parameter.split('=', 1)[0] || '';
      try { return decodeURIComponent(rawKey).toLowerCase() !== 'dn'; }
      catch { return rawKey.toLowerCase() !== 'dn'; }
    });
    return parameters.length ? `${base}?${parameters.join('&')}` : base;
  }
  if (/^ed2k:\/\/\|file\|/i.test(source)) {
    const parts = source.split('|');
    if (parts.length > 4) parts[2] = temporaryName;
    return parts.join('|');
  }
  return source;
}
function saveOfflineNameRestore(taskId, originalName, temporaryName) {
  const now = Math.floor(Date.now() / 1000);
  database.prepare(`INSERT INTO offline_name_restores
      (task_id, original_name, temporary_name, status, attempts, created_at, updated_at)
    VALUES (?, ?, ?, 'pending', 0, ?, ?)
    ON CONFLICT(task_id) DO UPDATE SET
      original_name = excluded.original_name,
      temporary_name = excluded.temporary_name,
      file_id = NULL,
      status = 'pending',
      attempts = 0,
      last_error = NULL,
      updated_at = excluded.updated_at`)
    .run(taskId, originalName, temporaryName, now, now);
}
function removeOfflineNameRestores(taskIds) {
  const statement = database.prepare('DELETE FROM offline_name_restores WHERE task_id = ?');
  for (const taskId of taskIds) statement.run(taskId);
}
function annotateOfflineNameRestores(data) {
  const list = Array.isArray(data?.list) ? data.list : Array.isArray(data?.taskList) ? data.taskList : [];
  if (!list.length) return data;
  const query = database.prepare('SELECT original_name, temporary_name, status, last_error FROM offline_name_restores WHERE task_id = ?');
  for (const task of list) {
    const taskId = String(task?.taskId || task?.id || '').trim();
    if (!taskId) continue;
    const restore = query.get(taskId);
    if (!restore) continue;
    task.nameRestoreStatus = restore.status === 'completed' ? 'restored' : (restore.last_error ? 'failed' : 'pending');
    task.nameRestoreError = restore.last_error || '';
    task.originalName = restore.original_name;
    if (restore.status === 'completed') task.fileName = restore.original_name;
  }
  return data;
}
let offlineRestoreReconcileRunning = false;
async function reconcileOfflineNameRestores(suppliedData) {
  if (offlineRestoreReconcileRunning || !token || !pendingOfflineNameRestoreCount()) return annotateOfflineNameRestores(suppliedData);
  offlineRestoreReconcileRunning = true;
  try {
    const data = suppliedData || (await apiPost('/cloudcollection/v1/list_task', { cursor: '', pageSize: 100 })).data || {};
    const list = Array.isArray(data.list) ? data.list : Array.isArray(data.taskList) ? data.taskList : [];
    const tasks = new Map(list.map((task) => [String(task?.taskId || task?.id || ''), task]));
    const pending = database.prepare("SELECT task_id, original_name, temporary_name, attempts, updated_at FROM offline_name_restores WHERE status = 'pending'").all();
    const complete = database.prepare("UPDATE offline_name_restores SET file_id = ?, status = 'completed', last_error = NULL, updated_at = ? WHERE task_id = ?");
    const failed = database.prepare("UPDATE offline_name_restores SET file_id = ?, attempts = attempts + 1, last_error = ?, updated_at = ? WHERE task_id = ?");
    const now = Math.floor(Date.now() / 1000);
    let renamed = false;
    for (const restore of pending) {
      const task = tasks.get(String(restore.task_id));
      if (!task || Number(task.status ?? task.taskStatus ?? task.state) !== 2) continue;
      const fileId = String(task.fileId || '').trim();
      if (!fileId) continue;
      if (String(task.fileName || '').trim() === restore.original_name) {
        complete.run(fileId, now, restore.task_id);
        continue;
      }
      if (restore.attempts > 0 && now - Number(restore.updated_at || 0) < 15) continue;
      try {
        await renameRemote(fileId, restore.original_name);
        complete.run(fileId, now, restore.task_id);
        task.fileName = restore.original_name;
        renamed = true;
      } catch (error) {
        failed.run(fileId, String(error?.message || error || '恢复原文件名失败').slice(0, 500), now, restore.task_id);
      }
    }
    if (renamed) {
      resetRemoteDirectoryCache();
      webDavDirectoryCache.clear();
      publishCloudDirectoryInvalidated([], { all: true, source: 'offline-name-restore' });
    }
    database.prepare("DELETE FROM offline_name_restores WHERE status = 'completed' AND updated_at < ?").run(now - 30 * 86_400);
    return annotateOfflineNameRestores(data);
  } finally {
    offlineRestoreReconcileRunning = false;
  }
}
// 精确失效版本的云端写操作执行器（对齐 Rust 端 publish_cloud_mutation 语义）：
// - `parentIds`：内容发生变化的父目录（新建/复制/移动的目标目录）；
// - `entryIds`：被移动/删除的条目，先从挂载缓存反查它们的原父目录；
// - 只有条目无法定位且 `unknownRequiresFullRefresh` 时才退化为全量失效。
// 旧行为是无条件三重全清 + all:true 广播，整理器逐条操作时会把所有缓存
// 打成筛子并让前端持续全量刷新。
async function executeFileTask(endpoint, payload, invalidation = {}) {
  const result = await apiPost(endpoint, payload);
  await waitOperation(result.data?.taskId);
  const parentIds = new Set((invalidation.parentIds || [])
    .map((value) => String(value ?? ''))
    .filter(Boolean));
  const entryIds = [...new Set((invalidation.entryIds || [])
    .map((value) => String(value ?? ''))
    .filter(Boolean))];
  const unknownRequiresFullRefresh = invalidation.unknownRequiresFullRefresh ?? true;
  if (!parentIds.size && !entryIds.length) {
    resetRemoteDirectoryCache();
    webDavDirectoryCache.clear();
    publishCloudDirectoryInvalidated([], { all: true, source: endpoint });
    return result.data || {};
  }
  const located = webDavDirectoryCache.invalidateEntries(entryIds);
  for (const parentId of located.parents) parentIds.add(parentId);
  const all = unknownRequiresFullRefresh && !located.allLocated;
  if (all) {
    resetRemoteDirectoryCache();
    webDavDirectoryCache.clear();
  } else {
    invalidateRemoteDirectoryIds([...entryIds, ...parentIds]);
    for (const parentId of parentIds) webDavDirectoryCache.invalidate(parentId);
  }
  publishCloudDirectoryInvalidated(all ? [] : [...parentIds], { all, source: endpoint });
  return result.data || {};
}
function publishRecycleBinChanged(source) {
  publish({ type: 'cloud-recycle-bin-changed', source: String(source) });
}
async function organizerListCloudChildren(parentId, reportPage = null) {
  if (!token) throw new Error('请先登录光鸭云盘');
  const records = [];
  let complete = false;
  for (let page = 0; page < 1000; page += 1) {
    const startedAt = Date.now();
    let response;
    const fetchPage = () => apiPost('/userres/v1/file/get_file_list', {
      page,
      pageSize: 100,
      parentId: String(parentId || ''),
      orderBy: 0,
      sortType: 0,
      needSubFolderStat: true,
    });
    try {
      response = reportPage
        ? await retryGcidExportScan(async (attempt) => {
          const attemptStartedAt = Date.now();
          try {
            return await fetchPage();
          }
          catch (error) {
            reportPage({
              level: error?.retryable === true ? 'warn' : 'error',
              event: 'scan_folder_page_attempt_failed',
              fields: {
                page,
                attempt: attempt + 1,
                max_attempts: GCID_EXPORT_SCAN_ATTEMPTS,
                retrying: error?.retryable === true && attempt + 1 < GCID_EXPORT_SCAN_ATTEMPTS,
                elapsed_ms_request: Date.now() - attemptStartedAt,
                error: error?.message || error,
              },
            });
            throw error;
          }
        })
        : await fetchPage();
    }
    catch (error) {
      reportPage?.({
        level: 'error',
        event: 'scan_folder_page_failed',
        fields: { page, elapsed_ms_request: Date.now() - startedAt, error: error?.message || error },
      });
      throw error;
    }
    const list = Array.isArray(response.data?.list) ? response.data.list : [];
    records.push(...list);
    const total = response.data?.total == null ? Number.NaN : Number(response.data.total);
    reportPage?.({
      level: 'info',
      event: 'scan_folder_page_succeeded',
      fields: {
        page,
        elapsed_ms_request: Date.now() - startedAt,
        page_entries: list.length,
        collected_entries: records.length,
        reported_total: Number.isFinite(total) ? total : null,
      },
    });
    if (!list.length
      || (Number.isFinite(total) && total >= 0 && records.length >= total)
      || (!Number.isFinite(total) && list.length < 100)) {
      complete = true;
      break;
    }
  }
  reconcileRemoteDirectoryCache(parentId, records, { complete });
  return records;
}
async function organizerCreateCloudDirectory(parentId, name) {
  const response = await apiPost('/userres/v1/file/create_dir', {
    parentId: String(parentId || ''),
    dirName: validateCloudName(name, '整理目录名称'),
    failIfNameExist: true,
  });
  await waitOperation(response.data?.taskId);
  resetRemoteDirectoryCache();
  webDavDirectoryCache.clear();
  publishCloudDirectoryInvalidated([parentId], { source: 'organizer-create-directory' });
  return response.data || {};
}
async function organizerRenameCloudEntry(fileId, name) {
  await renameRemote(fileId, name);
  // 重命名不改变目录成员关系：精确失效被改名条目及其所在父目录即可。
  // 整理器会并发地逐条改名，旧的"全量三重清空"会把所有缓存打成筛子。
  invalidateRemoteDirectoryIds([fileId]);
  const located = webDavDirectoryCache.invalidateEntries([fileId]);
  publishCloudDirectoryInvalidated(located.parents, { all: false, source: 'organizer-rename' });
}
async function organizerUploadBuffer(parentId, name, bytes) {
  const temporaryRoot = path.join(manualUploadRoot, 'organizer', crypto.randomUUID());
  const fileName = validateCloudName(name, '刮削文件名');
  const temporaryFile = path.join(temporaryRoot, fileName);
  await fsp.mkdir(temporaryRoot, { recursive: true });
  await fsp.writeFile(temporaryFile, Buffer.isBuffer(bytes) ? bytes : Buffer.from(bytes));
  const stat = await fsp.stat(temporaryFile);
  const mappingId = `__organizer__:${crypto.randomUUID()}`;
  const item = {
    mapping_id: mappingId,
    file_path: temporaryFile,
    history_path: temporaryFile,
    event_path: `云端刮削/${fileName}`,
    cleanup_path: temporaryRoot,
    remote_parent_id: String(parentId || ''),
    remote_dir: '',
    relative_path: fileName,
    change_kind: 'added',
    size: stat.size,
    mtime: stat.mtimeMs,
  };
  let completed = false;
  try {
    const outcome = await upload(item);
    if (outcome?.pending || !outcome?.remoteFileId) throw new Error(outcome?.pendingError || '刮削文件已上传，云端仍在确认入库');
    const key = queueKey(mappingId, temporaryFile);
    await finalizeConfirmedUpload(key, item, { taskId: outcome.taskId, remoteFileId: outcome.remoteFileId });
    completed = true;
    return { fileId: outcome.remoteFileId, id: outcome.remoteFileId };
  } finally {
    if (completed) await fsp.rm(temporaryRoot, { recursive: true, force: true });
  }
}
async function renameRemote(fileId, newName) { await apiPost('/userres/v1/file/rename', { fileId, newName }); }
async function batchRename(renames) {
  const work = (Array.isArray(renames) ? renames : []).map((item) => ({ fileId: String(item.fileId || ''), currentName: String(item.currentName || ''), newName: String(item.newName || '') })).filter((item) => item.currentName !== item.newName);
  if (!work.length) throw new Error('没有需要重命名的项目');
  const seen = new Set();
  for (const item of work) { const name = item.newName.trim(); if (!name || /[\\/:*?"<>|]/.test(name)) throw new Error(`无效的文件名：${item.newName}`); const key = name.toLocaleLowerCase(); if (seen.has(key)) throw new Error(`存在重复目标名称：${name}`); seen.add(key); }
  const staged = work.map((item, index) => ({ item, temporary: `.__gy_tmp_${crypto.randomUUID().replaceAll('-', '')}_${index}` }));
  let stagedCount = 0;
  let mutated = false;
  try {
    for (const entry of staged) { try { await renameRemote(entry.item.fileId, entry.temporary); mutated = true; stagedCount += 1; } catch (error) { for (const rollback of staged.slice(0, stagedCount).reverse()) { try { await renameRemote(rollback.item.fileId, rollback.item.currentName); } catch {} } throw new Error(`暂存重命名失败（${entry.item.currentName}）：${error.message}`); } }
    for (let index = 0; index < staged.length; index += 1) { const entry = staged[index]; try { await renameRemote(entry.item.fileId, entry.item.newName); } catch (error) { for (const rollback of staged.slice(0, index).reverse()) { try { await renameRemote(rollback.item.fileId, rollback.item.currentName); } catch {} } for (const rollback of staged.slice(index).reverse()) { try { await renameRemote(rollback.item.fileId, rollback.item.currentName); } catch {} } throw new Error(`目标重命名失败（${entry.item.newName}）：${error.message}`); } }
    return { renamed: staged.length };
  } finally {
    // 失效必须放在 finally：两阶段重命名中途失败且回滚也失败时，云端已经
    // 处于中间状态（残留 .__gy_tmp_* 名字），只有成功路径失效会让 UI 和
    // 挂载端继续展示改名前的旧列表。重命名不改变目录成员关系，按条目精确
    // 失效即可。
    if (mutated) {
      const fileIds = staged.map((entry) => entry.item.fileId);
      invalidateRemoteDirectoryIds(fileIds);
      const located = webDavDirectoryCache.invalidateEntries(fileIds);
      publishCloudDirectoryInvalidated(located.parents, { all: false, source: 'batch-rename' });
    }
  }
}
const webDavDirectoryCache = createDirectoryCache({
  onDirectoryInvalidated: (fileId) => invalidateRemoteDirectoryIds([fileId]),
});
const recycleClearTask = createRecycleClearTaskCoordinator({
  loadState: () => appStateValue('recycle_clear_task_v1'),
  saveState: (value) => saveAppStateValue('recycle_clear_task_v1', value),
  clearState: () => deleteAppStateValue('recycle_clear_task_v1'),
  apiPost: (endpoint, body, { timeoutMs }) => apiPost(endpoint, body, [], false, timeoutMs, false),
  deadlineMs: recycleClearDeadlineMs,
  pollMs: recycleClearPollMs,
  unknownGuardMs: recycleClearUnknownGuardMs,
  requestTimeoutMs,
  scope: () => authSessionScope.current(),
  onTerminal: ({ outcome }) => {
    // 清空回收站不影响普通目录树，只需要刷新回收站视图本身。
    publishRecycleBinChanged(`recycle-clear-${outcome}`);
  },
});
const webDavDirectoryRefreshTimer = setInterval(() => {
  if (!token) return;
  // 先对齐账号 scope，防止切换账号后的窗口期里用新账号令牌刷新旧账号目录。
  synchronizeWebDavCacheScope();
  void webDavDirectoryCache.refreshActive().catch(() => {});
}, 5_000);
webDavDirectoryRefreshTimer.unref?.();

function synchronizeWebDavCacheScope() {
  // 缓存作用域按账号而不是按令牌：令牌每 20 分钟轮换，用令牌哈希作 scope
  // 会让每次会话刷新都清空全部挂载目录缓存。仅在拿不到账号 scope 时才
  // 退回令牌哈希。
  const accountScope = authSessionScope.current();
  webDavDirectoryCache.setScope(accountScope
    || (token
      ? crypto.createHash('sha256').update(String(token)).digest('base64url')
      : 'logged-out'));
}
synchronizeWebDavCacheScope();
async function fetchWebDavChildren(parentId) {
  const records = [];
  let complete = false;
  for (let page = 0; page < 1000; page += 1) {
    const result = await apiPost('/userres/v1/file/get_file_list', {
      page,
      pageSize: 100,
      parentId: String(parentId || ''),
      orderBy: 0,
      sortType: 0,
      needSubFolderStat: true,
    });
    const list = Array.isArray(result.data?.list) ? result.data.list : [];
    records.push(...list);
    const total = result.data?.total == null ? Number.NaN : Number(result.data.total);
    if (!list.length
      || (Number.isFinite(total) && total >= 0 && records.length >= total)
      || (!Number.isFinite(total) && list.length < 100)) {
      complete = true;
      break;
    }
  }
  reconcileRemoteDirectoryCache(parentId, records, { complete });
  return records;
}
async function listWebDavChildren(parentId, options) {
  if (!token) throw new WebDavError(503, '请先登录光鸭云盘');
  synchronizeWebDavCacheScope();
  const normalizedParentId = String(parentId || '');
  return webDavDirectoryCache.get(
    normalizedParentId,
    () => fetchWebDavChildren(normalizedParentId),
    options,
  );
}
async function createWebDavDirectory({ parentId, name }) {
  const normalizedParentId = String(parentId || '');
  const result = await apiPost('/userres/v1/file/create_dir', {
    parentId: normalizedParentId,
    dirName: name,
    failIfNameExist: true,
  });
  await waitOperation(result.data?.taskId);
  webDavDirectoryCache.invalidate(normalizedParentId);
  publishCloudDirectoryInvalidated([normalizedParentId], { source: 'webdav-mkcol' });
  return result.data || {};
}
async function deleteWebDavEntry({ entry }) {
  const result = await apiPost('/userres/v1/file/delete_file', { fileIds: [entry.id] });
  await waitOperation(result.data?.taskId);
  const normalizedParentId = String(entry.parentId || '');
  webDavDirectoryCache.invalidate(normalizedParentId);
  if (entry.isDirectory) {
    webDavDirectoryCache.invalidateSubtree(entry.id);
    invalidateRemoteDirectoryIds([entry.id]);
  }
  publishCloudDirectoryInvalidated([normalizedParentId], { source: 'webdav-delete' });
}
async function moveWebDavEntry({ entry, parentId, name }) {
  const sourceParentId = String(entry.parentId || '');
  const destinationParentId = String(parentId || '');
  const moved = sourceParentId !== destinationParentId;
  if (entry.isDirectory) invalidateRemoteDirectoryIds([entry.id]);
  if (moved) {
    const result = await apiPost('/userres/v1/file/move_file', {
      fileIds: [entry.id],
      parentId: destinationParentId,
    });
    await waitOperation(result.data?.taskId);
    webDavDirectoryCache.invalidate(sourceParentId);
    webDavDirectoryCache.invalidate(destinationParentId);
  }
  if (entry.name !== name) {
    try {
      await renameRemote(entry.id, name);
      webDavDirectoryCache.invalidate(destinationParentId);
    } catch (error) {
      if (moved) {
        try {
          const rollback = await apiPost('/userres/v1/file/move_file', {
            fileIds: [entry.id],
            parentId: sourceParentId,
          });
          await waitOperation(rollback.data?.taskId);
          webDavDirectoryCache.invalidate(sourceParentId);
          webDavDirectoryCache.invalidate(destinationParentId);
        } catch (rollbackError) {
          throw new WebDavError(500, `${error.message}；恢复资源原目录也失败：${rollbackError.message}`);
        }
      }
      throw error;
    }
  }
  webDavDirectoryCache.invalidate(sourceParentId);
  webDavDirectoryCache.invalidate(destinationParentId);
  publishCloudDirectoryInvalidated(
    [sourceParentId, destinationParentId],
    { source: 'webdav-move' },
  );
}
async function copyWebDavEntry({ entry, parentId, name }) {
  const normalizedParentId = String(parentId || '');
  const before = (await listWebDavChildren(normalizedParentId, { force: true, foreground: true }))
    .map(normalizeWebDavEntry);
  if (entry.name !== name && before.some((item) => item.name === entry.name)) {
    throw new WebDavError(409, `目标目录中已有 ${entry.name}，无法安全完成改名复制`);
  }
  const beforeIds = new Set(before.map((item) => item.id));
  const result = await apiPost('/userres/v1/file/copy_file', {
    fileIds: [entry.id],
    parentId: normalizedParentId,
  });
  await waitOperation(result.data?.taskId);
  webDavDirectoryCache.invalidate(normalizedParentId);
  if (entry.name === name) {
    publishCloudDirectoryInvalidated([normalizedParentId], { source: 'webdav-copy' });
    return;
  }
  const after = (await listWebDavChildren(normalizedParentId, { force: true, foreground: true }))
    .map(normalizeWebDavEntry);
  const copied = after.find((item) => item.name === entry.name && !beforeIds.has(item.id));
  if (!copied?.id) throw new WebDavError(409, '云端复制已完成，但无法定位副本进行重命名');
  try {
    await renameRemote(copied.id, name);
  } catch (error) {
    try { await deleteWebDavEntry({ entry: copied }); } catch {}
    throw error;
  }
  webDavDirectoryCache.invalidate(normalizedParentId);
  publishCloudDirectoryInvalidated([normalizedParentId], { source: 'webdav-copy' });
}
async function putWebDavFile({ request, parentId, name, existing }) {
  const temporaryRoot = path.join(manualUploadRoot, 'webdav', crypto.randomUUID());
  const temporaryName = existing ? `.__gy_dav_${crypto.randomUUID().replaceAll('-', '')}` : name;
  const temporaryFile = path.join(temporaryRoot, temporaryName);
  await fsp.mkdir(temporaryRoot, { recursive: true });
  try {
    await pipeline(request, fs.createWriteStream(temporaryFile));
    const stat = await fsp.stat(temporaryFile);
    const item = {
      mapping_id: '',
      file_path: temporaryFile,
      event_path: `[WebDAV]/${name}`,
      remote_parent_id: String(parentId || ''),
      remote_dir: '',
      relative_path: name,
      change_kind: existing ? 'changed' : 'added',
      size: stat.size,
      mtime: stat.mtimeMs,
    };
    const uploaded = await upload(item);
    if (!uploaded.remoteFileId) {
      throw new WebDavError(503, uploaded.pendingError || '文件已上传，但云端暂未确认入库');
    }
    const uploadedEntry = {
      id: String(uploaded.remoteFileId),
      parentId: String(parentId || ''),
      name: temporaryName,
      isDirectory: false,
    };
    let backup = null;
    if (existing) {
      backup = {
        ...existing,
        parentId: String(parentId || ''),
        name: `.__gy_dav_backup_${crypto.randomUUID().replaceAll('-', '')}`,
      };
      try {
        await moveWebDavEntry({ entry: existing, parentId, name: backup.name });
      } catch (error) {
        try { await deleteWebDavEntry({ entry: uploadedEntry }); } catch {}
        throw error;
      }
    }
    try {
      if (temporaryName !== name) await renameRemote(uploadedEntry.id, name);
    } catch (error) {
      let rollbackError = null;
      if (backup) {
        try { await moveWebDavEntry({ entry: backup, parentId, name }); }
        catch (reason) { rollbackError = reason; }
      }
      try { await deleteWebDavEntry({ entry: uploadedEntry }); } catch {}
      if (rollbackError) throw new WebDavError(500, `${error.message}；恢复被覆盖文件也失败：${rollbackError.message}`);
      throw error;
    }
    if (backup) await deleteWebDavEntry({ entry: backup });
    const normalizedParentId = String(parentId || '');
    webDavDirectoryCache.invalidate(normalizedParentId);
    publishCloudDirectoryInvalidated([normalizedParentId], { source: 'webdav-put' });
    return { id: String(uploaded.remoteFileId) };
  } finally {
    await fsp.rm(temporaryRoot, { recursive: true, force: true });
  }
}
function webDavEtagMatches(value, etag) {
  const expected = String(etag || '').replace(/^W\//i, '');
  return String(value || '').split(',').map((item) => item.trim()).some((item) => item === '*' || item.replace(/^W\//i, '') === expected);
}
function finishWebDavConditional(response, statusCode, entry) {
  response.writeHead(statusCode, {
    etag: entry.etag,
    'last-modified': new Date(entry.modifiedAt).toUTCString(),
  });
  response.end();
}
async function readWebDavFile({ request, response, entry, headOnly }) {
  const ifMatch = request.headers['if-match'];
  if (ifMatch && !webDavEtagMatches(ifMatch, entry.etag)) {
    finishWebDavConditional(response, 412, entry);
    return;
  }
  const ifUnmodifiedSince = Date.parse(String(request.headers['if-unmodified-since'] || ''));
  if (!ifMatch && Number.isFinite(ifUnmodifiedSince) && entry.modifiedAt > ifUnmodifiedSince + 999) {
    finishWebDavConditional(response, 412, entry);
    return;
  }
  const ifNoneMatch = request.headers['if-none-match'];
  if (ifNoneMatch && webDavEtagMatches(ifNoneMatch, entry.etag)) {
    finishWebDavConditional(response, 304, entry);
    return;
  }
  const ifModifiedSince = Date.parse(String(request.headers['if-modified-since'] || ''));
  if (!ifNoneMatch && Number.isFinite(ifModifiedSince) && entry.modifiedAt <= ifModifiedSince + 999) {
    finishWebDavConditional(response, 304, entry);
    return;
  }
  const download = await getCloudDownload({ file_ids: [entry.id], packaged: false });
  const headers = {};
  if (request.headers.range) headers.range = request.headers.range;
  const upstream = await fetch(download.download_url, {
    method: 'GET',
    headers,
    signal: AbortSignal.timeout(ossTimeoutMs),
  });
  if (!upstream.ok && upstream.status !== 206 && upstream.status !== 304 && upstream.status !== 416) {
    throw new WebDavError(upstream.status === 404 ? 404 : 502, `云端文件读取失败（HTTP ${upstream.status}）`);
  }
  const responseHeaders = {
    'accept-ranges': upstream.headers.get('accept-ranges') || 'bytes',
    'content-type': upstream.headers.get('content-type') || 'application/octet-stream',
    etag: entry.etag,
    'last-modified': new Date(entry.modifiedAt).toUTCString(),
  };
  for (const name of ['content-length', 'content-range', 'content-disposition']) {
    const value = upstream.headers.get(name);
    if (value) responseHeaders[name] = value;
  }
  response.writeHead(upstream.status, responseHeaders);
  if (headOnly || !upstream.body) {
    await upstream.body?.cancel();
    response.end();
    return;
  }
  await pipeline(upstream.body, response);
}
const handleWebDav = createWebDavHandler({
  prefix: '/dav',
  listChildren: listWebDavChildren,
  createDirectory: createWebDavDirectory,
  deleteEntry: deleteWebDavEntry,
  moveEntry: moveWebDavEntry,
  copyEntry: copyWebDavEntry,
  putFile: putWebDavFile,
  readFile: readWebDavFile,
});
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
    failedUploads.delete(historyKey);
    const stamp = `${item.size}:${item.mtime}`;
    cancelledUploads.delete(historyKey);
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
  if (request.method === 'GET' && url.pathname === '/api/settings/network') {
    return json(response, 200, publicNetworkPreferences(), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/settings/network') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    return json(response, 200, updateNetworkPreferences(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/network/test') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    const target = String(body.target || '').trim().toLowerCase();
    const organizerState = organizer.state();
    const result = await testNetworkTarget(target, {
      proxyUrl: body.proxy_url ?? body.proxy ?? networkPreferences.proxy_url,
      tmdbApiBase: body.tmdb_api_base || organizerState.settings.tmdb_api_base,
      tmdbApiKey: body.tmdb_api_key || '',
      hdhiveBaseUrl: body.hdhive_base_url || hdhiveBaseUrl,
      fetchImpl: undiciFetch,
    });
    return json(response, 200, result, { 'cache-control': 'no-store' });
  }
  if (request.method === 'GET' && url.pathname === '/api/organizer') {
    return json(response, 200, organizer.state(), { 'cache-control': 'no-store' });
  }
  if (request.method === 'PUT' && url.pathname === '/api/organizer/settings') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    return json(response, 200, organizer.updateSettings(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/organizer/test') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    return json(response, 200, await organizer.testConnection(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/organizer/scrape-selected') {
    const body = await readBody(request, { maxBytes: 128 * 1024 });
    return json(response, 200, await organizer.scrapeSelected(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/organizer/mappings') {
    const body = await readBody(request, { maxBytes: 32 * 1024 });
    return json(response, 200, await organizer.addMapping(body));
  }
  const organizerMappingMatch = url.pathname.match(/^\/api\/organizer\/mappings\/([^/]+)$/);
  if (organizerMappingMatch && request.method === 'PATCH') {
    const body = await readBody(request, { maxBytes: 32 * 1024 });
    return json(response, 200, await organizer.updateMapping(decodeURIComponent(organizerMappingMatch[1]), body));
  }
  if (organizerMappingMatch && request.method === 'DELETE') {
    return json(response, 200, await organizer.removeMapping(decodeURIComponent(organizerMappingMatch[1])));
  }
  const organizerScanMatch = url.pathname.match(/^\/api\/organizer\/mappings\/([^/]+)\/scan$/);
  if (organizerScanMatch && request.method === 'POST') {
    return json(response, 200, await organizer.scanMapping(decodeURIComponent(organizerScanMatch[1])));
  }
  const organizerJobMatch = url.pathname.match(/^\/api\/organizer\/jobs\/([^/]+)\/(run|retry|rearchive|share)$/);
  if (organizerJobMatch && request.method === 'POST') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    const jobId = decodeURIComponent(organizerJobMatch[1]);
    const result = organizerJobMatch[2] === 'run'
      ? await organizer.runJob(jobId, body)
      : organizerJobMatch[2] === 'rearchive'
        ? await organizer.rearchiveJob(jobId, body)
        : organizerJobMatch[2] === 'share'
          ? await organizer.shareJob(jobId)
          : await organizer.retryJob(jobId, body);
    return json(response, 200, result);
  }
  const organizerJobDeleteMatch = url.pathname.match(/^\/api\/organizer\/jobs\/([^/]+)$/);
  if (organizerJobDeleteMatch && request.method === 'DELETE') {
    const body = await readBody(request, { maxBytes: 4 * 1024 });
    return json(response, 200, await organizer.removeJob(decodeURIComponent(organizerJobDeleteMatch[1]), body));
  }
  if (request.method === 'GET' && url.pathname === '/api/virtual-library') {
    return json(response, 200, virtualLibrary.info(), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/virtual-library/settings') {
    const body = await readBody(request, { maxBytes: 4 * 1024 });
    return json(response, 200, virtualLibrary.updateSettings(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/virtual-library/mappings') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    return json(response, 200, virtualLibrary.upsert(body), { 'cache-control': 'no-store' });
  }
  const virtualLibrarySyncMatch = url.pathname.match(/^\/api\/virtual-library\/mappings\/([^/]+)\/sync$/);
  if (virtualLibrarySyncMatch && request.method === 'POST') {
    return json(response, 200, virtualLibrary.sync(decodeURIComponent(virtualLibrarySyncMatch[1])), { 'cache-control': 'no-store' });
  }
  const virtualLibraryMappingMatch = url.pathname.match(/^\/api\/virtual-library\/mappings\/([^/]+)$/);
  if (virtualLibraryMappingMatch && request.method === 'DELETE') {
    return json(response, 200, virtualLibrary.remove(decodeURIComponent(virtualLibraryMappingMatch[1])), { 'cache-control': 'no-store' });
  }
  if (request.method === 'GET' && url.pathname === '/api/mount') {
    return json(response, 200, {
      enabled: true,
      running: true,
      configured: webdavAccessControl.required(),
      local_only: true,
      protocol: 'webdav',
      endpoint: webdavEndpoint,
      username: webdavUsername,
      password: '',
      password_hint: webdavAccessControl.required() ? '已设置；输入新密码可更新' : '尚未设置，请输入 12 位以上密码',
      error: null,
    });
  }
  if (request.method === 'POST' && url.pathname === '/api/mount/credentials') {
    const body = await readBody(request, { maxBytes: 4 * 1024 });
    const nextUsername = normalizeWebDavUsername(body.username);
    const nextPassword = normalizeWebDavPassword(body.password);
    webdavAccessControl.updateCredentials(request, nextUsername, nextPassword);
    webdavUsername = nextUsername;
    saveAppStateValue('webdav_username', webdavUsername);
    return json(response, 200, {
      enabled: true,
      running: true,
      configured: true,
      local_only: true,
      protocol: 'webdav',
      endpoint: webdavEndpoint,
      username: webdavUsername,
      password: '',
      password_hint: '已设置；输入新密码可更新',
      error: null,
    }, { 'cache-control': 'no-store' });
  }
  if (request.method === 'GET' && url.pathname === '/api/mount/native') {
    return json(response, 200, nativeMountManager.info(), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/mount/native/options') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    const options = normalizeNativeMountOptions(body.options || body);
    const result = nativeMountManager.setOptions(options);
    saveAppStateValue('native_mount_options', JSON.stringify(nativeMountManager.options()));
    return json(response, 200, result, { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/mount/native/start') {
    if (!webdavAccessControl.required()) throw new Error('请先设置独立的 WebDAV 账号密码');
    const body = await readBody(request, { maxBytes: 4 * 1024 });
    const password = normalizeWebDavPassword(body.password);
    if (!(await webdavAccessControl.verifyCode(password))) throw new Error('WebDAV 挂载密码错误');
    const result = await nativeMountManager.start({
      endpoint: `http://127.0.0.1:${webdavPort}/dav/`,
      username: webdavUsername,
      password,
    });
    return json(response, 200, result, { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/mount/native/stop') {
    return json(response, 200, await nativeMountManager.stop(), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/settings/transfer') {
    const body = await readBody(request);
    const transfer = updateTransferSettings(body);
    return json(response, 200, { ...transfer, transfer });
  }
  if (request.method === 'GET' && url.pathname === '/api/settings/offline') return json(response, 200, offlineSettings());
  if (request.method === 'POST' && url.pathname === '/api/settings/offline') {
    const body = await readBody(request, { maxBytes: 4 * 1024 });
    return json(response, 200, updateOfflineSettings(body));
  }
  if (request.method === 'GET' && url.pathname === '/api/settings/cache') return json(response, 200, cacheSettings());
  if (request.method === 'POST' && url.pathname === '/api/settings/cache') {
    const body = await readBody(request);
    return json(response, 200, updateCacheSettings(body));
  }
  if (request.method === 'GET' && url.pathname === '/api/developer/settings') {
    return json(response, 200, await developerSettingsForCurrentAccount(), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/developer/credentials') {
    const body = await readBody(request, { maxBytes: 8 * 1024 });
    return json(response, 200, updateDeveloperCredentials(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/developer/test') {
    const body = await readBody(request, { maxBytes: 4 * 1024 });
    return json(response, 200, await testDeveloperCredentials(body.probe_file_id ?? body.probeFileId), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/developer/mode') {
    const body = await readBody(request, { maxBytes: 4 * 1024 });
    return json(response, 200, await updateDeveloperMode(body.enabled), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/developer/targets') {
    const body = await readBody(request, { maxBytes: 8 * 1024 });
    return json(response, 200, upsertDeveloperTarget(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'DELETE' && url.pathname.startsWith('/api/developer/targets/')) {
    const id = decodeURIComponent(url.pathname.split('/').pop());
    return json(response, 200, deleteDeveloperTarget(id), { 'cache-control': 'no-store' });
  }
  if (request.method === 'GET' && url.pathname === '/api/developer/transfers') {
    return json(response, 200, { list: listDeveloperTransfers(url.searchParams.get('limit')) }, { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/developer/transfers') {
    const body = await readBody(request, { maxBytes: 32 * 1024 });
    return json(response, 202, await startDeveloperTransfer(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'GET' && url.pathname === '/api/cache') return json(response, 200, cacheState());
  if (request.method === 'POST' && url.pathname === '/api/cache/clear') return json(response, 200, clearManagedCaches());
  if (request.method === 'GET' && url.pathname === '/api/state') return json(response, 200, state());
  if (request.method === 'GET' && url.pathname === '/api/events') { response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache', connection: 'keep-alive' }); response.write(`data: ${JSON.stringify({ type: 'state', state: state() })}\n\n`); clients.add(response); request.on('close', () => clients.delete(response)); return; }
  if (request.method === 'POST' && url.pathname === '/api/auth/device/start') return json(response, 200, await startDeviceLogin());
  if (request.method === 'POST' && url.pathname === '/api/auth/device/poll') { const body = await readBody(request); return json(response, 200, await pollDeviceLogin(body.device_code)); }
  if (request.method === 'POST' && url.pathname === '/api/auth/sms/send') { const body = await readBody(request); return json(response, 200, await sendSmsLogin(body)); }
  if (request.method === 'POST' && url.pathname === '/api/auth/sms/login') { const body = await readBody(request); return json(response, 200, await completeSmsLogin(body)); }
  if (request.method === 'POST' && url.pathname === '/api/auth') {
    const body = await readBody(request);
    token = String(body.token || '').trim().replace(/^Bearer\s+/i, '') || null;
    refreshToken = null;
    await establishExplicitAuthSessionScope(token);
    resetRemoteDirectoryCache();
    synchronizeWebDavCacheScope();
    replaceAuthSession(token, null);
    publishState();
    pump();
    schedulePendingUploadRecovery(0);
    return json(response, 200, state());
  }
  if (request.method === 'GET' && url.pathname === '/api/assets') return json(response, 200, await apiPost('/assets/v1/get_assets', {}));
  if (request.method === 'GET' && url.pathname === '/api/global-config') return json(response, 200, await apiPost('/misc/v1/get_global_config', {}));
  if (request.method === 'GET' && url.pathname === '/api/overview') return json(response, 200, await apiOverview());
  if (request.method === 'GET' && url.pathname === '/api/files') {
    const body = {
      page: Math.max(0, Math.floor(Number(url.searchParams.get('page') || 0) || 0)),
      pageSize: 100,
      parentId: url.searchParams.get('parentId') || '',
      orderBy: 0,
      sortType: 0,
    };
    if (url.searchParams.get('resType') === '2') body.resType = 2;
    if (url.searchParams.get('refresh') === '1') {
      webDavDirectoryCache.invalidate(body.parentId);
      resetRemoteDirectoryCache();
    }
    const result = await apiFileReadWithDeveloperFallback('/userres/v1/file/get_file_list', body, fileListRequestTimeoutMs);
    const records = Array.isArray(result.data?.list) ? result.data.list : [];
    const total = result.data?.total == null ? Number.NaN : Number(result.data.total);
    const completeSnapshot = body.page === 0 && (
      (Number.isFinite(total) && total >= 0 && records.length >= total)
      || (!Number.isFinite(total) && records.length < body.pageSize)
    );
    reconcileRemoteDirectoryCache(body.parentId, records, { complete: completeSnapshot });
    // UI 刚从上游读取了这个目录：完整快照直接覆写挂载端缓存，分页快照只
    // 标脏。不再使用 invalidate——读操作递增 generation 会打断在途的
    // PROPFIND 分页加载并把该目录踢出后台预热队列。
    if (completeSnapshot && !body.resType) webDavDirectoryCache.overwriteSnapshot(body.parentId, records);
    else webDavDirectoryCache.markStale(body.parentId);
    return json(response, 200, result, { 'cache-control': 'no-store' });
  }
  if (request.method === 'GET' && url.pathname === '/api/files/detail') {
    const fileId = validateIdentifier(url.searchParams.get('fileId'), '文件 ID');
    return json(response, 200, await apiFileReadWithDeveloperFallback('/userres/v1/file/get_file_detail', { fileId }));
  }
  if (request.method === 'GET' && url.pathname === '/api/recent') {
    const body = {
      cursor: String(url.searchParams.get('cursor') || ''),
      pageSize: queryInteger(url, 'pageSize', 50, { minimum: 1, maximum: 1000 }),
    };
    const fileTypes = queryIntegerList(url, 'fileTypes', 'file_types', { maximum: 11 });
    const excludeFileTypes = queryIntegerList(url, 'excludeFileTypes', 'exclude_file_types', { maximum: 11 });
    if (fileTypes.length) body.fileTypes = fileTypes;
    if (excludeFileTypes.length) body.excludeFileTypes = excludeFileTypes;
    return json(response, 200, await apiPost('/userres/v1/get_user_action', body));
  }
  if (request.method === 'GET' && url.pathname === '/api/recycle') {
    return json(response, 200, await apiPost('/userres/v1/file/get_file_list', {
      page: queryInteger(url, 'page', 0, { minimum: 0 }),
      pageSize: queryInteger(url, 'pageSize', 100, { minimum: 1, maximum: 1000 }),
      parentId: '',
      dirType: 4,
      orderBy: 12,
      sortType: 1,
    }));
  }
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
  if (request.method === 'POST' && url.pathname === '/api/uploads/cancel') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    return json(response, 200, await cancelUploadTask(body.file_path ?? body.filePath, body.mapping_id ?? body.mappingId));
  }
  if (request.method === 'POST' && url.pathname === '/api/uploads/pause') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    return json(response, 200, await pauseUploadTask(body.file_path ?? body.filePath, body.mapping_id ?? body.mappingId));
  }
  if (request.method === 'POST' && url.pathname === '/api/uploads/resume') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    return json(response, 200, await resumeUploadTask(body.file_path ?? body.filePath, body.mapping_id ?? body.mappingId));
  }
  if (request.method === 'POST' && url.pathname === '/api/uploads/retry') {
    const body = await readBody(request, { maxBytes: 16 * 1024 });
    return json(response, 200, await retryUploadTask(body.file_path ?? body.filePath, body.mapping_id ?? body.mappingId));
  }
  if (request.method === 'POST' && url.pathname === '/api/files/create-folder') {
    const body = await readBody(request);
    const payload = {
      parentId: validateOptionalIdentifier(body.parent_id ?? body.parentId, '父目录 ID'),
      dirName: validateCloudName(body.dir_name ?? body.dirName, '文件夹名称'),
    };
    const failIfNameExist = body.fail_if_name_exist ?? body.failIfNameExist;
    if (failIfNameExist != null) {
      if (typeof failIfNameExist !== 'boolean') throw new Error('fail_if_name_exist 必须是布尔值');
      payload.failIfNameExist = failIfNameExist;
    }
    return json(response, 200, await executeFileTask('/userres/v1/file/create_dir', payload, {
      parentIds: [payload.parentId],
    }));
  }
  if (request.method === 'POST' && url.pathname === '/api/files/copy') {
    const body = await readBody(request);
    const parentId = validateOptionalIdentifier(body.parent_id, '父目录 ID');
    return json(response, 200, await executeFileTask('/userres/v1/file/copy_file', {
      fileIds: validateFileIds(body.file_ids),
      parentId,
    }, { parentIds: [parentId] }));
  }
  if (request.method === 'POST' && url.pathname === '/api/files/move') {
    const body = await readBody(request);
    const fileIds = validateFileIds(body.file_ids);
    const parentId = validateOptionalIdentifier(body.parent_id, '父目录 ID');
    return json(response, 200, await executeFileTask('/userres/v1/file/move_file', {
      fileIds,
      parentId,
    }, { parentIds: [parentId], entryIds: fileIds }));
  }
  if (request.method === 'POST' && url.pathname === '/api/files/delete') {
    const body = await readBody(request);
    const fileIds = validateFileIds(body.file_ids);
    const result = await executeFileTask('/userres/v1/file/delete_file', { fileIds }, { entryIds: fileIds });
    publishRecycleBinChanged('web-delete');
    return json(response, 200, result);
  }
  if (request.method === 'POST' && url.pathname === '/api/recycle/restore') {
    const body = await readBody(request);
    const result = await executeFileTask('/userres/v1/file/recycle_file', { fileIds: validateFileIds(body.file_ids) });
    publishRecycleBinChanged('web-restore');
    return json(response, 200, result);
  }
  if (request.method === 'POST' && url.pathname === '/api/recycle/delete') {
    const body = await readBody(request);
    // 彻底删除只影响回收站视图，普通目录缓存无需失效。
    const result = await apiPost('/userres/v1/file/delete_file', { fileIds: validateFileIds(body.file_ids) });
    await waitOperation(result.data?.taskId);
    publishRecycleBinChanged('web-permanent-delete');
    return json(response, 200, result.data || {});
  }
  if (request.method === 'POST' && url.pathname === '/api/recycle/clear') {
    const body = await readBody(request);
    const forceRetry = body.force_retry ?? body.forceRetry ?? false;
    if (typeof forceRetry !== 'boolean') throw new Error('force_retry 必须是布尔值');
    return json(response, 200, await recycleClearTask.clearRecycleBin({ forceRetry }), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/files/rename-batch') { const body = await readBody(request); return json(response, 200, await batchRename(body.renames)); }
  if (request.method === 'POST' && url.pathname === '/api/files/export-gcid') {
    const body = await readBody(request, { maxBytes: 64 * 1024 });
    return json(response, 200, await exportGcidJson(body), { 'cache-control': 'no-store' });
  }
  if (request.method === 'GET' && url.pathname === '/api/files/export-gcid-log') {
    return json(response, 200, readGcidExportDiagnosticLog(gcidExportDiagnosticFile), { 'cache-control': 'no-store' });
  }
  if (request.method === 'POST' && url.pathname === '/api/files/download') { const body = await readBody(request); return json(response, 200, await getCloudDownload(body)); }
  if (request.method === 'POST' && url.pathname === '/api/share') { const body = await readBody(request); return json(response, 200, await createManualShare(body)); }
  if (request.method === 'GET' && url.pathname === '/api/shares') return json(response, 200, await listAllShares());
  if (request.method === 'POST' && url.pathname === '/api/shares/delete') {
    const body = await readBody(request);
    const ids = validateShareIds(Array.isArray(body.ids) ? body.ids : body.share_ids);
    const result = await apiPost('/userres/v1/delete_share', { ids });
    return json(response, 200, result.data || {});
  }
  if (request.method === 'POST' && url.pathname === '/api/shares/update') {
    const body = await readBody(request);
    const validateDuration = validateInteger(body.validate_duration ?? body.validateDuration, '分享有效期', { minimum: 0 });
    if (![0, 86_400, 604_800, 2_592_000].includes(validateDuration)) throw new Error('分享有效期必须是 0、86400、604800 或 2592000');
    const result = await apiPost('/userres/v1/update_share', {
      id: validateIdentifier(body.id, '分享记录 ID'),
      validateDuration,
      downloadType: validateInteger(body.download_type ?? body.downloadType, '下载类型', { minimum: 0, maximum: 1 }),
      trafficLimit: String(validateInteger(body.traffic_limit ?? body.trafficLimit, '流量限制', { minimum: 0, maximum: maxShareTrafficBytes })),
    });
    return json(response, 200, result.data || {});
  }
  if (request.method === 'POST' && url.pathname === '/api/shares/delete-invalid') {
    await readBody(request);
    const result = await apiPost('/userres/v1/delete_invalid_share', {});
    return json(response, 200, result.data || {});
  }
  if (request.method === 'POST' && ['/api/direct-link/set', '/api/direct-link/unset'].includes(url.pathname)) {
    const body = await readBody(request);
    const endpoint = url.pathname.endsWith('/set') ? '/userres/v1/set_direct_link' : '/userres/v1/unset_direct_link';
    const result = await apiPost(endpoint, { fileId: validateIdentifier(body.file_id ?? body.fileId, '文件 ID') });
    return json(response, 200, result.data || {});
  }
  if (request.method === 'POST' && url.pathname === '/api/direct-link/get') {
    const body = await readBody(request);
    const shortLink = body.short_link ?? body.shortLink ?? false;
    if (typeof shortLink !== 'boolean') throw new Error('short_link 必须是布尔值');
    const result = await apiPost('/userres/v1/get_direct_link', {
      fileId: validateIdentifier(body.file_id ?? body.fileId, '文件 ID'),
      shortLink,
    });
    return json(response, 200, result.data || {});
  }
  if (request.method === 'POST' && url.pathname === '/api/received-share/open') { const body = await readBody(request); return json(response, 200, await openReceivedShare(body.url)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/files') { const body = await readBody(request); return json(response, 200, await listReceivedShareFiles(body.access_token, body.parent_id)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/restore') { const body = await readBody(request); return json(response, 200, await restoreReceivedShare(body)); }
  if (request.method === 'POST' && url.pathname === '/api/received-share/download') { const body = await readBody(request); return json(response, 200, await getReceivedShareDownload(body)); }
  if (request.method === 'GET' && url.pathname === '/api/offline') {
    const pageSize = queryInteger(url, 'pageSize', 100, { minimum: 1, maximum: 1000 });
    const page = queryInteger(url, 'page', 0, { minimum: 0 });
    const cursor = String(url.searchParams.get('cursor') || '');
    if (!cursor && page > 0) throw new Error('离线任务列表使用 cursor 翻页，不支持 page > 0');
    const body = { cursor, pageSize };
    const status = queryIntegerList(url, 'status', 'statuses', { maximum: 5 });
    if (status.length) body.status = status;
    const result = await apiPost('/cloudcollection/v1/list_task', body);
    result.data = await reconcileOfflineNameRestores(result.data || {});
    return json(response, 200, result);
  }
  if (request.method === 'POST' && url.pathname === '/api/offline/resolve') {
    const body = await readBody(request);
    const source = validateOfflineUrl(body.url);
    const result = await apiPost('/cloudcollection/v1/resolve_res', { url: source });
    return json(response, 200, result.data || {});
  }
  if (request.method === 'POST' && url.pathname === '/api/offline') {
    const body = await readBody(request);
    const source = validateOfflineUrl(body.url);
    const fileIndexes = validateOfflineFileIndexes(body.file_indexes ?? body.fileIndexes, source);
    const payload = {
      url: source,
      parentId: validateOptionalIdentifier(body.parent_id ?? body.parentId, '父目录 ID'),
    };
    if (fileIndexes.length) payload.fileIndexes = fileIndexes;
    const rawNewName = body.new_name ?? body.newName;
    const requestedName = rawNewName != null && String(rawNewName).trim()
      ? validateCloudName(rawNewName, '离线任务名称')
      : '';
    const shouldObfuscate = offlineFilenameObfuscationEnabled && /^(?:magnet:|ed2k:\/\/)/i.test(source);
    let originalName = requestedName;
    let temporaryName = '';
    if (shouldObfuscate) {
      const suppliedRestoreName = body.restore_name ?? body.restoreName;
      if (!originalName && suppliedRestoreName != null && String(suppliedRestoreName).trim()) {
        originalName = validateCloudName(suppliedRestoreName, '待恢复文件名');
      }
      if (!originalName) originalName = offlineSourceName(source);
      if (!originalName) {
        const resolved = await apiPost('/cloudcollection/v1/resolve_res', { url: source });
        originalName = offlineResolvedName(resolved);
      }
      originalName = validateCloudName(originalName, '待恢复文件名');
      temporaryName = offlineTemporaryName(originalName, source);
      payload.url = protectedOfflineSource(source, temporaryName);
      payload.newName = temporaryName;
    } else if (requestedName) {
      payload.newName = requestedName;
    }
    const result = await apiPost('/cloudcollection/v1/create_task', payload);
    if (shouldObfuscate) {
      const taskId = String(result.data?.taskId || result.data?.id || '').trim();
      if (!taskId) throw new Error('离线任务已提交，但光鸭没有返回 taskId，无法自动恢复文件名');
      saveOfflineNameRestore(taskId, originalName, temporaryName);
      result.data = {
        ...(result.data || {}),
        nameRestoreStatus: 'pending',
        originalName,
      };
      void reconcileOfflineNameRestores().catch(() => {});
    }
    return json(response, 200, result);
  }
  if (request.method === 'POST' && ['/api/offline/cancel', '/api/offline/delete'].includes(url.pathname)) {
    const body = await readBody(request);
    const taskIds = validateTaskIds(body.task_ids ?? body.taskIds);
    const result = await apiPost('/cloudcollection/v2/delete_task', { taskIds });
    removeOfflineNameRestores(taskIds);
    return json(response, 200, result.data || {});
  }
  if (request.method === 'POST' && url.pathname === '/api/offline/retry') {
    const body = await readBody(request);
    const result = await apiPost('/cloudcollection/v2/retry_task', { taskIds: validateTaskIds(body.task_ids ?? body.taskIds) });
    return json(response, 200, result.data || {});
  }
  if (request.method === 'GET' && url.pathname === '/api/offline/statistics') {
    return json(response, 200, await apiPost('/nd.bizcloudcollection.s/v1/get_task_statistics', {}));
  }
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
  if (request.method === 'POST' && url.pathname === '/api/mappings') {
    const body = await readBody(request);
    const localPath = allowedPath(body.local_path);
    const sourcePolicy = ['keep', 'archive', 'delete'].includes(body.source_policy) ? body.source_policy : 'keep';
    const archivePath = sourcePolicy === 'archive' ? allowedArchivePath(body.archive_path || archiveRoot) : null;
    if (archivePath && (archivePath === localPath || archivePath.startsWith(`${localPath}${path.sep}`))) throw new Error('归档目录不能位于被监控目录内部');
    if (body.auto_share && (!hdhiveBaseUrl || !hdhiveSecret)) throw new Error('开启自动分享前请先配置 Hdhive 地址和密钥');
    const organizerMappingId = String(body.organizer_mapping_id || '').trim();
    const remoteParentId = String(body.remote_parent_id || '');
    if (organizerMappingId) {
      const organizerMapping = organizer.state().mappings.find((item) => item.id === organizerMappingId && item.enabled);
      if (!organizerMapping) throw new Error('选择的上传后整理任务不存在或未启用');
      if (organizerMapping.source_dir_id !== remoteParentId) throw new Error('上传目标目录必须与所选整理任务的 A 目录完全一致');
    }
    const mapping = {
      id: crypto.randomUUID(), local_path: localPath, remote_path: normalizeRemote(body.remote_path), remote_parent_id: remoteParentId,
      enabled: true, source_policy: sourcePolicy, archive_path: archivePath, scan_existing: body.scan_existing !== false,
      sync_types: normalizeSyncTypes(body.sync_types), monitor_mode: normalizeMonitorMode(body.monitor_mode),
      auto_share: body.auto_share === true, organizer_mapping_id: organizerMappingId, watch_error: null,
    };
    const stat = await fsp.stat(mapping.local_path);
    if (!stat.isDirectory()) throw new Error('监控路径不是目录');
    mappings.push(mapping);
    await fsp.mkdir(archiveRoot, { recursive: true });
    await saveConfig();
    try { await startWatcher(mapping); }
    catch (error) { mappings = mappings.filter((item) => item.id !== mapping.id); await saveConfig(); throw new Error(`创建目录监控失败：${error.message}`); }
    publishState();
    return json(response, 200, mapping);
  }
  if (request.method === 'DELETE' && url.pathname.startsWith('/api/mappings/')) { const id = decodeURIComponent(url.pathname.split('/').pop()); await watchers.get(id)?.close(); watchers.delete(id); mappings = mappings.filter((item) => item.id !== id); for (const [key, item] of queue) if (item.mapping_id === id) queue.delete(key); for (const [key, item] of waitingFiles) if (item.mapping_id === id) waitingFiles.delete(key); for (const key of flashPreflightCache.keys()) if (key.startsWith(`${id}::`)) flashPreflightCache.delete(key); for (const key of history.keys()) if (key.startsWith(`${id}::`)) history.delete(key); for (const key of inflight.keys()) if (key.startsWith(`${id}::`)) inflight.delete(key); for (const key of pausedUploads) if (key.startsWith(`${id}::`)) pausedUploads.delete(key); for (const key of queuePauseRequests) if (key.startsWith(`${id}::`)) queuePauseRequests.delete(key); deleteMappingTransientUploads(id); await saveConfig(); publishState(); return json(response, 200, {}); }
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
    if (body.organizer_mapping_id !== undefined) {
      const organizerMappingId = String(body.organizer_mapping_id || '').trim();
      if (organizerMappingId) {
        const organizerMapping = organizer.state().mappings.find((item) => item.id === organizerMappingId && item.enabled);
        if (!organizerMapping) throw new Error('选择的上传后整理任务不存在或未启用');
        if (organizerMapping.source_dir_id !== String(mapping.remote_parent_id || '')) throw new Error('上传目标目录必须与所选整理任务的 A 目录完全一致');
      }
      mapping.organizer_mapping_id = organizerMappingId;
    }
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
  if (request.method === 'POST' && url.pathname === '/api/queue/pause') {
    paused = true;
    for (const key of inflightItems.keys()) queuePauseRequests.add(key);
    for (const client of activeUploadClients.values()) {
      try { client.cancel(); } catch {}
    }
    publishState();
    return json(response, 200, state());
  }
  if (request.method === 'POST' && url.pathname === '/api/queue/resume') { paused = false; pump(); return json(response, 200, state()); }
  json(response, 404, { error: 'not found' });
}
async function serveStatic(response, url) { const requested = url.pathname === '/' ? '/index.html' : url.pathname; const file = path.resolve(uiRoot, `.${requested}`); if (!file.startsWith(uiRoot + path.sep)) return json(response, 403, { error: 'forbidden' }); try { const content = await fsp.readFile(file); const type = file.endsWith('.html') ? 'text/html; charset=utf-8' : file.endsWith('.js') ? 'text/javascript; charset=utf-8' : file.endsWith('.css') ? 'text/css; charset=utf-8' : file.endsWith('.svg') ? 'image/svg+xml' : 'application/octet-stream'; response.writeHead(200, { 'content-type': type }); response.end(content); } catch { json(response, 404, { error: 'not found' }); } }

await fsp.mkdir(dataDir, { recursive: true }); await fsp.mkdir(manualUploadRoot, { recursive: true }); await cleanupUnreferencedManualUploads(); await fsp.mkdir(watchRoot, { recursive: true }); await fsp.mkdir(archiveRoot, { recursive: true });
try { const config = JSON.parse(await fsp.readFile(configFile, 'utf8')); mappings = Array.isArray(config.mappings) ? config.mappings.map((item) => ({ source_policy: 'keep', archive_path: null, scan_existing: true, remote_parent_id: '', sync_types: DEFAULT_SYNC_TYPES, monitor_mode: 'native', auto_share: false, organizer_mapping_id: '', watch_error: null, ...item, local_path: allowedPath(item.local_path), archive_path: item.archive_path ? allowedArchivePath(item.archive_path) : null, sync_types: normalizeSyncTypes(item.sync_types), monitor_mode: normalizeMonitorMode(item.monitor_mode), auto_share: item.auto_share === true, organizer_mapping_id: String(item.organizer_mapping_id || '') })) : []; savedShares = Array.isArray(config.saved_shares) ? config.saved_shares : []; } catch { mappings = []; savedShares = []; }
restoreUploadCheckpoints();
await restartWatchers();
await organizer.initialize();
virtualLibrary.start();
restorePendingAutoShares();
resumeHdhiveReceiptPolling();
resumeDeveloperTransfers();
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
server.requestTimeout = Math.max(requestTimeoutMs, ossTimeoutMs);
server.headersTimeout = Math.min(requestTimeoutMs, 15_000);

const webdavServer = http.createServer(async (request, response) => {
  const url = new URL(request.url, `http://${request.headers.host || 'localhost'}`);
  try {
    if (!webdavAccessControl.required()) {
      response.writeHead(503, {
        'content-type': 'text/plain; charset=utf-8',
        'cache-control': 'no-store',
        'retry-after': '60',
      });
      response.end('请先在光鸭设置页配置独立的 WebDAV 账号和密码');
      return;
    }
    const authorization = await webdavAccessControl.authenticate(request);
    if (!authorization.ok) return webdavAccessControl.reject(response, authorization);
    if (url.pathname === '/dav' || url.pathname.startsWith('/dav/')) {
      await handleWebDav(request, response, url);
      return;
    }
    response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store' });
    response.end('not found');
  } catch (error) {
    const statusCode = Number.isInteger(error.statusCode) ? error.statusCode : 400;
    response.writeHead(statusCode, {
      'content-type': 'text/plain; charset=utf-8',
      'cache-control': 'no-store',
      ...(error.headers || {}),
    });
    response.end(error.message);
  }
});
webdavServer.requestTimeout = Math.max(requestTimeoutMs, ossTimeoutMs);
webdavServer.headersTimeout = Math.min(requestTimeoutMs, 15_000);

const embyProxyServer = http.createServer(async (request, response) => {
  const url = new URL(request.url, `http://${request.headers.host || 'localhost'}`);
  try {
    await virtualLibrary.handleProxy(request, response, url);
  } catch (error) {
    response.writeHead(502, { 'content-type': 'text/plain; charset=utf-8', 'cache-control': 'no-store' });
    response.end(`Emby 代理请求失败：${error.message}`);
  }
});
embyProxyServer.requestTimeout = 0;
embyProxyServer.headersTimeout = Math.min(requestTimeoutMs, 15_000);
embyProxyServer.on('upgrade', (request, socket, head) => virtualLibrary.proxyUpgrade(request, socket, head));
embyProxyServer.once('error', (error) => {
  virtualLibrary.setProxyStatus({ running: false, error: error.message });
  console.error(`Guangya Emby proxy failed: ${error.message}`);
});
embyProxyServer.listen(embyProxyPort, embyProxyHost, () => {
  const actualPort = Number(embyProxyServer.address()?.port || embyProxyPort);
  const publicPort = embyProxyPublicPort || actualPort;
  virtualLibrary.setProxyStatus({ running: true, error: null, port: publicPort });
  const displayHost = embyProxyHost.includes(':') ? `[${embyProxyHost}]` : embyProxyHost;
  console.log(`Guangya Emby proxy listening on http://${displayHost}:${actualPort}/ -> ${virtualLibrary.info().emby_upstream}`);
});

webdavServer.listen(webdavPort, webdavHost, () => {
  const displayHost = webdavHost.includes(':') ? `[${webdavHost}]` : webdavHost;
  console.log(`Guangya WebDAV listening on http://${displayHost}:${webdavPort}/dav/, auth: ${webdavAccessControl.required() ? `enabled (${webdavUsername})` : 'not configured'}`);
});

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
    nativeMountManager.shutdown();
    virtualLibrary.close();
    await new Promise((resolve) => server.close(resolve));
    await new Promise((resolve) => webdavServer.close(resolve));
    await new Promise((resolve) => embyProxyServer.close(resolve));
    for (const watcher of watchers.values()) await watcher.close();
    await organizer.close();
    process.exit(0);
  }
});

process.once('exit', () => {
  clearInterval(webDavDirectoryRefreshTimer);
  webDavDirectoryCache.dispose();
  nativeMountManager.shutdown();
  virtualLibrary.close();
});

setInterval(() => {
  if (!refreshToken) return;
  void refreshSavedSession().catch((error) => {
    status('warning', `自动续期失败，将稍后重试：${error.message}`);
  });
}, tokenRefreshIntervalMs);

setInterval(() => {
  if (!token || !pendingOfflineNameRestoreCount()) return;
  void reconcileOfflineNameRestores().catch(() => {});
}, 5_000);
