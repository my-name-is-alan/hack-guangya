import http from 'node:http';
import fsp from 'node:fs/promises';
import path from 'node:path';
import crypto from 'node:crypto';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const uiRoot = path.join(root, 'dist');
const port = Number(process.env.PORT || 18081);
const state = { logged_in: true, paused: false, pending: 0, active_uploads: 0, mappings: [], saved_shares: [], hdhive: { configured: false, base_url: '', instance_id: 'mock-instance' }, auto_share_receipts: [] };
const files = new Map([
  ['', [
    { fileId: 'folder-docs', fileName: '工作资料', resType: 2, subFolderCount: 2, utime: 1784600000 },
    { fileId: 'folder-media', fileName: '媒体素材', resType: 2, subFolderCount: 1, utime: 1784500000 },
    { fileId: 'file-1', fileName: 'IMG_2026_001.JPG', resType: 1, ext: 'JPG', fileSize: 3567104, utime: 1784400000 },
    { fileId: 'file-2', fileName: 'IMG_2026_002.JPG', resType: 1, ext: 'JPG', fileSize: 4096000, utime: 1784300000 },
    { fileId: 'file-3', fileName: '项目说明.docx', resType: 1, ext: 'docx', fileSize: 88312, utime: 1784200000 },
  ]],
  ['folder-docs', [
    { fileId: 'folder-2026', fileName: '2026', resType: 2, subFolderCount: 0, utime: 1784600000 },
    { fileId: 'folder-contracts', fileName: '合同', resType: 2, subFolderCount: 0, utime: 1784500000 },
  ]],
  ['folder-media', [{ fileId: 'folder-images', fileName: '图片', resType: 2, subFolderCount: 0, utime: 1784400000 }]],
]);

function json(response, status, payload) { response.writeHead(status, { 'content-type': 'application/json; charset=utf-8' }); response.end(JSON.stringify(payload)); }
async function body(request) { const chunks = []; for await (const chunk of request) chunks.push(chunk); return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {}; }
function contentType(file) { return file.endsWith('.html') ? 'text/html; charset=utf-8' : file.endsWith('.js') ? 'text/javascript; charset=utf-8' : file.endsWith('.css') ? 'text/css; charset=utf-8' : 'application/octet-stream'; }
async function staticFile(response, pathname) { const file = path.resolve(uiRoot, `.${pathname === '/' ? '/index.html' : pathname}`); if (!file.startsWith(uiRoot + path.sep)) return json(response, 403, { error: 'forbidden' }); try { const content = await fsp.readFile(file); response.writeHead(200, { 'content-type': contentType(file) }); response.end(content); } catch { json(response, 404, { error: 'not found' }); } }

const server = http.createServer(async (request, response) => {
  const url = new URL(request.url, `http://${request.headers.host}`);
  if (!url.pathname.startsWith('/api/')) return staticFile(response, url.pathname);
  if (request.method === 'GET' && url.pathname === '/api/state') return json(response, 200, state);
  if (request.method === 'GET' && url.pathname === '/api/overview') return json(response, 200, { profile: { sub: 'mock-user-001', name: '界面测试账号', phone_number: '138****8000', picture: 'data:image/svg+xml,%3Csvg xmlns="http://www.w3.org/2000/svg" width="96" height="96"%3E%3Crect width="96" height="96" rx="48" fill="%231677ff"/%3E%3Ctext x="48" y="60" text-anchor="middle" font-size="36" fill="white"%3E%E6%B5%8B%3C/text%3E%3C/svg%3E' }, assets: { usedSpaceSize: 32212254720, totalSpaceSize: 107374182400, vipStatus: 2, svipStatus: 0, vipExpireTime: Math.floor(Date.now() / 1000) + 86400 * 30, systemTime: Math.floor(Date.now() / 1000) } });
  if (request.method === 'GET' && url.pathname === '/api/events') { response.writeHead(200, { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' }); response.write(`data: ${JSON.stringify({ type: 'state', state })}\n\n`); return; }
  if (request.method === 'GET' && url.pathname === '/api/files') { const list = files.get(url.searchParams.get('parentId') || '') || []; return json(response, 200, { list, total: list.length }); }
  if (request.method === 'POST' && url.pathname === '/api/mappings') { const input = await body(request); const mapping = { id: crypto.randomUUID(), ...input, enabled: true }; state.mappings.push(mapping); return json(response, 200, mapping); }
  if (request.method === 'POST' && url.pathname === '/api/hdhive/config') { const input = await body(request); state.hdhive = { ...state.hdhive, configured: Boolean(input.base_url && input.secret), base_url: input.base_url || '' }; return json(response, 200, state.hdhive); }
  if (request.method === 'POST' && /^\/api\/mappings\/[^/]+\/auto-share-backfill$/.test(url.pathname)) return json(response, 202, { scheduled: 0 });
  if (request.method === 'POST' && /^\/api\/auto-share\/events\/[^/]+\/retry$/.test(url.pathname)) return json(response, 202, { status: 'accepted' });
  if (request.method === 'PATCH' && url.pathname.startsWith('/api/mappings/')) { const input = await body(request); const id = decodeURIComponent(url.pathname.split('/').pop()); const mapping = state.mappings.find((item) => item.id === id); if (mapping) Object.assign(mapping, input); return json(response, mapping ? 200 : 404, mapping || { error: 'not found' }); }
  if (request.method === 'POST' && url.pathname === '/api/files/rename-batch') { const input = await body(request); for (const rename of input.renames || []) for (const list of files.values()) { const item = list.find((entry) => entry.fileId === rename.fileId); if (item) item.fileName = rename.newName; } return json(response, 200, { renamed: input.renames?.length || 0 }); }
  if (request.method === 'POST' && ['/api/files/copy', '/api/files/move', '/api/files/delete', '/api/share', '/api/offline'].includes(url.pathname)) { await body(request); return json(response, 200, {}); }
  if (request.method === 'POST' && url.pathname === '/api/upload') { for await (const _ of request) {} return json(response, 200, { uploaded: 1 }); }
  if (request.method === 'GET' && url.pathname === '/api/offline') return json(response, 200, { list: [] });
  return json(response, 200, {});
});

server.listen(port, '127.0.0.1', () => console.log(`Mock Guangya UI server listening on ${port}`));
