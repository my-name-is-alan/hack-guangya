export function pick(object, keys, fallback = '') {
  for (const key of keys) if (object && object[key] !== undefined && object[key] !== null && object[key] !== '') return object[key];
  return fallback;
}
export function normalizeAvatarUrl(value) {
  const source = value && typeof value === 'object' ? pick(value, ['url', 'src', 'original', 'large'], '') : value;
  const url = String(source || '').trim();
  return url.startsWith('//') ? `https:${url}` : url;
}
export function unwrapData(payload) { return payload?.data || payload || {}; }
export function errorText(error) { return error instanceof Error ? error.message : String(error?.message || error || '未知错误'); }
export function formatSize(size) {
  const number = Number(size || 0);
  if (!number) return '—';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let index = 0;
  let value = number;
  while (value >= 1024 && index < units.length - 1) { value /= 1024; index += 1; }
  return `${value >= 10 || index === 0 ? Math.round(value) : value.toFixed(1)} ${units[index]}`;
}
export function formatTime(value) {
  if (!value) return '—';
  const number = Number(value);
  const date = new Date(number < 10 ** 12 ? number * 1000 : number);
  return Number.isNaN(date.getTime()) ? '—' : date.toLocaleString();
}
export function fileId(record) { return pick(record, ['fileId', 'id']); }
export function isFolder(record) { return Number(record.resType) === 2; }
export function uploadFileName(filePath) {
  return String(filePath || '').split(/[\\/]/).filter(Boolean).pop() || '未命名文件';
}
export function newDownloadId() {
  return globalThis.crypto?.randomUUID?.() || `download-${Date.now()}-${Math.random().toString(16).slice(2)}`;
}
export async function copyText(value, message) {
  try { await navigator.clipboard.writeText(value); message.success('已复制到剪贴板'); }
  catch { message.info(value); }
}

const OFFLINE_STATUS_MAP = {
  0: ['排队等待', 'default'],
  1: ['下载中', 'processing'],
  2: ['已完成', 'success'],
  3: ['下载失败', 'error'],
  4: ['已取消', 'warning'],
  5: ['资源违规', 'error'],
};
export function offlineStatus(record) {
  const raw = pick(record, ['status', 'taskStatus', 'state'], null);
  if (raw === null || raw === '') {
    const err = Number(pick(record, ['errCode', 'errorCode'], 0));
    return err ? ['下载失败', 'error'] : ['处理中', 'processing'];
  }
  const text = String(raw).trim();
  if (/^-?\d+$/.test(text)) return OFFLINE_STATUS_MAP[Number(text)] || ['处理中', 'processing'];
  const lowered = text.toLowerCase();
  if (['success', 'done', 'complete', 'completed', 'finish', 'finished'].includes(lowered)) return ['已完成', 'success'];
  if (['fail', 'failed', 'error'].includes(lowered)) return ['下载失败', 'error'];
  if (['cancel', 'canceled', 'cancelled'].includes(lowered)) return ['已取消', 'warning'];
  if (['pending', 'waiting', 'queue', 'queued'].includes(lowered)) return ['排队等待', 'default'];
  return ['下载中', 'processing'];
}

export function cloudShareStatus(record) {
  return ({ 1: ['分享中', 'success'], 2: ['已过期', 'warning'], 3: ['已取消', 'default'], 4: ['已封禁', 'error'] })[Number(record.shareStatus)] || ['未知', 'default'];
}

export const sourcePolicyLabel = (value) => ({ keep: '保留源文件', archive: '上传后归档', delete: '上传后删除' }[value] || value);
export const sourcePolicyColor = (value) => ({ keep: 'blue', archive: 'gold', delete: 'red' }[value] || 'default');

export function receiptStatusLabel(receipt) {
  return ({ accepted: 'Hdhive 已接收', processing: 'Hdhive 处理中', completed: '处理完成', needs_review: '待人工处理', failed: '处理失败', delivery_failed: '等待重新投递', waiting_upload: '等待失败文件重传', sending: '正在通知 Hdhive' })[receipt.status] || receipt.status;
}
export function receiptActionLabel(action) {
  return ({ created: '已投稿', updated: '已更新', no_change: '内容未变化', baseline_initialized: '已建立内容基线' })[action] || action || '';
}
export function receiptDisplayMessage(receipt) {
  if (!receipt) return '';
  if (receipt.status === 'completed') {
    const outcome = ({ created: '影巢投稿完成', updated: '影巢内容更新完成', no_change: '影巢确认内容没有变化', baseline_initialized: '影巢已建立内容基线' })[receipt.action] || '影巢处理完成';
    return receipt.notification_status === 'sent' ? `${outcome}，消息已推送` : outcome;
  }
  if (receipt.status === 'needs_review') return receipt.message || '影巢需要人工补充信息';
  if (['failed', 'delivery_failed'].includes(receipt.status)) return receipt.message || '影巢处理失败，请重试';
  return receipt.message || (receipt.status === 'processing' ? '影巢正在解析并投稿' : '影巢已接收，等待处理');
}
export function receiptAlertType(status) {
  if (status === 'completed') return 'success';
  if (['failed', 'delivery_failed'].includes(status)) return 'error';
  if (status === 'needs_review') return 'warning';
  return 'info';
}
export function receiptColor(status) {
  return status === 'completed' ? 'green' : status === 'needs_review' ? 'orange' : ['failed', 'delivery_failed'].includes(status) ? 'red' : status === 'waiting_upload' ? 'gold' : 'blue';
}

export const extensionPresets = [
  { key: 'video', label: '视频', extensions: ['mp4', 'mov', 'mkv', 'avi', 'wmv', 'flv', 'webm', 'm4v', 'ts', 'mts', 'm2ts', '3gp'] },
  { key: 'image', label: '图片', extensions: ['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'heic', 'heif', 'avif', 'tif', 'tiff', 'raw', 'cr2', 'nef', 'arw', 'dng'] },
  { key: 'subtitle', label: '字幕', extensions: ['srt', 'ass', 'ssa', 'vtt', 'sub', 'idx', 'sup', 'lrc'] },
  { key: 'audio', label: '音频', extensions: ['mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg', 'opus', 'wma', 'aiff'] },
];
export const defaultSyncExtensions = [
  ...extensionPresets.find((item) => item.key === 'image').extensions,
  ...extensionPresets.find((item) => item.key === 'video').extensions,
  ...extensionPresets.find((item) => item.key === 'audio').extensions,
];
export function presetExtensions(key) {
  return [...(extensionPresets.find((item) => item.key === key)?.extensions || [])];
}
export function normalizeExtensions(values) {
  const seen = new Set();
  const result = [];
  for (const raw of Array.isArray(values) ? values : []) {
    const value = String(raw || '').trim().replace(/^\./, '').toLowerCase();
    if (!value || seen.has(value)) continue;
    seen.add(value);
    result.push(value);
  }
  return result;
}
export function mappingExtensions(mapping) {
  const normalized = normalizeExtensions(mapping?.sync_types);
  return normalized.length ? normalized : [...defaultSyncExtensions];
}
export function syncTypeSummary(mapping) {
  const extensions = mappingExtensions(mapping);
  return extensions.length > 3 ? `${extensions.slice(0, 3).join('/')} 等 ${extensions.length} 项` : extensions.join('/');
}
