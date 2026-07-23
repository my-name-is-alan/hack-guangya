export function parseGuangyaShareLink(value) {
  const text = String(value || '').trim();
  const candidate = text.match(/https?:\/\/[^\s<>"']+/i)?.[0] || text;
  let parsed;
  try {
    parsed = new URL(candidate.replace(/[，。；;]+$/, ''));
  } catch {
    throw new Error('请输入完整的光鸭分享链接');
  }
  const host = parsed.hostname.toLowerCase();
  if (host !== 'guangyapan.com' && !host.endsWith('.guangyapan.com')) {
    throw new Error('只支持 guangyapan.com 的分享链接');
  }
  const parts = parsed.pathname.split('/').filter(Boolean);
  const shareIndex = parts.findIndex((part) => part.toLowerCase() === 's');
  const shareId = shareIndex >= 0 ? parts[shareIndex + 1] || '' : '';
  if (!/^[a-zA-Z0-9_-]+$/.test(shareId)) throw new Error('光鸭分享链接中缺少有效的 share_id');
  const code = String(parsed.searchParams.get('code') || '').trim();
  const normalized = new URL(`https://www.guangyapan.com/s/${shareId}`);
  if (code) normalized.searchParams.set('code', code);
  normalized.hash = '/share';
  return { shareId, code, url: normalized.toString() };
}
