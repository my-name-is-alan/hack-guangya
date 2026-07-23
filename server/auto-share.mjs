import crypto from 'node:crypto';

function normalizeRemote(value) {
  return String(value || '').replaceAll('\\', '/').split('/').filter(Boolean).join('/');
}

export function autoShareTargetFor(relativePath, mappingId = 'mapping') {
  if (!relativePath || !mappingId || String(mappingId).startsWith('__')) return null;
  const parts = normalizeRemote(relativePath).split('/').filter(Boolean);
  if (!parts.length) return null;
  return {
    key: parts[0],
    type: parts.length === 1 ? 'file' : 'folder',
    title: parts[0],
    relativePath: parts.join('/'),
  };
}

export const DEFAULT_SHARE_TEMPLATE = '光鸭云盘用户给你分享了{{filename}}，点击链接或复制整段内容，打开「光鸭APP」即可获取。\n链接：{{link}}';

export function shareFilePayload(fileIds, title = '') {
  const normalizedTitle = String(title || '').trim() || '云盘分享';
  return {
    fileIds,
    title: normalizedTitle,
    validateDuration: 0,
    shareType: 0,
    code: '',
    autoFillCode: false,
    // 与光鸭网页版的普通分享请求保持一致。
    trafficLimit: '0',
    maxRestoreCount: 0,
    downloadType: 1,
    shareTemplate: DEFAULT_SHARE_TEMPLATE,
  };
}

export function signHdhiveRequest(secret, method, pathname, bodyText, timestamp) {
  const bodyHash = crypto.createHash('sha256').update(bodyText).digest('hex');
  const canonical = `${timestamp}\n${method.toUpperCase()}\n${pathname}\n${bodyHash}`;
  return `v1=${crypto.createHmac('sha256', secret).update(canonical).digest('hex')}`;
}
