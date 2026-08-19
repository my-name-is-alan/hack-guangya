// 文件打开分发的纯函数：类型识别、LRC 歌词解析、外部播放器 scheme。
// 打开行为的扩展名集合独立于备份同步的 extensionPresets，
// 避免影响备份格式选择 UI。

import { pick, presetExtensions } from './formatters.js';

export const OPEN_KIND = Object.freeze({
  FOLDER: 'folder',
  VIDEO: 'video',
  AUDIO: 'audio',
  IMAGE: 'image',
  TEXT: 'text',
  OTHER: 'other',
});

// 视频统一交给外部播放器，格式覆盖比浏览器解码更广。
const VIDEO_OPEN_EXTENSIONS = new Set([...presetExtensions('video'), 'rmvb', 'rm']);
// 图片查看器基于 <img>，只接管浏览器能渲染的格式；heic/raw 走详情或下载。
const IMAGE_OPEN_EXTENSIONS = new Set(['jpg', 'jpeg', 'png', 'gif', 'webp', 'bmp', 'svg', 'avif', 'ico', 'jfif']);
const AUDIO_OPEN_EXTENSIONS = new Set(presetExtensions('audio'));
// Chromium 系（含 WebView2）可原生解码的音频容器；之外的格式引导外部播放器或下载。
export const BROWSER_AUDIO_EXTENSIONS = new Set(['mp3', 'wav', 'flac', 'aac', 'm4a', 'ogg', 'opus', 'weba']);
// sup 是位图字幕（二进制），不按文本处理。
const SUBTITLE_TEXT_EXTENSIONS = ['srt', 'ass', 'ssa', 'vtt', 'sub', 'idx', 'lrc'];
const TEXT_OPEN_EXTENSIONS = new Set([
  'txt', 'json', 'md', 'markdown', 'log', 'xml', 'yml', 'yaml', 'ini', 'conf', 'cfg', 'toml', 'csv', 'tsv',
  'nfo', 'js', 'mjs', 'cjs', 'ts', 'jsx', 'tsx', 'css', 'scss', 'less', 'html', 'htm', 'vue',
  'sh', 'bat', 'cmd', 'ps1', 'py', 'rb', 'java', 'c', 'cc', 'cpp', 'h', 'hpp', 'rs', 'go', 'php', 'sql',
  'properties', 'env', 'gitignore', 'editorconfig', 'strm',
  ...SUBTITLE_TEXT_EXTENSIONS,
]);

export function fileExtensionOf(record) {
  const explicit = String(pick(record, ['fileSuffix', 'extension', 'ext'], '') || '').trim().replace(/^\./, '');
  if (explicit) return explicit.toLowerCase();
  const name = String(pick(record, ['fileName', 'name'], '') || '');
  const dot = name.lastIndexOf('.');
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : '';
}

export function baseNameOf(record) {
  const name = String(pick(record, ['fileName', 'name'], '') || '');
  const dot = name.lastIndexOf('.');
  return (dot > 0 ? name.slice(0, dot) : name).toLowerCase();
}

export function openKindOf(record) {
  if (!record) return OPEN_KIND.OTHER;
  if (Number(record.resType) === 2) return OPEN_KIND.FOLDER;
  const extension = fileExtensionOf(record);
  if (VIDEO_OPEN_EXTENSIONS.has(extension)) return OPEN_KIND.VIDEO;
  if (IMAGE_OPEN_EXTENSIONS.has(extension)) return OPEN_KIND.IMAGE;
  if (AUDIO_OPEN_EXTENSIONS.has(extension)) return OPEN_KIND.AUDIO;
  if (TEXT_OPEN_EXTENSIONS.has(extension)) return OPEN_KIND.TEXT;
  return OPEN_KIND.OTHER;
}

export function browserCanPlayAudio(record) {
  return BROWSER_AUDIO_EXTENSIONS.has(fileExtensionOf(record));
}

/** 同目录歌词匹配：优先同名 .lrc，其次“同名.语言.lrc”这类前缀匹配。 */
export function findLyricsSibling(record, siblings) {
  const base = baseNameOf(record);
  if (!base) return null;
  const candidates = (Array.isArray(siblings) ? siblings : [])
    .filter((item) => Number(item?.resType) !== 2 && fileExtensionOf(item) === 'lrc');
  return candidates.find((item) => baseNameOf(item) === base)
    || candidates.find((item) => baseNameOf(item).startsWith(`${base}.`))
    || null;
}

/**
 * 解析 LRC 歌词：支持一行多个时间标签、[offset:±ms] 整体偏移，
 * 忽略增强格式的 <mm:ss.xx> 逐字标签。返回按时间升序的 { time, text }。
 */
export function parseLrc(text) {
  const lines = [];
  let offsetMs = 0;
  for (const rawLine of String(text || '').split(/\r\n|\n|\r/)) {
    const offsetMatch = rawLine.match(/^\s*\[offset:\s*([+-]?\d+)\s*\]\s*$/i);
    if (offsetMatch) {
      offsetMs = Number(offsetMatch[1]) || 0;
      continue;
    }
    const tagPattern = /\[(\d{1,3}):(\d{1,2}(?:[.:]\d{1,3})?)\]/g;
    const times = [];
    let match;
    while ((match = tagPattern.exec(rawLine)) !== null) {
      const minutes = Number(match[1]);
      const seconds = Number(match[2].replace(':', '.'));
      if (Number.isFinite(minutes) && Number.isFinite(seconds)) times.push(minutes * 60 + seconds);
    }
    if (!times.length) continue;
    const content = rawLine
      .replace(/\[[^\]]*\]/g, '')
      .replace(/<\d{1,3}:\d{1,2}(?:[.:]\d{1,3})?>/g, '')
      .trim();
    for (const time of times) lines.push({ time, text: content });
  }
  lines.sort((left, right) => left.time - right.time);
  // [offset:+500] 表示歌词整体提前 500ms 显示。
  return offsetMs
    ? lines.map((line) => ({ ...line, time: Math.max(0, line.time - offsetMs / 1000) }))
    : lines;
}

/** 二分查找当前播放进度对应的歌词行下标，无匹配返回 -1。 */
export function lrcIndexAt(lines, seconds) {
  if (!Array.isArray(lines) || !lines.length) return -1;
  let low = 0;
  let high = lines.length - 1;
  let result = -1;
  while (low <= high) {
    const middle = (low + high) >> 1;
    if (lines[middle].time <= seconds) {
      result = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }
  return result;
}

/** Web 端可用的外部播放器 URL scheme；macOS 上把 IINA 排到最前。 */
export function externalPlayerOptions(isMac = detectMac()) {
  const options = [
    { id: 'potplayer', name: 'PotPlayer', hint: 'Windows', buildUrl: (url) => `potplayer://${url}` },
    { id: 'vlc', name: 'VLC', hint: 'Windows 需先注册 vlc:// 协议', buildUrl: (url) => `vlc://${url}` },
    { id: 'iina', name: 'IINA', hint: 'macOS', buildUrl: (url) => `iina://weblink?url=${encodeURIComponent(url)}` },
    { id: 'mpv', name: 'mpv', hint: '需安装 mpv-handler', buildUrl: (url) => `mpv://${url}` },
  ];
  if (!isMac) return options;
  return [...options.filter((item) => item.id === 'iina'), ...options.filter((item) => item.id !== 'iina')];
}

function detectMac() {
  if (typeof navigator === 'undefined') return false;
  return /mac/i.test(navigator.platform || navigator.userAgent || '');
}

export function formatPlayTime(seconds) {
  const total = Math.max(0, Math.floor(Number(seconds) || 0));
  const minutes = Math.floor(total / 60);
  const remain = total % 60;
  return `${minutes}:${String(remain).padStart(2, '0')}`;
}
