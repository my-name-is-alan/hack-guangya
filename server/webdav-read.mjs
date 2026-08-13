import { pipeline } from 'node:stream/promises';
import { WebDavError } from './webdav.mjs';

const REDIRECT_MODES = new Set(['off', 'auto', 'always']);
// WebDAV clients known to mishandle 302 on GET; they keep the streaming proxy path.
const REDIRECT_INCAPABLE_AGENTS = /microsoft-webdav-miniredir|webdavfs|davfs|gvfs/i;

const MIME_TYPES = {
  aac: 'audio/aac',
  ass: 'text/plain; charset=utf-8',
  avi: 'video/x-msvideo',
  flac: 'audio/flac',
  flv: 'video/x-flv',
  gif: 'image/gif',
  jpeg: 'image/jpeg',
  jpg: 'image/jpeg',
  json: 'application/json',
  m2ts: 'video/mp2t',
  m4a: 'audio/mp4',
  m4v: 'video/x-m4v',
  md: 'text/markdown; charset=utf-8',
  mkv: 'video/x-matroska',
  mov: 'video/quicktime',
  mp3: 'audio/mpeg',
  mp4: 'video/mp4',
  mpeg: 'video/mpeg',
  mpg: 'video/mpeg',
  nfo: 'text/xml; charset=utf-8',
  ogg: 'audio/ogg',
  opus: 'audio/opus',
  pdf: 'application/pdf',
  png: 'image/png',
  srt: 'text/plain; charset=utf-8',
  ssa: 'text/plain; charset=utf-8',
  svg: 'image/svg+xml',
  ts: 'video/mp2t',
  txt: 'text/plain; charset=utf-8',
  wav: 'audio/wav',
  webm: 'video/webm',
  webp: 'image/webp',
  wmv: 'video/x-ms-wmv',
  xml: 'application/xml',
  zip: 'application/zip',
};

export function webDavContentType(name) {
  const extension = String(name || '').split('.').pop()?.toLowerCase();
  return MIME_TYPES[extension] || 'application/octet-stream';
}

export function normalizeWebDavRedirectMode(value) {
  const mode = String(value ?? '').trim().toLowerCase();
  if (!mode || mode === '1' || mode === 'on' || mode === 'true' || mode === 'auto') return 'auto';
  if (mode === '0' || mode === 'false' || mode === 'off') return 'off';
  if (!REDIRECT_MODES.has(mode)) throw new Error('GUANGYA_WEBDAV_REDIRECT 只支持 off、auto 或 always');
  return mode;
}

export function webDavRedirectAllowed(mode, userAgent) {
  if (mode === 'always') return true;
  if (mode === 'off') return false;
  return !REDIRECT_INCAPABLE_AGENTS.test(String(userAgent || ''));
}

function webDavEtagMatches(value, etag) {
  const expected = String(etag || '').replace(/^W\//i, '');
  return String(value || '').split(',').map((item) => item.trim()).some((item) => item === '*' || item.replace(/^W\//i, '') === expected);
}

function finishConditional(response, statusCode, entry) {
  response.writeHead(statusCode, {
    etag: entry.etag,
    'last-modified': new Date(entry.modifiedAt).toUTCString(),
  });
  response.end();
}

function conditionalStatus(request, entry) {
  const ifMatch = request.headers['if-match'];
  if (ifMatch && !webDavEtagMatches(ifMatch, entry.etag)) return 412;
  const ifUnmodifiedSince = Date.parse(String(request.headers['if-unmodified-since'] || ''));
  if (!ifMatch && Number.isFinite(ifUnmodifiedSince) && entry.modifiedAt > ifUnmodifiedSince + 999) return 412;
  const ifNoneMatch = request.headers['if-none-match'];
  if (ifNoneMatch && webDavEtagMatches(ifNoneMatch, entry.etag)) return 304;
  const ifModifiedSince = Date.parse(String(request.headers['if-modified-since'] || ''));
  if (!ifNoneMatch && Number.isFinite(ifModifiedSince) && entry.modifiedAt <= ifModifiedSince + 999) return 304;
  return 0;
}

export function createWebDavFileReader({
  downloadUrls,
  fetchImpl = fetch,
  redirectMode = 'auto',
  timeoutMs = 600_000,
}) {
  if (typeof downloadUrls?.get !== 'function') throw new TypeError('WebDAV 读取需要直链缓存');
  const mode = normalizeWebDavRedirectMode(redirectMode);

  return async function readWebDavFile({ request, response, entry, headOnly }) {
    const condition = conditionalStatus(request, entry);
    if (condition) {
      finishConditional(response, condition, entry);
      return;
    }
    if (headOnly) {
      response.writeHead(200, {
        'accept-ranges': 'bytes',
        'content-type': webDavContentType(entry.name),
        'content-length': String(entry.size),
        etag: entry.etag,
        'last-modified': new Date(entry.modifiedAt).toUTCString(),
      });
      response.end();
      return;
    }
    if (webDavRedirectAllowed(mode, request.headers['user-agent'])) {
      const location = await downloadUrls.get(entry.id);
      response.writeHead(302, {
        location,
        'cache-control': 'no-store',
        'accept-ranges': 'bytes',
        etag: entry.etag,
        'last-modified': new Date(entry.modifiedAt).toUTCString(),
      });
      response.end();
      return;
    }
    const fetchDownload = async (force) => {
      const url = await downloadUrls.get(entry.id, { force });
      const headers = {};
      if (request.headers.range) headers.range = request.headers.range;
      return fetchImpl(url, { method: 'GET', headers, signal: AbortSignal.timeout(timeoutMs) });
    };
    let upstream = await fetchDownload(false);
    if (upstream.status === 403 || upstream.status === 410) {
      // Cached signed URL may have expired upstream; refresh once.
      await upstream.body?.cancel();
      upstream = await fetchDownload(true);
    }
    if (!upstream.ok && upstream.status !== 206 && upstream.status !== 304 && upstream.status !== 416) {
      await upstream.body?.cancel();
      downloadUrls.invalidate(entry.id);
      throw new WebDavError(upstream.status === 404 ? 404 : 502, `云端文件读取失败（HTTP ${upstream.status}）`);
    }
    const responseHeaders = {
      'accept-ranges': upstream.headers.get('accept-ranges') || 'bytes',
      'content-type': upstream.headers.get('content-type') || webDavContentType(entry.name),
      etag: entry.etag,
      'last-modified': new Date(entry.modifiedAt).toUTCString(),
    };
    for (const name of ['content-length', 'content-range', 'content-disposition']) {
      const value = upstream.headers.get(name);
      if (value) responseHeaders[name] = value;
    }
    response.writeHead(upstream.status, responseHeaders);
    if (!upstream.body) {
      response.end();
      return;
    }
    await pipeline(upstream.body, response);
  };
}
