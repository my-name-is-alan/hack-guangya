import crypto from 'node:crypto';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import http from 'node:http';
import https from 'node:https';
import net from 'node:net';
import path from 'node:path';
import tls from 'node:tls';
import { fetch as undiciFetch } from 'undici';

const VIDEO_EXTENSIONS = new Set(['3gp', 'asf', 'avi', 'flv', 'm2ts', 'm4v', 'mkv', 'mov', 'mp4', 'mpeg', 'mpg', 'mts', 'rm', 'rmvb', 'ts', 'vob', 'webm', 'wmv']);
const AUDIO_EXTENSIONS = new Set(['aac', 'ac3', 'aiff', 'alac', 'ape', 'dff', 'dsf', 'dts', 'flac', 'm4a', 'mp3', 'ogg', 'opus', 'wav', 'wma']);
const METADATA_EXTENSIONS = new Set(['ass', 'cue', 'gif', 'jpeg', 'jpg', 'lrc', 'nfo', 'png', 'srt', 'ssa', 'sub', 'sup', 'vtt', 'webp', 'xml']);
const MANIFEST_NAME = '.guangya-virtual-library.json';
const DEFAULT_EMBY_UPSTREAM = 'http://127.0.0.1:8096';
const MAX_ITEMS = 100_000;
const MAX_DEPTH = 64;
const MAX_METADATA_BYTES = 64 * 1024 * 1024;
const HOP_BY_HOP_HEADERS = new Set(['connection', 'keep-alive', 'proxy-authenticate', 'proxy-authorization', 'proxy-connection', 'te', 'trailer', 'transfer-encoding', 'upgrade']);

function cleanText(value) { return String(value ?? '').trim(); }
function extension(name) { return path.extname(cleanText(name)).slice(1).toLowerCase(); }
export function virtualFileKind(name) {
  const suffix = extension(name);
  if (VIDEO_EXTENSIONS.has(suffix) || AUDIO_EXTENSIONS.has(suffix)) return 'strm';
  if (METADATA_EXTENSIONS.has(suffix)) return 'metadata';
  return '';
}
function isWithin(root, target) { const relative = path.relative(root, target); return relative === '' || (!relative.startsWith('..') && !path.isAbsolute(relative)); }
function safeComponent(value) {
  let output = cleanText(value).replace(/[<>:"/\\|?*\u0000-\u001f\u007f]/g, '_').replace(/[. ]+$/g, '').trim();
  if (!output || output === '.' || output === '..') output = '未命名';
  const stem = output.split('.')[0].toUpperCase();
  if (/^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$/.test(stem)) output = `_${output}`;
  return output;
}
function virtualComponent(value) { return cleanText(value).replace(/[\\/\u0000-\u001f\u007f]/g, '_') || '未命名'; }
export function strmFileName(name) {
  const safe = safeComponent(name);
  const suffix = path.extname(safe);
  return `${suffix ? safe.slice(0, -suffix.length) : safe}.strm`;
}
export function virtualMediaPath(sourcePath, segments, name) {
  const prefix = cleanText(sourcePath).replaceAll('\\', '/').replace(/\/+$/, '');
  const suffix = [...segments, name].map(virtualComponent).join('/');
  return `${prefix.startsWith('/') ? prefix : `/${prefix}`}/${suffix}`.replace(/\/{2,}/g, '/');
}
export function strmContent(virtualPath) { return `${cleanText(virtualPath)}\n`; }
function normalizedVirtualPath(value) { return cleanText(value).replaceAll('\\', '/').replace(/\/{2,}/g, '/').replace(/\/$/, '').toLowerCase(); }
function normalizeRemoteEntry(value) {
  const id = cleanText(value?.fileId || value?.id);
  const name = cleanText(value?.fileName || value?.name);
  if (!id || !name) return null;
  return {
    id,
    name,
    isDirectory: Number(value?.resType ?? value?.type) === 2 || value?.isDirectory === true,
    size: Math.max(0, Number(value?.fileSize ?? value?.size ?? 0) || 0),
    modifiedMs: Math.max(0, Number(value?.utime ?? value?.updatedAt ?? value?.modifiedAt ?? value?.mtime ?? value?.ctime ?? 0) || 0),
  };
}
function normalizeRefreshMinutes(value) {
  const number = Math.round(Number(value));
  if (!Number.isInteger(number) || number < 1 || number > 1440) throw new Error('虚拟库刷新间隔必须为 1 到 1440 分钟');
  return number;
}
export function normalizeEmbyUpstream(value) {
  const raw = cleanText(value) || DEFAULT_EMBY_UPSTREAM;
  let parsed;
  try { parsed = new URL(raw); } catch { throw new Error('Emby 原始服务地址无效'); }
  if (!['http:', 'https:'].includes(parsed.protocol) || parsed.username || parsed.password || parsed.search || parsed.hash) throw new Error('Emby 原始服务地址必须是无账号和查询参数的 HTTP(S) 地址');
  if (parsed.pathname !== '/' && parsed.pathname !== '') throw new Error('Emby 原始服务地址不要包含路径，例如 http://127.0.0.1:8096');
  return parsed.origin;
}
function normalizedTarget(root, value) {
  const target = path.resolve(cleanText(value) || root);
  if (!isWithin(root, target)) throw new Error(`Docker/Web 虚拟库目录必须位于 ${root}`);
  return target;
}
function normalizeMapping(root, input = {}) {
  const sourceDirId = cleanText(input.source_dir_id || input.sourceDirId);
  if (!/^[A-Za-z0-9._:-]{1,256}$/.test(sourceDirId)) throw new Error('虚拟库云端目录 ID 无效');
  const sourcePath = cleanText(input.source_path || input.sourcePath);
  if (!sourcePath) throw new Error('虚拟库云端目录路径不能为空');
  const localPath = normalizedTarget(root, input.local_path || input.localPath);
  const name = cleanText(input.name) || sourcePath.split('/').filter(Boolean).at(-1) || '虚拟库';
  if (name.length > 80) throw new Error('虚拟库名称不能超过 80 个字符');
  return {
    id: cleanText(input.id) || crypto.randomUUID(),
    name,
    source_dir_id: sourceDirId,
    source_path: sourcePath,
    local_path: localPath,
    include_metadata: input.include_metadata === true || input.includeMetadata === true,
    enabled: input.enabled !== false,
  };
}
async function readManifest(root) {
  try { return JSON.parse(await fsp.readFile(path.join(root, MANIFEST_NAME), 'utf8')); }
  catch { return { version: 2, source_dir_id: '', entries: {} }; }
}
async function writeManifest(root, manifest) { await fsp.writeFile(path.join(root, MANIFEST_NAME), JSON.stringify(manifest, null, 2)); }
function playbackItemId(pathname) {
  const match = pathname.match(/^\/(?:emby\/)?(?:videos|audio)\/([^/]+)\/(?:stream(?:\.[^/]+)?|original(?:\.[^/]+)?)$/i)
    || pathname.match(/^\/(?:emby\/)?items\/([^/]+)\/file$/i);
  if (!match) return '';
  try { return decodeURIComponent(match[1]); } catch { return ''; }
}
function copyAuthHeaders(headers) {
  const copied = {};
  for (const name of ['authorization', 'x-emby-authorization', 'x-emby-token', 'user-agent']) {
    if (headers[name]) copied[name] = headers[name];
  }
  return copied;
}

export function createVirtualLibraryService({
  database,
  cloud,
  root,
  proxyPort = 18096,
  embyUpstream = DEFAULT_EMBY_UPSTREAM,
  fetchImpl = undiciFetch,
  publish = () => {},
}) {
  const virtualRoot = path.resolve(root);
  let currentProxyPort = Number(proxyPort) || 0;
  let proxyRunning = false;
  let proxyError = null;
  fs.mkdirSync(virtualRoot, { recursive: true });
  database.exec(`
    CREATE TABLE IF NOT EXISTS virtual_library_settings (
      id INTEGER PRIMARY KEY CHECK (id = 1),
      refresh_minutes INTEGER NOT NULL DEFAULT 15,
      emby_upstream TEXT NOT NULL DEFAULT '${DEFAULT_EMBY_UPSTREAM}',
      updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS virtual_library_mappings (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      source_dir_id TEXT NOT NULL,
      source_path TEXT NOT NULL,
      local_path TEXT NOT NULL,
      include_metadata INTEGER NOT NULL DEFAULT 0,
      enabled INTEGER NOT NULL DEFAULT 1,
      updated_at INTEGER NOT NULL
    );
  `);
  const settingsColumns = database.prepare('PRAGMA table_info(virtual_library_settings)').all().map((column) => column.name);
  const addedEmbyUpstream = !settingsColumns.includes('emby_upstream');
  if (addedEmbyUpstream) database.exec(`ALTER TABLE virtual_library_settings ADD COLUMN emby_upstream TEXT NOT NULL DEFAULT '${DEFAULT_EMBY_UPSTREAM}'`);
  database.prepare('INSERT OR IGNORE INTO virtual_library_settings (id, refresh_minutes, emby_upstream, updated_at) VALUES (1, 15, ?, ?)').run(normalizeEmbyUpstream(embyUpstream), Math.floor(Date.now() / 1000));
  if (addedEmbyUpstream || !cleanText(database.prepare('SELECT emby_upstream FROM virtual_library_settings WHERE id = 1').get()?.emby_upstream)) {
    database.prepare('UPDATE virtual_library_settings SET emby_upstream = ? WHERE id = 1').run(normalizeEmbyUpstream(embyUpstream));
  }
  const statuses = new Map();

  function mappings() {
    return database.prepare('SELECT id, name, source_dir_id, source_path, local_path, include_metadata, enabled FROM virtual_library_mappings ORDER BY updated_at DESC').all().map((row) => ({
      ...row,
      include_metadata: Boolean(row.include_metadata),
      enabled: Boolean(row.enabled),
    }));
  }
  function settings() {
    const row = database.prepare('SELECT refresh_minutes, emby_upstream FROM virtual_library_settings WHERE id = 1').get();
    return { refreshMinutes: Number(row?.refresh_minutes || 15), embyUpstream: normalizeEmbyUpstream(row?.emby_upstream || embyUpstream) };
  }
  function info() {
    const current = settings();
    return {
      proxy_endpoint: `http://127.0.0.1:${currentProxyPort}/`,
      proxy_port: currentProxyPort,
      proxy_running: proxyRunning,
      proxy_error: proxyError,
      emby_upstream: current.embyUpstream,
      refresh_minutes: current.refreshMinutes,
      virtual_root: virtualRoot,
      mappings: mappings(),
      statuses: Object.fromEntries(statuses),
    };
  }
  function emitInfo() { publish({ type: 'virtual-library', data: info() }); }
  function setProxyStatus({ running, error = null, port } = {}) {
    if (typeof running === 'boolean') proxyRunning = running;
    if (Number.isInteger(Number(port)) && Number(port) >= 0 && Number(port) <= 65535) currentProxyPort = Number(port);
    proxyError = error ? cleanText(error) : null;
    emitInfo();
    return info();
  }
  function updateSettings(input = {}) {
    const current = settings();
    const refreshMinutes = normalizeRefreshMinutes(input.refresh_minutes ?? input.refreshMinutes ?? current.refreshMinutes);
    const upstream = normalizeEmbyUpstream(input.emby_upstream ?? input.embyUpstream ?? current.embyUpstream);
    database.prepare('UPDATE virtual_library_settings SET refresh_minutes = ?, emby_upstream = ?, updated_at = ? WHERE id = 1').run(refreshMinutes, upstream, Math.floor(Date.now() / 1000));
    emitInfo();
    return info();
  }
  function upsert(input = {}) {
    const mapping = normalizeMapping(virtualRoot, input.mapping || input);
    const currentMappings = mappings();
    if (!currentMappings.some((item) => item.id === mapping.id) && currentMappings.length >= 32) throw new Error('虚拟库最多配置 32 个目录');
    if (currentMappings.some((item) => item.id !== mapping.id
      && (isWithin(item.local_path, mapping.local_path) || isWithin(mapping.local_path, item.local_path)))) {
      throw new Error('虚拟库本地目录不能与其他配置相同或互相包含');
    }
    database.prepare(`INSERT INTO virtual_library_mappings
      (id, name, source_dir_id, source_path, local_path, include_metadata, enabled, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET name=excluded.name, source_dir_id=excluded.source_dir_id,
      source_path=excluded.source_path, local_path=excluded.local_path,
      include_metadata=excluded.include_metadata, enabled=excluded.enabled, updated_at=excluded.updated_at`)
      .run(mapping.id, mapping.name, mapping.source_dir_id, mapping.source_path, mapping.local_path, Number(mapping.include_metadata), Number(mapping.enabled), Math.floor(Date.now() / 1000));
    emitInfo();
    return info();
  }
  function remove(id) {
    if (!database.prepare('DELETE FROM virtual_library_mappings WHERE id = ?').run(cleanText(id)).changes) throw new Error('虚拟库配置不存在');
    statuses.delete(cleanText(id));
    emitInfo();
    return info();
  }
  async function writeMetadata(entry, target) {
    if (entry.size > MAX_METADATA_BYTES) throw new Error(`元数据文件超过 64 MB：${entry.name}`);
    const url = await cloud.getDownloadUrl(entry.id);
    const response = await fetchImpl(url);
    if (!response.ok) throw new Error(`下载元数据失败（${entry.name}）：HTTP ${response.status}`);
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length > MAX_METADATA_BYTES) throw new Error(`元数据文件超过 64 MB：${entry.name}`);
    await fsp.mkdir(path.dirname(target), { recursive: true });
    await fsp.writeFile(target, bytes);
  }
  async function syncInner(mapping) {
    const targetRoot = normalizedTarget(virtualRoot, mapping.local_path);
    await fsp.mkdir(targetRoot, { recursive: true });
    const previous = await readManifest(targetRoot);
    const next = { version: 2, source_dir_id: mapping.source_dir_id, entries: {} };
    const queue = [{ parentId: mapping.source_dir_id, relative: '', virtualSegments: [], depth: 0 }];
    const outputs = new Set();
    const summary = { strm_files: 0, metadata_files: 0, skipped_files: 0 };
    let scanned = 0;
    while (queue.length) {
      const current = queue.shift();
      if (current.depth > MAX_DEPTH) throw new Error(`云端目录超过 ${MAX_DEPTH} 层，已停止同步`);
      const children = (await cloud.listChildren(current.parentId)).map(normalizeRemoteEntry).filter(Boolean);
      for (const entry of children) {
        scanned += 1;
        if (scanned > MAX_ITEMS) throw new Error(`单个虚拟库超过 ${MAX_ITEMS} 项，已停止同步`);
        if (entry.isDirectory) {
          const relative = path.join(current.relative, safeComponent(entry.name));
          await fsp.mkdir(path.join(targetRoot, relative), { recursive: true });
          queue.push({ parentId: entry.id, relative, virtualSegments: [...current.virtualSegments, entry.name], depth: current.depth + 1 });
          continue;
        }
        const kind = virtualFileKind(entry.name);
        if (!kind || (kind === 'metadata' && !mapping.include_metadata)) { summary.skipped_files += 1; continue; }
        const relative = path.join(current.relative, kind === 'strm' ? strmFileName(entry.name) : safeComponent(entry.name));
        const key = relative.split(path.sep).join('/');
        const collision = process.platform === 'win32' ? key.toLowerCase() : key;
        if (outputs.has(collision)) throw new Error(`多个云端文件会生成同一本地文件：${key}`);
        outputs.add(collision);
        const target = path.join(targetRoot, relative);
        const virtualPath = kind === 'strm' ? virtualMediaPath(mapping.source_path, current.virtualSegments, entry.name) : '';
        const manifestEntry = { source_id: entry.id, size: entry.size, modified_ms: entry.modifiedMs, kind, virtual_path: virtualPath };
        const old = previous.entries?.[key];
        const unchanged = old && old.source_id === manifestEntry.source_id && Number(old.size) === manifestEntry.size
          && Number(old.modified_ms) === manifestEntry.modified_ms && old.kind === kind && old.virtual_path === virtualPath && fs.existsSync(target);
        if (kind === 'strm') {
          const content = strmContent(virtualPath);
          const sameContent = unchanged && await fsp.readFile(target, 'utf8').then((value) => value === content).catch(() => false);
          if (!sameContent) { await fsp.mkdir(path.dirname(target), { recursive: true }); await fsp.writeFile(target, content); }
          summary.strm_files += 1;
        } else {
          if (!unchanged) await writeMetadata(entry, target);
          summary.metadata_files += 1;
        }
        next.entries[key] = manifestEntry;
      }
    }
    for (const key of Object.keys(previous.entries || {})) {
      if (next.entries[key]) continue;
      const candidate = path.resolve(targetRoot, key);
      if (isWithin(targetRoot, candidate)) await fsp.rm(candidate, { force: true });
    }
    await writeManifest(targetRoot, next);
    return summary;
  }
  function sync(id) {
    const mapping = mappings().find((item) => item.id === cleanText(id));
    if (!mapping) throw new Error('虚拟库配置不存在');
    if (!mapping.enabled) throw new Error('该虚拟库已停用');
    if (statuses.get(mapping.id)?.running) throw new Error('该虚拟库正在同步');
    statuses.set(mapping.id, { ...(statuses.get(mapping.id) || {}), running: true, error: null });
    emitInfo();
    void syncInner(mapping).then((summary) => {
      statuses.set(mapping.id, { running: false, last_sync_at: Math.floor(Date.now() / 1000), ...summary, error: null });
      emitInfo();
    }).catch((error) => {
      statuses.set(mapping.id, { ...(statuses.get(mapping.id) || {}), running: false, error: error.message });
      emitInfo();
    });
    return info();
  }
  async function sourceIdForVirtualPath(value) {
    const expected = normalizedVirtualPath(value);
    if (!expected) return '';
    for (const mapping of mappings().filter((item) => item.enabled)) {
      const manifest = await readManifest(mapping.local_path);
      for (const entry of Object.values(manifest.entries || {})) {
        if (entry.kind === 'strm' && normalizedVirtualPath(entry.virtual_path) === expected) return cleanText(entry.source_id);
      }
    }
    return '';
  }
  async function playbackSourceId(request, url) {
    if (!['GET', 'HEAD'].includes(request.method)) return '';
    const itemId = playbackItemId(url.pathname);
    if (!itemId) return '';
    const upstream = settings().embyUpstream;
    const prefix = /^\/emby\//i.test(url.pathname) ? '/emby' : '';
    const playbackUrl = new URL(`${prefix}/Items/${encodeURIComponent(itemId)}/PlaybackInfo`, upstream);
    for (const [name, value] of url.searchParams) {
      if (['api_key', 'x-emby-token', 'userid'].includes(name.toLowerCase())) playbackUrl.searchParams.set(name, value);
    }
    const response = await fetchImpl(playbackUrl, { headers: copyAuthHeaders(request.headers), redirect: 'manual' });
    if (!response.ok) return '';
    const payload = await response.json();
    const sources = Array.isArray(payload?.MediaSources) ? payload.MediaSources : Array.isArray(payload?.mediaSources) ? payload.mediaSources : [];
    const requestedId = cleanText(url.searchParams.get('MediaSourceId') || url.searchParams.get('mediaSourceId'));
    const ordered = requestedId
      ? [...sources.filter((source) => cleanText(source?.Id || source?.id) === requestedId), ...sources.filter((source) => cleanText(source?.Id || source?.id) !== requestedId)]
      : sources;
    for (const source of ordered) {
      const sourceId = await sourceIdForVirtualPath(source?.Path || source?.path);
      if (sourceId) return sourceId;
    }
    return '';
  }
  function proxyHttp(request, response, url) {
    return new Promise((resolve, reject) => {
      const upstream = new URL(settings().embyUpstream);
      const headers = { ...request.headers, host: upstream.host };
      for (const name of HOP_BY_HOP_HEADERS) delete headers[name];
      if (request.headers.host) headers['x-forwarded-host'] = request.headers.host;
      headers['x-forwarded-proto'] = 'http';
      const transport = upstream.protocol === 'https:' ? https : http;
      const outgoing = transport.request({
        protocol: upstream.protocol,
        hostname: upstream.hostname,
        port: upstream.port || (upstream.protocol === 'https:' ? 443 : 80),
        method: request.method,
        path: `${url.pathname}${url.search}`,
        headers,
      }, (upstreamResponse) => {
        const responseHeaders = { ...upstreamResponse.headers };
        for (const name of HOP_BY_HOP_HEADERS) delete responseHeaders[name];
        const location = cleanText(responseHeaders.location);
        if (location.startsWith(upstream.origin) && request.headers.host) responseHeaders.location = `http://${request.headers.host}${location.slice(upstream.origin.length)}`;
        response.writeHead(upstreamResponse.statusCode || 502, responseHeaders);
        upstreamResponse.pipe(response);
        upstreamResponse.once('end', resolve);
        upstreamResponse.once('error', reject);
      });
      outgoing.once('error', reject);
      request.once('aborted', () => outgoing.destroy());
      request.pipe(outgoing);
    });
  }
  async function handleProxy(request, response, url) {
    let sourceId = '';
    try { sourceId = await playbackSourceId(request, url); } catch {}
    if (sourceId) {
      const location = await cloud.getDownloadUrl(sourceId);
      response.writeHead(302, { location, 'cache-control': 'no-store', 'access-control-allow-origin': '*' });
      response.end();
      return;
    }
    await proxyHttp(request, response, url);
  }
  const upgradeSockets = new Set();
  function proxyUpgrade(request, socket, head) {
    const upstream = new URL(settings().embyUpstream);
    const connect = upstream.protocol === 'https:' ? tls.connect : net.connect;
    const upstreamSocket = connect({
      host: upstream.hostname,
      port: Number(upstream.port || (upstream.protocol === 'https:' ? 443 : 80)),
      ...(upstream.protocol === 'https:' ? { servername: upstream.hostname } : {}),
    });
    upgradeSockets.add(socket);
    upgradeSockets.add(upstreamSocket);
    upstreamSocket.once('connect', () => {
      const headers = [];
      for (let index = 0; index < request.rawHeaders.length; index += 2) {
        const name = request.rawHeaders[index];
        const value = request.rawHeaders[index + 1];
        headers.push(`${name}: ${name.toLowerCase() === 'host' ? upstream.host : value}`);
      }
      upstreamSocket.write(`${request.method} ${request.url} HTTP/${request.httpVersion}\r\n${headers.join('\r\n')}\r\n\r\n`);
      if (head?.length) upstreamSocket.write(head);
      socket.pipe(upstreamSocket).pipe(socket);
    });
    const fail = () => {
      if (!socket.destroyed) socket.end('HTTP/1.1 502 Bad Gateway\r\nConnection: close\r\n\r\n');
      upstreamSocket.destroy();
    };
    upstreamSocket.once('error', fail);
    socket.once('error', () => upstreamSocket.destroy());
    socket.once('close', () => {
      upgradeSockets.delete(socket);
      upstreamSocket.destroy();
    });
    upstreamSocket.once('close', () => {
      upgradeSockets.delete(upstreamSocket);
      socket.destroy();
    });
  }
  let refreshTimer = null;
  function start() {
    const schedule = () => {
      clearTimeout(refreshTimer);
      refreshTimer = setTimeout(() => {
        for (const mapping of mappings().filter((item) => item.enabled)) {
          try { sync(mapping.id); } catch {}
        }
        schedule();
      }, settings().refreshMinutes * 60_000);
      refreshTimer.unref?.();
    };
    schedule();
  }
  function close() {
    clearTimeout(refreshTimer);
    for (const socket of upgradeSockets) socket.destroy();
    upgradeSockets.clear();
  }
  return { info, updateSettings, upsert, remove, sync, handleProxy, proxyUpgrade, setProxyStatus, start, close };
}
