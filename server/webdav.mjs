import crypto from 'node:crypto';

export class WebDavError extends Error {
  constructor(statusCode, message, headers = {}) {
    super(message);
    this.name = 'WebDavError';
    this.statusCode = statusCode;
    this.headers = headers;
  }
}

function xmlEscape(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&apos;');
}

function validSegment(value) {
  const segment = String(value || '');
  if (!segment || segment === '.' || segment === '..' || /[\\/\0]/.test(segment)) {
    throw new WebDavError(400, 'WebDAV 路径无效');
  }
  return segment;
}

export function decodeWebDavPath(pathname, prefix = '/dav') {
  const normalizedPrefix = `/${String(prefix || '').replace(/^\/+|\/+$/g, '')}`;
  if (pathname !== normalizedPrefix && !pathname.startsWith(`${normalizedPrefix}/`)) {
    throw new WebDavError(404, 'WebDAV 路径不存在');
  }
  const relative = pathname.slice(normalizedPrefix.length).replace(/^\/+|\/+$/g, '');
  if (!relative) return [];
  try {
    return relative.split('/').map((part) => validSegment(decodeURIComponent(part)));
  } catch (error) {
    if (error instanceof WebDavError) throw error;
    throw new WebDavError(400, 'WebDAV 路径编码无效');
  }
}

function entryTimestamp(raw) {
  const value = raw.updatedAt ?? raw.updateTime ?? raw.modifiedAt ?? raw.modifyTime
    ?? raw.utime ?? raw.createdAt ?? raw.createTime ?? raw.ctime ?? 0;
  if (typeof value === 'string' && !/^\d+$/.test(value)) {
    const parsed = Date.parse(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  const number = Number(value || 0);
  if (!Number.isFinite(number) || number <= 0) return Date.now();
  return number < 10_000_000_000 ? number * 1000 : number;
}

export function normalizeWebDavEntry(raw) {
  const id = String(raw?.fileId ?? raw?.id ?? '');
  const name = String(raw?.fileName ?? raw?.name ?? '');
  const isDirectory = Number(raw?.resType ?? raw?.type) === 2 || raw?.isDirectory === true;
  const size = Math.max(0, Number(raw?.fileSize ?? raw?.size ?? 0) || 0);
  const modifiedAt = entryTimestamp(raw || {});
  return {
    id,
    name,
    isDirectory,
    size,
    modifiedAt,
    etag: `"gy-${id || crypto.createHash('sha1').update(name).digest('hex')}-${Math.round(modifiedAt)}-${size}"`,
    raw,
  };
}

function encodedHref(prefix, segments, isDirectory) {
  const base = `/${String(prefix || '').replace(/^\/+|\/+$/g, '')}`;
  const suffix = segments.map((segment) => encodeURIComponent(segment)).join('/');
  const href = suffix ? `${base}/${suffix}` : `${base}/`;
  return isDirectory && !href.endsWith('/') ? `${href}/` : href;
}

function contentType(entry) {
  if (entry.isDirectory) return 'httpd/unix-directory';
  const extension = entry.name.split('.').pop()?.toLowerCase();
  return {
    txt: 'text/plain; charset=utf-8',
    md: 'text/markdown; charset=utf-8',
    json: 'application/json',
    pdf: 'application/pdf',
    jpg: 'image/jpeg',
    jpeg: 'image/jpeg',
    png: 'image/png',
    gif: 'image/gif',
    webp: 'image/webp',
    svg: 'image/svg+xml',
    mp4: 'video/mp4',
    mp3: 'audio/mpeg',
    zip: 'application/zip',
  }[extension] || 'application/octet-stream';
}

function propertyResponse(prefix, segments, entry) {
  const href = encodedHref(prefix, segments, entry.isDirectory);
  const resourceType = entry.isDirectory ? '<D:collection/>' : '';
  const length = entry.isDirectory ? '' : `<D:getcontentlength>${entry.size}</D:getcontentlength>`;
  return `<D:response>
    <D:href>${xmlEscape(href)}</D:href>
    <D:propstat>
      <D:prop>
        <D:displayname>${xmlEscape(entry.name || '光鸭云盘')}</D:displayname>
        <D:resourcetype>${resourceType}</D:resourcetype>
        ${length}
        <D:getcontenttype>${xmlEscape(contentType(entry))}</D:getcontenttype>
        <D:getlastmodified>${new Date(entry.modifiedAt).toUTCString()}</D:getlastmodified>
        <D:creationdate>${new Date(entry.modifiedAt).toISOString()}</D:creationdate>
        <D:getetag>${xmlEscape(entry.etag)}</D:getetag>
        <D:supportedlock>
          <D:lockentry><D:lockscope><D:exclusive/></D:lockscope><D:locktype><D:write/></D:locktype></D:lockentry>
        </D:supportedlock>
      </D:prop>
      <D:status>HTTP/1.1 200 OK</D:status>
    </D:propstat>
  </D:response>`;
}

function directoryIndex(prefix, segments, entry, children) {
  const title = entry.name || '光鸭云盘';
  const parent = segments.length
    ? `<li><a href="${xmlEscape(encodedHref(prefix, segments.slice(0, -1), true))}">../</a></li>`
    : '';
  const items = [...children]
    .sort((left, right) => Number(right.isDirectory) - Number(left.isDirectory)
      || left.name.localeCompare(right.name, 'zh-CN'))
    .map((child) => {
      const href = encodedHref(prefix, [...segments, child.name], child.isDirectory);
      const label = `${child.name}${child.isDirectory ? '/' : ''}`;
      return `<li><a href="${xmlEscape(href)}">${xmlEscape(label)}</a></li>`;
    })
    .join('');
  return `<!doctype html>
<html lang="zh-CN">
<head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>${xmlEscape(title)}</title></head>
<body><h1>${xmlEscape(title)}</h1><ul>${parent}${items}</ul></body>
</html>`;
}

function send(response, statusCode, body = '', headers = {}) {
  response.writeHead(statusCode, {
    'cache-control': 'no-store',
    ...headers,
  });
  response.end(body);
}

async function findChild(listChildren, parentId, name, options) {
  const children = (await listChildren(parentId, options)).map(normalizeWebDavEntry);
  const exact = children.find((entry) => entry.name === name);
  if (exact) return exact;
  const folded = children.filter((entry) => entry.name.toLocaleLowerCase() === name.toLocaleLowerCase());
  return folded.length === 1 ? folded[0] : null;
}

async function resolveEntry(listChildren, segments, options) {
  if (!segments.length) {
    return {
      entry: {
        id: '',
        name: '光鸭云盘',
        isDirectory: true,
        size: 0,
        modifiedAt: Date.now(),
        etag: '"gy-root"',
        raw: {},
      },
      parentId: '',
    };
  }
  let parentId = '';
  let entry = null;
  for (const segment of segments) {
    entry = await findChild(listChildren, parentId, segment, options);
    if (!entry) throw new WebDavError(404, `云端项目不存在：${segment}`);
    if (segment !== segments.at(-1) && !entry.isDirectory) {
      throw new WebDavError(409, `路径中包含文件：${segment}`);
    }
    const previousParentId = parentId;
    parentId = entry.id;
    entry.parentId = previousParentId;
  }
  return { entry, parentId: entry.parentId };
}

async function resolveParent(listChildren, segments, options) {
  if (!segments.length) throw new WebDavError(403, '不能修改 WebDAV 根目录');
  const name = segments.at(-1);
  if (segments.length === 1) return {
    parentId: '',
    name,
    existing: await findChild(listChildren, '', name, options),
  };
  const { entry } = await resolveEntry(listChildren, segments.slice(0, -1), options);
  if (!entry.isDirectory) throw new WebDavError(409, '目标父路径不是目录');
  return {
    parentId: entry.id,
    name,
    existing: await findChild(listChildren, entry.id, name, options),
  };
}

function readCacheOptions(request) {
  const cacheControl = String(request.headers['cache-control'] || '');
  const pragma = String(request.headers.pragma || '');
  if (/\b(?:no-cache|no-store|max-age\s*=\s*0)\b/i.test(cacheControl) || /\bno-cache\b/i.test(pragma)) {
    return { force: true, foreground: true };
  }
  // Normal reads use the backend's short fresh window and stale-while-revalidate
  // policy. Merely visiting a directory after that window triggers a refresh.
  return undefined;
}

const WRITE_CACHE_OPTIONS = Object.freeze({ force: true, foreground: true });

function destinationSegments(request, prefix) {
  const value = String(request.headers.destination || '');
  if (!value) throw new WebDavError(400, '缺少 Destination 请求头');
  let pathname;
  try {
    pathname = new URL(value, 'http://localhost').pathname;
  } catch {
    throw new WebDavError(400, 'Destination 地址无效');
  }
  return decodeWebDavPath(pathname, prefix);
}

export function createWebDavHandler({
  prefix = '/dav',
  listChildren,
  createDirectory,
  deleteEntry,
  moveEntry,
  copyEntry,
  putFile,
  readFile,
}) {
  if (![listChildren, createDirectory, deleteEntry, moveEntry, copyEntry, putFile, readFile].every((item) => typeof item === 'function')) {
    throw new TypeError('WebDAV backend 不完整');
  }

  return async function handleWebDav(request, response, url) {
    const method = String(request.method || 'GET').toUpperCase();
    const segments = decodeWebDavPath(url.pathname, prefix);

    if (method === 'OPTIONS') {
      return send(response, 204, '', {
        allow: 'OPTIONS, PROPFIND, PROPPATCH, GET, HEAD, PUT, MKCOL, DELETE, MOVE, COPY',
        dav: '1',
        ms_author_via: 'DAV',
      });
    }

    if (method === 'PROPFIND') {
      const depth = String(request.headers.depth || '1').toLowerCase();
      if (!['0', '1'].includes(depth)) {
        throw new WebDavError(403, '仅支持 Depth: 0 或 1');
      }
      const cacheOptions = readCacheOptions(request);
      const { entry } = await resolveEntry(listChildren, segments, cacheOptions);
      const responses = [propertyResponse(prefix, segments, entry)];
      if (depth === '1' && entry.isDirectory) {
        const children = (await listChildren(entry.id, cacheOptions)).map(normalizeWebDavEntry);
        for (const child of children) responses.push(propertyResponse(prefix, [...segments, child.name], child));
      }
      const body = `<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">${responses.join('')}</D:multistatus>`;
      return send(response, 207, body, { 'content-type': 'application/xml; charset=utf-8', dav: '1' });
    }

    if (method === 'PROPPATCH') {
      const { entry } = await resolveEntry(listChildren, segments, readCacheOptions(request));
      const body = `<?xml version="1.0" encoding="utf-8"?><D:multistatus xmlns:D="DAV:">${propertyResponse(prefix, segments, entry)}</D:multistatus>`;
      return send(response, 207, body, { 'content-type': 'application/xml; charset=utf-8', dav: '1' });
    }

    if (method === 'GET' || method === 'HEAD') {
      const cacheOptions = readCacheOptions(request);
      const { entry } = await resolveEntry(listChildren, segments, cacheOptions);
      if (entry.isDirectory) {
        const children = (await listChildren(entry.id, cacheOptions)).map(normalizeWebDavEntry);
        const body = directoryIndex(prefix, segments, entry, children);
        return send(response, 200, method === 'HEAD' ? '' : body, {
          'content-type': 'text/html; charset=utf-8',
          'content-length': String(Buffer.byteLength(body)),
        });
      }
      return readFile({ request, response, entry, headOnly: method === 'HEAD' });
    }

    if (method === 'PUT') {
      const target = await resolveParent(listChildren, segments, WRITE_CACHE_OPTIONS);
      if (target.existing?.isDirectory) throw new WebDavError(405, '不能用文件覆盖目录');
      const result = await putFile({ request, ...target });
      return send(response, target.existing ? 204 : 201, '', {
        etag: result?.etag || `"gy-${result?.id || crypto.randomUUID()}"`,
      });
    }

    if (method === 'MKCOL') {
      const target = await resolveParent(listChildren, segments, WRITE_CACHE_OPTIONS);
      if (target.existing) throw new WebDavError(405, '目标已经存在');
      await createDirectory({ parentId: target.parentId, name: target.name });
      return send(response, 201);
    }

    if (method === 'DELETE') {
      const { entry } = await resolveEntry(listChildren, segments, WRITE_CACHE_OPTIONS);
      if (!entry.id) throw new WebDavError(403, '不能删除 WebDAV 根目录');
      await deleteEntry({ entry });
      return send(response, 204);
    }

    if (method === 'MOVE' || method === 'COPY') {
      const { entry } = await resolveEntry(listChildren, segments, WRITE_CACHE_OPTIONS);
      if (!entry.id) throw new WebDavError(403, '不能移动或复制 WebDAV 根目录');
      const destination = await resolveParent(
        listChildren,
        destinationSegments(request, prefix),
        WRITE_CACHE_OPTIONS,
      );
      const overwrite = String(request.headers.overwrite || 'T').toUpperCase() !== 'F';
      if (destination.existing?.id === entry.id) {
        if (method === 'MOVE') return send(response, 204);
        throw new WebDavError(403, '不能把资源复制到自身');
      }
      if (destination.existing && !overwrite) throw new WebDavError(412, '目标已经存在');
      const replaced = destination.existing;
      let backup = null;
      if (replaced) {
        backup = {
          ...replaced,
          parentId: destination.parentId,
          name: `.__gy_dav_backup_${crypto.randomUUID().replaceAll('-', '')}`,
        };
        await moveEntry({ entry: replaced, parentId: destination.parentId, name: backup.name });
      }
      try {
        if (method === 'MOVE') {
          await moveEntry({ entry, parentId: destination.parentId, name: destination.name });
        } else {
          await copyEntry({ entry, parentId: destination.parentId, name: destination.name });
        }
      } catch (error) {
        if (backup) {
          try {
            await moveEntry({ entry: backup, parentId: destination.parentId, name: destination.name });
          } catch (rollbackError) {
            throw new WebDavError(500, `${error.message}；恢复被覆盖目标也失败：${rollbackError.message}`);
          }
        }
        throw error;
      }
      if (backup) await deleteEntry({ entry: backup });
      return send(response, replaced ? 204 : 201);
    }

    throw new WebDavError(405, `不支持 WebDAV 方法：${method}`, {
      allow: 'OPTIONS, PROPFIND, PROPPATCH, GET, HEAD, PUT, MKCOL, DELETE, MOVE, COPY',
    });
  };
}
