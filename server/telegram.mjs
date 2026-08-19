/**
 * Telegram Bot 通知与交互渠道。
 *
 * 两种接入模式（都以 bot 身份运行，Telegram 协议限制只有 bot 能发 inline keyboard）：
 * - bot_api：HTTPS 调 Bot API（官方 api.telegram.org 或自建反代），getUpdates 长轮询，
 *   走全局代理（HTTP/SOCKS5 均可）。
 * - mtproto：api_id + api_hash + bot token，经 teleproto（GramJS 分支）以 MTProto 协议
 *   直连数据中心，仅支持 SOCKS5 代理；StringSession 持久化在 app_state。
 *
 * 出站通知：整理完成（入库）、识别失败（带重新整理 keyboard）、光鸭登录失效（带扫码按钮）、
 * Emby webhook（入库/播放/登录）。入站命令：/status /jobs /logs /update /login /help 与
 * `re <任务ID> tmdbid=…` 重新整理指令。
 */
import crypto from 'node:crypto';
import QRCode from 'qrcode';
import { fetch as undiciFetch } from 'undici';
import { createProxiedFetch, normalizeProxyUrl } from './network-preferences.mjs';

const DEFAULT_BOT_API_BASE = 'https://api.telegram.org';
const NOTIFY_CATEGORIES = ['organize', 'review', 'auth', 'emby_new', 'emby_play', 'emby_login'];
const DEFAULT_NOTIFY = Object.freeze({
  organize: true,
  review: true,
  auth: true,
  emby_new: true,
  emby_play: true,
  emby_login: true,
});
const BOT_COMMANDS = [
  { command: 'status', description: '系统状态总览' },
  { command: 'jobs', description: '最近整理任务' },
  { command: 'logs', description: '最新运行日志（默认 50 条）' },
  { command: 'update', description: '检查更新' },
  { command: 'login', description: '获取光鸭扫码登录二维码' },
  { command: 'help', description: '帮助与命令说明' },
];
const ERROR_CODE_LABELS = {
  recognition_failed: '识别失败',
  tmdb_not_found: 'TMDB 没有找到匹配条目',
  tmdb_not_configured: '尚未配置 TMDB API Key',
  tmdb_unavailable: 'TMDB 服务不可用',
  ambiguous_match: '匹配结果不唯一，需要人工确认',
  title_required: '未能从文件名解析出标题',
  episode_required: '未识别到季集号',
  video_required: '没有可整理的视频文件',
  source_missing: '云端源文件已不存在',
  source_unavailable: '云端源暂不可用',
  transfer_failed: '云端转移执行失败',
  rearchive_failed: '重新归档失败',
  completed_warning: '完成但有提示',
};
const JOB_STATUS_LABELS = {
  recognizing: '识别中',
  ready: '待执行',
  needs_review: '待人工处理',
  running: '整理中',
  completed: '已完成',
  completed_warning: '完成（有提示）',
  failed: '失败',
};
const JOB_STATUS_ICONS = {
  recognizing: '🔍',
  ready: '🟡',
  needs_review: '⚠️',
  running: '🔄',
  completed: '✅',
  completed_warning: '☑️',
  failed: '❌',
};

function cleanText(value) {
  return String(value ?? '').trim();
}
function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
function parseJsonSafe(value, fallback = null) {
  try { return value == null || value === '' ? fallback : JSON.parse(value); } catch { return fallback; }
}
export function escapeHtml(value) {
  return String(value ?? '').replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;');
}
const esc = escapeHtml;

export function parseChatIds(value) {
  return String(value ?? '')
    .split(/[\s,;，、]+/)
    .map((item) => item.trim())
    .filter((item) => /^-?\d{1,20}$/.test(item));
}

export function normalizeTelegramApiBaseUrl(value) {
  const raw = cleanText(value);
  if (!raw) return '';
  let parsed;
  try { parsed = new URL(raw); } catch { throw new Error('Telegram API 地址必须是完整的 HTTP(S) URL'); }
  if (!['http:', 'https:'].includes(parsed.protocol)) throw new Error('Telegram API 地址必须使用 HTTP 或 HTTPS');
  if (parsed.username || parsed.password || parsed.search || parsed.hash) throw new Error('Telegram API 地址不能包含账号、查询参数或片段');
  parsed.pathname = parsed.pathname.replace(/\/+$/, '');
  return parsed.toString().replace(/\/$/, '');
}

export function shortJobId(id) {
  return String(id || '').replaceAll('-', '').slice(0, 8) || '--------';
}

export function findJobByRef(jobs, ref) {
  const normalized = cleanText(ref).toLowerCase().replaceAll('-', '');
  if (!normalized) return { error: '请提供任务 ID（可在 /jobs 或失败通知中查看）' };
  if (!/^[0-9a-f]{4,32}$/.test(normalized)) return { error: `任务 ID 格式不正确：${ref}` };
  const matches = (jobs || []).filter((job) => String(job.id || '').toLowerCase().replaceAll('-', '').startsWith(normalized));
  if (!matches.length) return { error: `没有找到任务 ${ref}，可先用 /jobs 查看最近任务` };
  if (matches.length > 1) return { error: `任务 ID 前缀 ${ref} 匹配到 ${matches.length} 个任务，请使用更长的前缀` };
  return { job: matches[0] };
}

/** 解析 `re` 之后的覆盖参数（也用于 ForceReply 回复内容）。 */
export function parseOverrideTokens(tokens) {
  const input = {};
  for (const raw of tokens || []) {
    const token = cleanText(raw);
    if (!token) continue;
    const lower = token.toLowerCase();
    const eq = token.indexOf('=');
    if (eq > 0) {
      const key = lower.slice(0, eq);
      const value = token.slice(eq + 1).trim();
      if (['tmdbid', 'tmdb', 'tmdb_id', 'id'].includes(key)) {
        if (!/^\d{1,10}$/.test(value)) return { error: `TMDB ID 必须是数字：${token}` };
        input.tmdb_id = Number(value);
      } else if (['s', 'season'].includes(key)) {
        if (!/^\d{1,3}$/.test(value)) return { error: `季号必须是数字：${token}` };
        input.season = Number(value);
      } else if (['e', 'ep', 'episode'].includes(key)) {
        if (!/^\d{1,4}$/.test(value)) return { error: `集号必须是数字：${token}` };
        input.episode = Number(value);
      } else if (['type', 't', 'media', 'media_type'].includes(key)) {
        const mapped = { tv: 'tv', movie: 'movie', 剧集: 'tv', 电视剧: 'tv', 电影: 'movie' }[value.toLowerCase()];
        if (!mapped) return { error: `类型只支持 tv 或 movie：${token}` };
        input.media_type = mapped;
      } else if (['title', 'name'].includes(key)) {
        if (!value) return { error: '标题不能为空' };
        input.title = value;
      } else if (['y', 'year'].includes(key)) {
        if (!/^\d{4}$/.test(value)) return { error: `年份必须是 4 位数字：${token}` };
        input.year = Number(value);
      } else {
        return { error: `无法识别参数：${token}（支持 tmdbid= s= e= type= title= year=）` };
      }
      continue;
    }
    if (['tv', '剧集', '电视剧'].includes(lower)) { input.media_type = 'tv'; continue; }
    if (['movie', '电影'].includes(lower)) { input.media_type = 'movie'; continue; }
    if (/^s\d{1,3}$/.test(lower)) { input.season = Number(lower.slice(1)); continue; }
    if (/^e\d{1,4}$/.test(lower)) { input.episode = Number(lower.slice(1)); continue; }
    if (/^\d{1,10}$/.test(token)) { input.tmdb_id = Number(token); continue; }
    return { error: `无法识别参数：${token}（支持 tmdbid= s= e= tv/movie 或直接给出 TMDB 数字 ID）` };
  }
  return { input };
}

/** 解析 `re <任务ID> tmdbid=12345 [tv|movie] [s=1] [e=2]` 重新整理命令。 */
export function parseReCommand(text) {
  const trimmed = cleanText(text);
  if (!trimmed) return null;
  const tokens = trimmed.split(/\s+/);
  const head = tokens[0].toLowerCase().replace(/@[\w_]+$/, '');
  if (head !== 're' && head !== '/re') return null;
  if (tokens.length < 2) return { error: '用法：re <任务ID> tmdbid=12345 [tv|movie] [s=1] [e=2]' };
  const parsed = parseOverrideTokens(tokens.slice(2));
  if (parsed.error) return { error: parsed.error };
  return { jobRef: tokens[1], input: parsed.input };
}

function sourceBaseName(job) {
  const parts = String(job?.source_path || '').replaceAll('\\', '/').split('/').filter(Boolean);
  return parts[parts.length - 1] || String(job?.source_path || '') || '未知来源';
}

export function describeJobTitle(job) {
  const title = cleanText(job?.query_title) || sourceBaseName(job);
  const year = job?.query_year ? ` (${job.query_year})` : '';
  const type = job?.media_type === 'tv' ? ' · 剧集' : job?.media_type === 'movie' ? ' · 电影' : '';
  const season = job?.media_type === 'tv' && job?.season != null ? ` S${String(job.season).padStart(2, '0')}` : '';
  return `${title}${year}${type}${season}`;
}

function errorCodeLabel(code) {
  const normalized = cleanText(code);
  return ERROR_CODE_LABELS[normalized] || normalized || '未知原因';
}

function jobTargetPath(job, mapping) {
  const relative = cleanText(job?.preview?.share_relative_path);
  const base = cleanText(mapping?.target_path).replace(/\/+$/, '');
  if (!relative) return base;
  if (!base) return relative;
  return `${base}/${relative.replace(/^\/+/, '')}`;
}

/** 整理完成（入库）通知文本。 */
export function formatOrganizeDone(job, mapping = null) {
  const lines = [`✅ <b>入库完成</b>：${esc(describeJobTitle(job))}`];
  lines.push(`📁 来源：${esc(sourceBaseName(job))}`);
  const target = jobTargetPath(job, mapping);
  if (target) lines.push(`🎯 目标：${esc(target)}`);
  if (cleanText(job?.message)) lines.push(`💬 ${esc(job.message)}`);
  const share = job?.result?.share?.share_url;
  if (cleanText(share)) lines.push(`🔗 ${esc(share)}`);
  lines.push(`🆔 <code>${esc(shortJobId(job?.id))}</code>`);
  return lines.join('\n');
}

/** 识别失败 / 整理失败通知，附重新整理 keyboard。 */
export function formatReviewNeeded(job) {
  const failed = job?.status === 'failed';
  const heading = failed ? '❌ <b>整理失败</b>' : '⚠️ <b>识别待处理</b>';
  const shortId = shortJobId(job?.id);
  const lines = [`${heading}：${esc(describeJobTitle(job))}`];
  lines.push(`📁 来源：${esc(job?.source_path || sourceBaseName(job))}`);
  lines.push(`⛔ 原因：${esc(errorCodeLabel(job?.error_code))}`);
  if (cleanText(job?.message)) lines.push(`💬 ${esc(job.message)}`);
  lines.push(`🆔 <code>${esc(shortId)}</code>`);
  lines.push(`↩️ 手动指定：<code>re ${esc(shortId)} tmdbid=12345 tv s=1</code>`);
  return {
    text: lines.join('\n'),
    keyboard: [
      [
        { text: '🔁 重新识别', data: `retry:${job?.id}` },
        { text: '▶️ 重新整理', data: `run:${job?.id}` },
      ],
      [{ text: '🔎 填写 TMDB ID', data: `ask:${job?.id}` }],
    ],
  };
}

function embyItemLabel(item = {}) {
  const name = cleanText(item.Name);
  if (cleanText(item.Type) === 'Episode') {
    const series = cleanText(item.SeriesName);
    const season = item.ParentIndexNumber != null ? `S${String(item.ParentIndexNumber).padStart(2, '0')}` : '';
    const episode = item.IndexNumber != null ? `E${String(item.IndexNumber).padStart(2, '0')}` : '';
    return [series, `${season}${episode}`.trim(), name].filter(Boolean).join(' ');
  }
  if (!name) return '';
  return item.ProductionYear ? `${name} (${item.ProductionYear})` : name;
}

function playbackProgressLabel(payload = {}) {
  const position = Number(payload.PlaybackInfo?.PositionTicks ?? payload.Session?.PlayState?.PositionTicks ?? 0);
  const total = Number(payload.Item?.RunTimeTicks ?? 0);
  if (!Number.isFinite(position) || position <= 0) return '';
  const seconds = Math.floor(position / 10_000_000);
  const minutes = Math.floor(seconds / 60);
  const clock = `${String(minutes).padStart(2, '0')}:${String(seconds % 60).padStart(2, '0')}`;
  if (!Number.isFinite(total) || total <= 0) return clock;
  const percent = Math.max(0, Math.min(100, Math.round((position / total) * 100)));
  return `${clock}（${percent}%）`;
}

/** 解析 Emby webhook 请求体：兼容 JSON、multipart/form-data 的 data 字段与 urlencoded。 */
export function parseEmbyWebhookBody(contentType, buffer) {
  const type = String(contentType || '').toLowerCase();
  const body = Buffer.isBuffer(buffer) ? buffer : Buffer.from(String(buffer ?? ''), 'utf8');
  try {
    if (type.includes('multipart/form-data')) {
      const boundary = /boundary="?([^";,]+)"?/i.exec(String(contentType || ''))?.[1];
      if (!boundary) return null;
      const fields = parseMultipartTextFields(body, boundary);
      return fields.data ? JSON.parse(fields.data) : null;
    }
    if (type.includes('application/x-www-form-urlencoded')) {
      const data = new URLSearchParams(body.toString('utf8')).get('data');
      return data ? JSON.parse(data) : null;
    }
    const text = body.toString('utf8').trim();
    if (text.startsWith('{')) return JSON.parse(text);
  } catch {
    return null;
  }
  return null;
}

function parseMultipartTextFields(buffer, boundary) {
  const delimiter = Buffer.from(`--${boundary}`);
  const fields = {};
  let index = buffer.indexOf(delimiter);
  while (index !== -1) {
    const start = index + delimiter.length;
    if (buffer.subarray(start, start + 2).toString('latin1') === '--') break;
    const next = buffer.indexOf(delimiter, start);
    const part = buffer.subarray(start, next === -1 ? buffer.length : next);
    const headerEnd = part.indexOf('\r\n\r\n');
    if (headerEnd !== -1) {
      const headerText = part.subarray(0, headerEnd).toString('utf8');
      const nameMatch = /name="([^"]+)"/i.exec(headerText);
      const isFile = /filename="/i.test(headerText);
      if (nameMatch && !isFile) {
        let value = part.subarray(headerEnd + 4);
        if (value.subarray(-2).toString('latin1') === '\r\n') value = value.subarray(0, -2);
        if (value.length <= 256 * 1024) fields[nameMatch[1]] = value.toString('utf8');
      }
    }
    if (next === -1) break;
    index = next;
  }
  return fields;
}

/** 把 Emby webhook 事件映射为通知类别与文本；未知事件返回 null。 */
export function describeEmbyEvent(payload) {
  if (!payload || typeof payload !== 'object') return null;
  const event = cleanText(payload.Event ?? payload.event).toLowerCase();
  if (!event) return null;
  const item = payload.Item || {};
  const user = payload.User || {};
  const session = payload.Session || {};
  const server = cleanText(payload.Server?.Name);
  const itemLabel = embyItemLabel(item) || cleanText(payload.Title) || '未知条目';
  if (event === 'library.new') {
    const lines = [`📥 <b>Emby 入库</b>${server ? `（${esc(server)}）` : ''}`, esc(itemLabel)];
    if (cleanText(item.Path)) lines.push(`📁 ${esc(item.Path)}`);
    if (cleanText(item.Overview)) lines.push(esc(cleanText(item.Overview).slice(0, 200)));
    return { category: 'emby_new', text: lines.join('\n') };
  }
  if (event.startsWith('playback.')) {
    const action = { start: '开始播放', stop: '停止播放', pause: '暂停播放', unpause: '继续播放', progress: '播放进度' }[event.slice('playback.'.length)] || event;
    const icon = { 开始播放: '▶️', 停止播放: '⏹️', 暂停播放: '⏸️', 继续播放: '▶️' }[action] || '▶️';
    const device = [cleanText(session.DeviceName), cleanText(session.Client)].filter(Boolean).join(' · ');
    const progress = playbackProgressLabel(payload);
    const lines = [`${icon} <b>Emby ${esc(action)}</b>${server ? `（${esc(server)}）` : ''}`];
    if (cleanText(user.Name)) lines.push(`👤 ${esc(user.Name)}`);
    lines.push(`🎬 ${esc(itemLabel)}`);
    if (device) lines.push(`📱 ${esc(device)}`);
    if (progress) lines.push(`⏳ ${esc(progress)}`);
    return { category: 'emby_play', text: lines.join('\n') };
  }
  if (event === 'user.authenticated' || event === 'user.authenticationfailed') {
    const failed = event.endsWith('failed');
    const lines = [`${failed ? '🚨' : '🔐'} <b>Emby ${failed ? '登录失败' : '用户登录'}</b>${server ? `（${esc(server)}）` : ''}`];
    const who = cleanText(user.Name) || cleanText(payload.Title);
    if (who) lines.push(`👤 ${esc(who)}`);
    if (cleanText(session.RemoteEndPoint)) lines.push(`🌐 ${esc(session.RemoteEndPoint)}`);
    const device = [cleanText(session.DeviceName), cleanText(session.Client)].filter(Boolean).join(' · ');
    if (device) lines.push(`📱 ${esc(device)}`);
    return { category: 'emby_login', text: lines.join('\n') };
  }
  return null;
}

/** 长文本按行拆分为不超过 limit 字符的多段（Telegram 单条消息上限 4096）。 */
export function chunkLines(lines, limit = 3500) {
  const chunks = [];
  let current = '';
  for (const line of lines) {
    const piece = String(line).length > limit ? `${String(line).slice(0, limit - 1)}…` : String(line);
    if (current && current.length + piece.length + 1 > limit) {
      chunks.push(current);
      current = piece;
    } else {
      current = current ? `${current}\n${piece}` : piece;
    }
  }
  if (current) chunks.push(current);
  return chunks;
}

// ---------------------------------------------------------------------------
// Bot API 传输（HTTPS + getUpdates 长轮询）
// ---------------------------------------------------------------------------

export function createBotApiTransport({ botToken, apiBaseUrl = '', fetchImpl = undiciFetch, getProxyUrl = () => '', log = () => {} }) {
  const base = `${(cleanText(apiBaseUrl) || DEFAULT_BOT_API_BASE).replace(/\/+$/, '')}/bot${cleanText(botToken)}`;
  let running = false;
  let pollController = null;

  async function call(method, params = {}, { timeoutMs = 30_000, signal = null } = {}) {
    const proxied = createProxiedFetch(getProxyUrl(), fetchImpl);
    const signals = [AbortSignal.timeout(timeoutMs)];
    if (signal) signals.push(signal);
    const response = await proxied(`${base}/${method}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(params),
      signal: AbortSignal.any(signals),
    });
    const payload = await response.json().catch(() => ({}));
    if (!payload.ok) {
      const error = new Error(`Telegram ${method} 失败：${payload.description || `HTTP ${response.status}`}`);
      error.status = Number(payload.error_code || response.status || 0);
      throw error;
    }
    return payload.result;
  }

  function toReplyMarkup({ keyboard, forceReply } = {}) {
    if (Array.isArray(keyboard) && keyboard.length) {
      return { inline_keyboard: keyboard.map((row) => row.map((button) => ({ text: button.text, callback_data: button.data }))) };
    }
    if (forceReply) return { force_reply: true, selective: true };
    return undefined;
  }

  async function dispatchUpdate(update, handlers) {
    if (update.message) {
      const message = update.message;
      const text = cleanText(message.text);
      if (!text) return;
      await handlers.onMessage({
        chatId: String(message.chat?.id ?? ''),
        senderId: String(message.from?.id ?? ''),
        text,
        messageId: Number(message.message_id),
        replyToId: message.reply_to_message?.message_id != null ? Number(message.reply_to_message.message_id) : null,
      });
      return;
    }
    if (update.callback_query) {
      const query = update.callback_query;
      await handlers.onCallback({
        chatId: String(query.message?.chat?.id ?? query.from?.id ?? ''),
        senderId: String(query.from?.id ?? ''),
        data: String(query.data ?? ''),
        messageId: query.message?.message_id != null ? Number(query.message.message_id) : null,
        answer: async (text) => {
          try { await call('answerCallbackQuery', { callback_query_id: query.id, text: text ? String(text).slice(0, 190) : undefined }); } catch {}
        },
      });
    }
  }

  return {
    kind: 'bot_api',
    async getMe() {
      const me = await call('getMe');
      return { username: cleanText(me?.username) };
    },
    start(handlers) {
      running = true;
      void (async () => {
        let offset = 0;
        let failureDelay = 5_000;
        while (running) {
          pollController = new AbortController();
          try {
            const updates = await call(
              'getUpdates',
              { timeout: 50, offset, allowed_updates: ['message', 'callback_query'] },
              { timeoutMs: 65_000, signal: pollController.signal },
            );
            failureDelay = 5_000;
            for (const update of updates || []) {
              offset = Math.max(offset, Number(update.update_id) + 1);
              try { await dispatchUpdate(update, handlers); }
              catch (error) { log('warning', `Telegram 消息处理失败：${error.message}`); }
            }
          } catch (error) {
            if (!running) return;
            if ([401, 404].includes(error.status)) {
              running = false;
              log('error', `Telegram Bot Token 无效，已停止轮询：${error.message}`);
              handlers.onFatal?.(error);
              return;
            }
            if (error.status === 409) {
              log('warning', 'Telegram getUpdates 冲突（409）：同一个 Bot Token 正在其他实例轮询，请只在一端启用');
            } else {
              log('warning', `Telegram 轮询失败，${Math.round(failureDelay / 1000)} 秒后重试：${error.message}`);
            }
            await sleep(failureDelay);
            failureDelay = Math.min(failureDelay * 2, 300_000);
          }
        }
      })();
    },
    async stop() {
      running = false;
      try { pollController?.abort(); } catch {}
    },
    async sendMessage(chatId, text, { keyboard, forceReply } = {}) {
      const result = await call('sendMessage', {
        chat_id: chatId,
        text,
        parse_mode: 'HTML',
        link_preview_options: { is_disabled: true },
        reply_markup: toReplyMarkup({ keyboard, forceReply }),
      });
      return Number(result.message_id);
    },
    async sendPhoto(chatId, buffer, { caption, filename = 'photo.png' } = {}) {
      const form = new FormData();
      form.append('chat_id', String(chatId));
      if (caption) form.append('caption', caption);
      form.append('parse_mode', 'HTML');
      form.append('photo', new Blob([buffer], { type: 'image/png' }), filename);
      const proxied = createProxiedFetch(getProxyUrl(), fetchImpl);
      const response = await proxied(`${base}/sendPhoto`, { method: 'POST', body: form, signal: AbortSignal.timeout(60_000) });
      const payload = await response.json().catch(() => ({}));
      if (!payload.ok) throw new Error(`Telegram sendPhoto 失败：${payload.description || `HTTP ${response.status}`}`);
      return Number(payload.result?.message_id);
    },
    async editMessage(chatId, messageId, text, { keyboard } = {}) {
      await call('editMessageText', {
        chat_id: chatId,
        message_id: messageId,
        text,
        parse_mode: 'HTML',
        reply_markup: toReplyMarkup({ keyboard }),
      });
    },
    async setCommands(commands) {
      await call('setMyCommands', { commands });
    },
  };
}

// ---------------------------------------------------------------------------
// MTProto 传输（teleproto / GramJS 分支，bot 身份登录）
// ---------------------------------------------------------------------------

export async function createMtprotoTransport({ apiId, apiHash, botToken, sessionString = '', saveSession = () => {}, getProxyUrl = () => '', log = () => {} }) {
  const numericApiId = Number(apiId);
  if (!Number.isInteger(numericApiId) || numericApiId <= 0) throw new Error('MTProto 模式需要有效的 API ID');
  if (!cleanText(apiHash)) throw new Error('MTProto 模式需要 API Hash');
  if (!cleanText(botToken)) throw new Error('MTProto 模式需要 Bot Token');
  let modules;
  try {
    const [core, sessions, buttonModule, eventsModule, uploadsModule, bigIntModule] = await Promise.all([
      import('teleproto'),
      import('teleproto/sessions'),
      import('teleproto/tl/custom/button'),
      import('teleproto/events'),
      import('teleproto/client/uploads'),
      import('big-integer'),
    ]);
    modules = {
      TelegramClient: core.TelegramClient,
      Api: core.Api,
      StringSession: sessions.StringSession,
      Button: buttonModule.Button,
      NewMessage: eventsModule.NewMessage,
      CallbackQuery: eventsModule.CallbackQuery,
      CustomFile: uploadsModule.CustomFile,
      bigInt: bigIntModule.default,
    };
  } catch (error) {
    throw new Error(`MTProto 依赖（teleproto）加载失败，请重新安装依赖：${error.message}`);
  }
  const { TelegramClient, Api, StringSession, Button, NewMessage, CallbackQuery, CustomFile, bigInt } = modules;

  let proxy;
  const proxyUrl = normalizeProxyUrl(getProxyUrl() || '');
  if (proxyUrl) {
    const parsed = new URL(proxyUrl);
    if (parsed.protocol.startsWith('socks')) {
      proxy = {
        ip: parsed.hostname,
        port: Number(parsed.port || 1080),
        socksType: 5,
        username: parsed.username ? decodeURIComponent(parsed.username) : undefined,
        password: parsed.password ? decodeURIComponent(parsed.password) : undefined,
        timeout: 10,
      };
    } else {
      log('warning', 'MTProto 模式仅支持 SOCKS5 代理，已忽略当前 HTTP 代理并尝试直连');
    }
  }

  const client = new TelegramClient(new StringSession(cleanText(sessionString)), numericApiId, cleanText(apiHash), {
    connectionRetries: 3,
    retryDelay: 2_000,
    autoReconnect: true,
    requestRetries: 2,
    proxy,
  });
  try { client.setLogLevel('error'); } catch {}
  await client.start({ botAuthToken: cleanText(botToken) });
  try { saveSession(String(client.session.save() || '')); } catch {}

  /** bot 对已互动过的对象可以用 access_hash=0 兜底构造 InputPeer。 */
  async function entity(chatId) {
    const raw = cleanText(chatId);
    try {
      return await client.getInputEntity(/^-?\d+$/.test(raw) ? bigInt(raw) : raw);
    } catch {
      if (!/^-?\d+$/.test(raw)) throw new Error(`无法解析 Telegram 会话：${raw}`);
      if (raw.startsWith('-100')) return new Api.InputPeerChannel({ channelId: bigInt(raw.slice(4)), accessHash: bigInt.zero });
      if (raw.startsWith('-')) return new Api.InputPeerChat({ chatId: bigInt(raw.slice(1)) });
      return new Api.InputPeerUser({ userId: bigInt(raw), accessHash: bigInt.zero });
    }
  }

  function toButtons({ keyboard, forceReply } = {}) {
    if (Array.isArray(keyboard) && keyboard.length) {
      return keyboard.map((row) => row.map((button) => Button.inline(button.text, Buffer.from(button.data, 'utf8'))));
    }
    if (forceReply) return Button.forceReply(true, true);
    return undefined;
  }

  return {
    kind: 'mtproto',
    async getMe() {
      const me = await client.getMe();
      return { username: cleanText(me?.username) };
    },
    start(handlers) {
      client.addEventHandler(async (event) => {
        try {
          const message = event.message;
          const text = cleanText(message?.message);
          if (!text || message.out) return;
          await handlers.onMessage({
            chatId: String(message.chatId ?? ''),
            senderId: String(message.senderId ?? ''),
            text,
            messageId: Number(message.id),
            replyToId: message.replyTo?.replyToMsgId != null ? Number(message.replyTo.replyToMsgId) : null,
          });
        } catch (error) {
          log('warning', `Telegram 消息处理失败：${error.message}`);
        }
      }, new NewMessage({ incoming: true }));
      client.addEventHandler(async (event) => {
        try {
          await handlers.onCallback({
            chatId: String(event.chatId ?? event.query?.userId ?? ''),
            senderId: String(event.senderId ?? event.query?.userId ?? ''),
            data: event.data ? Buffer.from(event.data).toString('utf8') : '',
            messageId: event.messageId != null ? Number(event.messageId) : null,
            answer: async (text) => {
              try { await event.answer({ message: text ? String(text).slice(0, 190) : undefined }); } catch {}
            },
          });
        } catch (error) {
          log('warning', `Telegram 回调处理失败：${error.message}`);
        }
      }, new CallbackQuery({}));
    },
    async stop() {
      try { await client.disconnect(); } catch {}
      try { await client.destroy(); } catch {}
    },
    async sendMessage(chatId, text, { keyboard, forceReply } = {}) {
      const sent = await client.sendMessage(await entity(chatId), {
        message: text,
        parseMode: 'html',
        buttons: toButtons({ keyboard, forceReply }),
        linkPreview: false,
      });
      return Number(sent.id);
    },
    async sendPhoto(chatId, buffer, { caption, filename = 'photo.png' } = {}) {
      const sent = await client.sendFile(await entity(chatId), {
        file: new CustomFile(filename, buffer.length, filename, buffer),
        caption,
        parseMode: 'html',
      });
      return Number(sent?.id);
    },
    async editMessage(chatId, messageId, text, { keyboard } = {}) {
      await client.editMessage(await entity(chatId), {
        message: Number(messageId),
        text,
        parseMode: 'html',
        buttons: toButtons({ keyboard }),
      });
    },
    async setCommands(commands) {
      await client.invoke(new Api.bots.SetBotCommands({
        scope: new Api.BotCommandScopeDefault(),
        langCode: '',
        commands: commands.map((item) => new Api.BotCommand({ command: item.command, description: item.description })),
      }));
    },
  };
}

// ---------------------------------------------------------------------------
// Telegram 服务：配置、生命周期、通知分发与命令路由
// ---------------------------------------------------------------------------

export function createTelegramService({
  database,
  env = process.env,
  fetchImpl = undiciFetch,
  getProxyUrl = () => '',
  logBuffer,
  version = '0.0.0',
  platform = 'Web',
  runtime = {},
}) {
  function readState(key) {
    return database.prepare('SELECT value FROM app_state WHERE key = ?').get(key)?.value;
  }
  function writeState(key, value) {
    database.prepare('INSERT INTO app_state (key, value, updated_at) VALUES (?, ?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at')
      .run(key, String(value ?? ''), Math.floor(Date.now() / 1000));
  }
  function log(level, message) {
    logBuffer?.push(level, message);
    if (level === 'error' || level === 'warning') console.warn(`[telegram] ${message}`);
    else console.log(`[telegram] ${message}`);
  }

  let current = null;
  let connected = false;
  let botUsername = '';
  let lastError = null;
  let generation = 0;
  let sendChain = Promise.resolve();
  let authExpiredNotified = false;
  let lastLoggedIn = Boolean(runtime.isLoggedIn?.());
  let loginFlow = null;
  const pendingTmdbPrompts = new Map();

  const envFlag = (value) => ['1', 'true', 'yes', 'on'].includes(cleanText(value).toLowerCase());

  function effectiveSettings() {
    const envEnabled = cleanText(env.TELEGRAM_ENABLED);
    const enabled = envEnabled ? envFlag(envEnabled) : readState('telegram_enabled') === 'true';
    const mode = (cleanText(env.TELEGRAM_MODE) || cleanText(readState('telegram_mode')) || 'bot_api').toLowerCase() === 'mtproto' ? 'mtproto' : 'bot_api';
    const botToken = cleanText(env.TELEGRAM_BOT_TOKEN) || cleanText(readState('telegram_bot_token'));
    let apiBaseUrl = '';
    try { apiBaseUrl = normalizeTelegramApiBaseUrl(cleanText(env.TELEGRAM_API_BASE_URL) || cleanText(readState('telegram_api_base_url'))); } catch {}
    const apiId = cleanText(env.TELEGRAM_API_ID) || cleanText(readState('telegram_api_id'));
    const apiHash = cleanText(env.TELEGRAM_API_HASH) || cleanText(readState('telegram_api_hash'));
    const chatIds = parseChatIds(cleanText(env.TELEGRAM_CHAT_ID) || cleanText(readState('telegram_chat_id')));
    const notify = { ...DEFAULT_NOTIFY, ...(parseJsonSafe(readState('telegram_notify'), {}) || {}) };
    const configured = mode === 'mtproto' ? Boolean(botToken && apiId && apiHash) : Boolean(botToken);
    return {
      enabled,
      mode,
      bot_token: botToken,
      api_base_url: apiBaseUrl,
      api_id: apiId,
      api_hash: apiHash,
      chat_ids: chatIds,
      notify,
      configured,
      enabled_managed_by_environment: Boolean(envEnabled),
      mode_managed_by_environment: Boolean(cleanText(env.TELEGRAM_MODE)),
      bot_token_managed_by_environment: Boolean(cleanText(env.TELEGRAM_BOT_TOKEN)),
      api_base_url_managed_by_environment: Boolean(cleanText(env.TELEGRAM_API_BASE_URL)),
      api_id_managed_by_environment: Boolean(cleanText(env.TELEGRAM_API_ID)),
      api_hash_managed_by_environment: Boolean(cleanText(env.TELEGRAM_API_HASH)),
      chat_id_managed_by_environment: Boolean(cleanText(env.TELEGRAM_CHAT_ID)),
    };
  }

  function webhookSecret() {
    let secret = cleanText(readState('telegram_emby_webhook_secret'));
    if (!secret) {
      secret = crypto.randomBytes(16).toString('hex');
      writeState('telegram_emby_webhook_secret', secret);
    }
    return secret;
  }

  function publicSettings() {
    const settings = effectiveSettings();
    return {
      enabled: settings.enabled,
      mode: settings.mode,
      chat_id: settings.chat_ids.join(','),
      api_base_url: settings.api_base_url,
      api_id: settings.api_id,
      bot_token_configured: Boolean(settings.bot_token),
      api_hash_configured: Boolean(settings.api_hash),
      configured: settings.configured,
      notify: NOTIFY_CATEGORIES.reduce((result, key) => { result[key] = settings.notify[key] !== false; return result; }, {}),
      enabled_managed_by_environment: settings.enabled_managed_by_environment,
      mode_managed_by_environment: settings.mode_managed_by_environment,
      bot_token_managed_by_environment: settings.bot_token_managed_by_environment,
      api_base_url_managed_by_environment: settings.api_base_url_managed_by_environment,
      api_id_managed_by_environment: settings.api_id_managed_by_environment,
      api_hash_managed_by_environment: settings.api_hash_managed_by_environment,
      chat_id_managed_by_environment: settings.chat_id_managed_by_environment,
      connected,
      bot_username: botUsername,
      last_error: lastError,
      webhook: {
        secret: webhookSecret(),
        path: `/webhooks/emby?token=${webhookSecret()}`,
        gateway_path: `/guangya/webhooks/emby?token=${webhookSecret()}`,
      },
    };
  }

  function updateSettings(input = {}) {
    const before = effectiveSettings();
    if (typeof input.enabled === 'boolean') writeState('telegram_enabled', String(input.enabled));
    if (input.mode !== undefined) {
      const mode = cleanText(input.mode).toLowerCase();
      if (!['bot_api', 'mtproto'].includes(mode)) throw new Error('接入模式只支持 bot_api 或 mtproto');
      writeState('telegram_mode', mode);
    }
    if (typeof input.bot_token === 'string') {
      const value = input.bot_token.trim();
      if (value === 'off') writeState('telegram_bot_token', '');
      else if (value) {
        if (!/^\d+:[\w-]{20,}$/.test(value)) throw new Error('Bot Token 格式不正确（应为 123456:ABC-DEF… 形式）');
        writeState('telegram_bot_token', value);
      }
    }
    if (typeof input.api_base_url === 'string') {
      writeState('telegram_api_base_url', normalizeTelegramApiBaseUrl(input.api_base_url));
    }
    if (input.api_id !== undefined) {
      const value = cleanText(input.api_id);
      if (value && !/^\d{1,12}$/.test(value)) throw new Error('API ID 必须是数字');
      writeState('telegram_api_id', value);
    }
    if (typeof input.api_hash === 'string') {
      const value = input.api_hash.trim();
      if (value === 'off') writeState('telegram_api_hash', '');
      else if (value) {
        if (!/^[0-9a-f]{32}$/i.test(value)) throw new Error('API Hash 格式不正确（32 位十六进制）');
        writeState('telegram_api_hash', value);
      }
    }
    if (typeof input.chat_id === 'string') {
      const tokens = input.chat_id.split(/[\s,;，、]+/).map((item) => item.trim()).filter(Boolean);
      const invalid = tokens.filter((item) => !/^-?\d{1,20}$/.test(item));
      if (invalid.length) throw new Error(`Chat ID 必须是数字（可给机器人发送 /start 获取）：${invalid.join(', ')}`);
      writeState('telegram_chat_id', tokens.join(','));
    }
    if (input.notify && typeof input.notify === 'object') {
      const merged = { ...DEFAULT_NOTIFY, ...(parseJsonSafe(readState('telegram_notify'), {}) || {}) };
      for (const key of NOTIFY_CATEGORIES) {
        if (typeof input.notify[key] === 'boolean') merged[key] = input.notify[key];
      }
      writeState('telegram_notify', JSON.stringify(merged));
    }
    if (input.regenerate_webhook_secret === true) {
      writeState('telegram_emby_webhook_secret', crypto.randomBytes(16).toString('hex'));
    }
    const after = effectiveSettings();
    if (after.enabled && !after.configured) {
      // 回滚启用状态，避免留下“已启用但未配置”的中间态。
      if (!after.enabled_managed_by_environment) writeState('telegram_enabled', String(before.enabled));
      throw new Error(after.mode === 'mtproto'
        ? '启用 MTProto 模式需要填写 API ID、API Hash 和 Bot Token'
        : '启用前请先填写 Bot Token');
    }
    // 凭据变化会使旧 MTProto 会话失效，主动清掉避免用错账号。
    if (before.bot_token !== after.bot_token || before.api_id !== after.api_id || before.api_hash !== after.api_hash) {
      writeState('telegram_mtproto_session', '');
    }
    void restart();
    return publicSettings();
  }

  async function buildTransport(settings) {
    if (settings.mode === 'mtproto') {
      return createMtprotoTransport({
        apiId: settings.api_id,
        apiHash: settings.api_hash,
        botToken: settings.bot_token,
        sessionString: cleanText(readState('telegram_mtproto_session')),
        saveSession: (value) => writeState('telegram_mtproto_session', value),
        getProxyUrl,
        log,
      });
    }
    return createBotApiTransport({
      botToken: settings.bot_token,
      apiBaseUrl: settings.api_base_url,
      fetchImpl,
      getProxyUrl,
      log,
    });
  }

  function startIfConfigured() {
    const settings = effectiveSettings();
    if (!settings.enabled || !settings.configured) return;
    const gen = generation;
    void (async () => {
      let delay = 5_000;
      while (gen === generation) {
        let transport = null;
        try {
          transport = await buildTransport(settings);
          const me = await transport.getMe();
          if (gen !== generation) { await transport.stop?.().catch?.(() => {}); return; }
          current = transport;
          botUsername = me.username || '';
          connected = true;
          lastError = null;
          transport.start({
            onMessage: (message) => onMessage(message),
            onCallback: (query) => onCallback(query),
            onFatal: (error) => {
              connected = false;
              lastError = error.message;
              runtime.pushStatus?.('warning', `Telegram Bot 已停止：${error.message}`);
            },
          });
          log('info', `Telegram Bot 已连接${botUsername ? `：@${botUsername}` : ''}（${settings.mode === 'mtproto' ? 'MTProto' : 'Bot API'} 模式）`);
          runtime.pushStatus?.('success', `Telegram Bot 已连接${botUsername ? `：@${botUsername}` : ''}`);
          void transport.setCommands(BOT_COMMANDS).catch((error) => log('warning', `注册 Telegram 命令菜单失败：${error.message}`));
          return;
        } catch (error) {
          if (transport) { try { await transport.stop(); } catch {} }
          if (gen !== generation) return;
          connected = false;
          lastError = error.message;
          log('warning', `Telegram 连接失败，${Math.round(delay / 1000)} 秒后重试：${error.message}`);
          await sleep(delay);
          delay = Math.min(delay * 2, 600_000);
        }
      }
    })();
  }

  async function restart() {
    generation += 1;
    const previous = current;
    current = null;
    connected = false;
    botUsername = '';
    lastError = null;
    if (loginFlow) { clearTimeout(loginFlow.timer); loginFlow = null; }
    pendingTmdbPrompts.clear();
    if (previous) { try { await previous.stop(); } catch {} }
    startIfConfigured();
  }

  function enqueueSend(task) {
    const next = sendChain.then(task, task);
    sendChain = next.catch(() => {});
    return next;
  }

  function sendTo(chatId, text, options = {}) {
    return enqueueSend(async () => {
      if (!current) throw new Error('Telegram Bot 尚未连接');
      return current.sendMessage(chatId, text, options);
    }).catch((error) => {
      log('warning', `Telegram 消息发送失败：${error.message}`);
      return null;
    });
  }

  function notify(category, text, { keyboard, photo } = {}) {
    const settings = effectiveSettings();
    if (!settings.enabled || !settings.configured) return;
    if (category && settings.notify[category] === false) return;
    const chatId = settings.chat_ids[0];
    if (!chatId) return;
    void enqueueSend(async () => {
      if (!current) throw new Error('Telegram Bot 尚未连接');
      if (photo?.buffer) {
        try {
          await current.sendPhoto(chatId, photo.buffer, { caption: text, filename: photo.filename || 'photo.jpg' });
          return;
        } catch (error) {
          log('warning', `Telegram 图片发送失败，回退为文本：${error.message}`);
        }
      }
      await current.sendMessage(chatId, text, { keyboard });
    }).catch((error) => {
      log('warning', `Telegram 通知发送失败：${error.message}`);
    });
  }

  async function fetchImageBuffer(url) {
    if (!cleanText(url)) return null;
    try {
      const proxied = createProxiedFetch(getProxyUrl(), fetchImpl);
      const response = await proxied(url, { signal: AbortSignal.timeout(15_000) });
      if (!response.ok) return null;
      const bytes = Buffer.from(await response.arrayBuffer());
      if (!bytes.length || bytes.length > 8 * 1024 * 1024) return null;
      return bytes;
    } catch {
      return null;
    }
  }

  // ------------------------------------------------------------------
  // 事件观察（由 server 的 publish 管道转发）
  // ------------------------------------------------------------------

  function observeEvent(payload) {
    try {
      if (!payload || typeof payload !== 'object') return;
      if (payload.type === 'state') {
        const loggedIn = Boolean(payload.state?.logged_in);
        if (loggedIn && !lastLoggedIn) authExpiredNotified = false;
        lastLoggedIn = loggedIn;
        return;
      }
      if (payload.type === 'organizer' && payload.event === 'job-updated') {
        const status = cleanText(payload.status);
        if (!['completed', 'completed_warning', 'needs_review', 'failed'].includes(status)) return;
        const organizer = runtime.organizer?.();
        if (!organizer) return;
        const state = organizer.state();
        const job = (state.jobs || []).find((item) => item.id === payload.job_id);
        if (!job || job.status !== status) return;
        if (status === 'completed' || status === 'completed_warning') {
          const mapping = (state.mappings || []).find((item) => item.id === job.mapping_id) || null;
          void notifyOrganizeDone(job, mapping);
        } else {
          notifyReviewNeeded(job);
        }
      }
    } catch (error) {
      log('warning', `Telegram 事件处理失败：${error.message}`);
    }
  }

  async function notifyOrganizeDone(job, mapping) {
    const settings = effectiveSettings();
    if (!settings.enabled || !settings.configured || settings.notify.organize === false || !settings.chat_ids.length) return;
    const text = formatOrganizeDone(job, mapping);
    const posterUrl = cleanText(job?.preview?.metadata?.poster_url);
    const poster = posterUrl ? await fetchImageBuffer(posterUrl) : null;
    notify('organize', text, poster ? { photo: { buffer: poster, filename: 'poster.jpg' } } : {});
  }

  function notifyReviewNeeded(job) {
    const { text, keyboard } = formatReviewNeeded(job);
    notify('review', text, { keyboard });
  }

  function notifyAuthExpired(reason = '') {
    const settings = effectiveSettings();
    if (!settings.enabled || !settings.configured || settings.notify.auth === false) return;
    if (authExpiredNotified) return;
    authExpiredNotified = true;
    const lines = ['🔑 <b>光鸭登录已失效</b>'];
    if (cleanText(reason)) lines.push(esc(reason));
    lines.push('请重新扫码登录；也可以点击下方按钮，直接在 Telegram 中完成扫码。');
    notify('auth', lines.join('\n'), { keyboard: [[{ text: '📷 获取扫码登录二维码', data: 'login' }]] });
  }

  // ------------------------------------------------------------------
  // 命令路由
  // ------------------------------------------------------------------

  function isAllowed(settings, chatId, senderId) {
    return settings.chat_ids.includes(String(chatId)) || settings.chat_ids.includes(String(senderId));
  }

  function helpText(settings, chatId) {
    const authorized = isAllowed(settings, chatId, chatId);
    const lines = [
      `<b>光鸭云盘工作台</b> v${esc(version)}（${esc(platform)}）`,
      '',
      '/status - 系统状态总览',
      '/jobs - 最近整理任务与失败处理',
      '/logs [数量] - 最新运行日志（默认 50 条）',
      '/update - 检查更新',
      '/login - 获取光鸭扫码登录二维码',
      '/help - 本帮助',
      '',
      '重新整理命令：',
      '<code>re &lt;任务ID&gt; tmdbid=12345 [tv|movie] [s=1] [e=2]</code>',
      '例如：<code>re ab12cd34 tmdbid=94605 tv s=1</code>',
      '',
      `当前会话 Chat ID：<code>${esc(chatId)}</code>`,
    ];
    if (!authorized) {
      lines.push('', '⚠️ 该会话尚未授权：请把上面的 Chat ID 填入「设置 → Telegram 通知」后再使用。');
    }
    return lines.join('\n');
  }

  function statusText() {
    const settings = effectiveSettings();
    const snapshot = runtime.snapshot?.() || {};
    const organizerState = runtime.organizer?.()?.state?.() || { jobs: [], mappings: [], counts: {} };
    const virtualLibrary = runtime.virtualLibraryInfo?.() || {};
    const webdav = runtime.webdavInfo?.() || {};
    const counts = organizerState.counts || {};
    const countsLine = Object.entries(counts)
      .map(([key, value]) => `${JOB_STATUS_LABELS[key] || key} ${value}`)
      .join(' · ');
    const mappings = Array.isArray(snapshot.mappings) ? snapshot.mappings : [];
    const lines = [
      `<b>光鸭云盘工作台</b> v${esc(version)}（${esc(platform)}）`,
      `登录状态：${snapshot.logged_in ? '✅ 已登录' : '❌ 未登录（发送 /login 扫码登录）'}`,
      `上传队列：等待 ${Number(snapshot.pending) || 0} · 进行中 ${Number(snapshot.active_uploads) || 0}${snapshot.paused ? ' · 已暂停' : ''}`,
      `备份任务：${mappings.length} 个（启用 ${mappings.filter((item) => item.enabled).length} 个）`,
      `整理监控：${(organizerState.mappings || []).length} 个`,
      `整理任务：${countsLine || '暂无记录'}`,
      `WebDAV：${webdav.configured ? `已配置（端口 ${webdav.port ?? '-'}）` : '未配置'}`,
      `Emby 网关：${virtualLibrary.gateway_running ? `运行中（端口 ${virtualLibrary.gateway_port ?? '-'}）` : '未运行'}`,
      `Telegram：${connected ? `已连接（${settings.mode === 'mtproto' ? 'MTProto' : 'Bot API'}）` : '未连接'}`,
    ];
    return lines.join('\n');
  }

  function jobsResponse() {
    const state = runtime.organizer?.()?.state?.() || { jobs: [] };
    const jobs = (state.jobs || []).slice(0, 10);
    if (!jobs.length) return { text: '暂无整理任务记录' };
    const lines = ['<b>最近整理任务</b>'];
    for (const job of jobs) {
      const icon = JOB_STATUS_ICONS[job.status] || '▫️';
      const label = JOB_STATUS_LABELS[job.status] || job.status;
      lines.push(`${icon} <code>${esc(shortJobId(job.id))}</code> ${esc(describeJobTitle(job))} — ${esc(label)}`);
    }
    const actionable = jobs.filter((job) => ['needs_review', 'failed'].includes(job.status)).slice(0, 5);
    const keyboard = actionable.map((job) => ([
      { text: `🔁 ${shortJobId(job.id)}`, data: `retry:${job.id}` },
      { text: `▶️ 整理`, data: `run:${job.id}` },
      { text: `🔎 TMDB`, data: `ask:${job.id}` },
    ]));
    if (actionable.length) lines.push('', '待处理任务可直接点击下方按钮操作：');
    return { text: lines.join('\n'), keyboard: keyboard.length ? keyboard : undefined };
  }

  async function sendLogs(chatId, limitToken) {
    const limit = Math.max(1, Math.min(200, Number(limitToken) || 50));
    const entries = logBuffer?.list(limit) || [];
    if (!entries.length) {
      await sendTo(chatId, '暂无日志记录');
      return;
    }
    const lines = entries.map((entry) => {
      const time = new Date(entry.time).toLocaleTimeString('zh-CN', { hour12: false });
      return `${time} [${entry.level}] ${entry.message}`;
    });
    const chunks = chunkLines(lines).slice(0, 5);
    for (const chunk of chunks) {
      await sendTo(chatId, `<pre>${esc(chunk)}</pre>`);
    }
  }

  function submitJobRun(chatId, job, input, { retryOnly = false } = {}) {
    const organizer = runtime.organizer?.();
    if (!organizer) {
      void sendTo(chatId, '当前运行端未接入整理服务');
      return;
    }
    const action = retryOnly ? organizer.retryJob(job.id, input) : organizer.runJob(job.id, input);
    // 整理可能耗时数分钟，不阻塞消息循环；结果通过 job-updated 通知回推。
    void Promise.resolve(action).catch((error) => {
      void sendTo(chatId, `⚠️ ${retryOnly ? '重新识别' : '重新整理'}提交失败：${esc(error.message)}`);
    });
  }

  function describeOverrides(input = {}) {
    const parts = [];
    if (input.tmdb_id != null) parts.push(`tmdbid=${input.tmdb_id}`);
    if (input.media_type) parts.push(input.media_type);
    if (input.season != null) parts.push(`s=${input.season}`);
    if (input.episode != null) parts.push(`e=${input.episode}`);
    if (input.title) parts.push(`title=${input.title}`);
    if (input.year != null) parts.push(`year=${input.year}`);
    return parts.length ? `（${parts.join(' ')}）` : '';
  }

  async function handleReText(chatId, text) {
    const parsed = parseReCommand(text);
    if (!parsed) return false;
    if (parsed.error) {
      await sendTo(chatId, esc(parsed.error));
      return true;
    }
    const state = runtime.organizer?.()?.state?.() || { jobs: [] };
    const found = findJobByRef(state.jobs, parsed.jobRef);
    if (found.error) {
      await sendTo(chatId, esc(found.error));
      return true;
    }
    await sendTo(chatId, `已提交重新整理：<code>${esc(shortJobId(found.job.id))}</code> ${esc(describeJobTitle(found.job))}${esc(describeOverrides(parsed.input))}`);
    submitJobRun(chatId, found.job, parsed.input);
    return true;
  }

  async function promptTmdbInput(chatId, job) {
    const text = [
      `请<b>回复本条消息</b>填写 TMDB ID，可附加类型与季集号。`,
      `例如：<code>94605 tv s=1</code>`,
      `任务：<code>${esc(shortJobId(job.id))}</code> ${esc(describeJobTitle(job))}`,
    ].join('\n');
    const messageId = await enqueueSend(async () => {
      if (!current) throw new Error('Telegram Bot 尚未连接');
      return current.sendMessage(chatId, text, { forceReply: true });
    }).catch((error) => {
      log('warning', `Telegram 消息发送失败：${error.message}`);
      return null;
    });
    if (messageId != null) {
      pendingTmdbPrompts.set(`${chatId}:${messageId}`, { jobId: job.id, expiresAt: Date.now() + 10 * 60_000 });
    }
  }

  function cleanupPendingPrompts() {
    const now = Date.now();
    for (const [key, value] of pendingTmdbPrompts) {
      if (value.expiresAt < now) pendingTmdbPrompts.delete(key);
    }
  }

  async function handleTmdbReply(chatId, pending, text) {
    const parsed = parseOverrideTokens(cleanText(text).split(/\s+/));
    if (parsed.error) {
      await sendTo(chatId, esc(parsed.error));
      return;
    }
    if (!Object.keys(parsed.input).length) {
      await sendTo(chatId, '请提供 TMDB ID，例如：<code>94605 tv s=1</code>');
      return;
    }
    const state = runtime.organizer?.()?.state?.() || { jobs: [] };
    const job = (state.jobs || []).find((item) => item.id === pending.jobId);
    if (!job) {
      await sendTo(chatId, '任务已不存在，可能已被清理');
      return;
    }
    await sendTo(chatId, `已提交重新整理：<code>${esc(shortJobId(job.id))}</code>${esc(describeOverrides(parsed.input))}`);
    submitJobRun(chatId, job, parsed.input);
  }

  // ------------------------------------------------------------------
  // 光鸭扫码登录流程
  // ------------------------------------------------------------------

  function pickValue(data, keys, fallback = '') {
    for (const key of keys) {
      const value = data?.[key];
      if (value != null && cleanText(value)) return cleanText(value);
    }
    return fallback;
  }

  async function beginLoginFlow(chatId) {
    if (!runtime.startDeviceLogin || !runtime.pollDeviceLogin) {
      await sendTo(chatId, '当前运行端不支持在 Telegram 中扫码登录');
      return;
    }
    if (loginFlow) {
      clearTimeout(loginFlow.timer);
      loginFlow = null;
    }
    let data;
    try {
      data = await runtime.startDeviceLogin();
    } catch (error) {
      await sendTo(chatId, `创建扫码登录任务失败：${esc(error.message)}`);
      return;
    }
    const deviceCode = pickValue(data, ['device_code', 'deviceCode']);
    const uri = pickValue(data, [
      'short_uri_complete', 'shortUriComplete',
      'verification_uri_complete', 'verificationUriComplete',
      'verification_url', 'verificationUrl',
      'verification_uri', 'verificationUri',
    ]);
    if (!deviceCode || !uri) {
      await sendTo(chatId, '官方没有返回完整扫码信息，请稍后重试');
      return;
    }
    const expiresIn = Math.max(30, Number(data.expires_in || 120));
    const userCode = pickValue(data, ['user_code', 'userCode']);
    let png;
    try {
      png = await QRCode.toBuffer(uri, { type: 'png', width: 512, margin: 2, errorCorrectionLevel: 'M' });
    } catch (error) {
      await sendTo(chatId, `生成二维码失败：${esc(error.message)}`);
      return;
    }
    const caption = [
      '📷 <b>光鸭扫码登录</b>',
      '请使用光鸭 App 扫码并确认登录',
      userCode ? `用户码：<code>${esc(userCode)}</code>` : '',
      `二维码有效期约 ${expiresIn} 秒`,
    ].filter(Boolean).join('\n');
    const sent = await enqueueSend(async () => {
      if (!current) throw new Error('Telegram Bot 尚未连接');
      return current.sendPhoto(chatId, png, { caption, filename: 'guangya-login.png' });
    }).catch((error) => {
      log('warning', `发送登录二维码失败：${error.message}`);
      return null;
    });
    if (sent == null) return;
    const flow = {
      deviceCode,
      chatId,
      deadline: Date.now() + expiresIn * 1000,
      interval: Math.max(2, Number(data.interval || 3)),
      timer: null,
    };
    loginFlow = flow;
    scheduleLoginPoll(flow);
  }

  function scheduleLoginPoll(flow) {
    if (loginFlow !== flow) return;
    flow.timer = setTimeout(async () => {
      if (loginFlow !== flow) return;
      if (Date.now() > flow.deadline) {
        loginFlow = null;
        void sendTo(flow.chatId, '二维码已过期，请重新获取', { keyboard: [[{ text: '📷 重新获取二维码', data: 'login' }]] });
        return;
      }
      try {
        const result = await runtime.pollDeviceLogin(flow.deviceCode);
        if (loginFlow !== flow) return;
        if (result?.authenticated) {
          loginFlow = null;
          authExpiredNotified = false;
          void sendTo(flow.chatId, '✅ 扫码登录成功，光鸭会话已恢复');
          return;
        }
        if (result?.slow_down) flow.interval = Math.min(60, flow.interval * 2);
        scheduleLoginPoll(flow);
      } catch (error) {
        if (loginFlow !== flow) return;
        loginFlow = null;
        void sendTo(flow.chatId, `扫码登录失败：${esc(error.message)}`, { keyboard: [[{ text: '📷 重新获取二维码', data: 'login' }]] });
      }
    }, flow.interval * 1000);
  }

  // ------------------------------------------------------------------
  // 入站消息 / 回调
  // ------------------------------------------------------------------

  async function onMessage({ chatId, senderId, text, replyToId }) {
    const settings = effectiveSettings();
    cleanupPendingPrompts();
    const command = text.split(/\s+/)[0].toLowerCase().replace(/@[\w_]+$/, '');
    if (!isAllowed(settings, chatId, senderId)) {
      // 未授权会话只回应 /start 与 /help，用于获取 Chat ID。
      if (command === '/start' || command === '/help') {
        await sendTo(chatId, helpText(settings, chatId));
      }
      return;
    }
    if (replyToId != null) {
      const key = `${chatId}:${replyToId}`;
      const pending = pendingTmdbPrompts.get(key);
      if (pending) {
        pendingTmdbPrompts.delete(key);
        await handleTmdbReply(chatId, pending, text);
        return;
      }
    }
    if (await handleReText(chatId, text)) return;
    const argument = text.split(/\s+/)[1];
    switch (command) {
      case '/start':
      case '/help':
        await sendTo(chatId, helpText(settings, chatId));
        return;
      case '/status':
        await sendTo(chatId, statusText());
        return;
      case '/jobs': {
        const { text: jobsText, keyboard } = jobsResponse();
        await sendTo(chatId, jobsText, { keyboard });
        return;
      }
      case '/logs':
        await sendLogs(chatId, argument);
        return;
      case '/update': {
        try {
          const info = await runtime.updateInfo?.();
          const message = typeof info === 'string' ? info : info?.text || '当前运行端不支持检查更新';
          await sendTo(chatId, message, typeof info === 'object' && info?.keyboard ? { keyboard: info.keyboard } : {});
        } catch (error) {
          await sendTo(chatId, `检查更新失败：${esc(error.message)}`);
        }
        return;
      }
      case '/login':
        await beginLoginFlow(chatId);
        return;
      default:
        if (command.startsWith('/')) {
          await sendTo(chatId, '未知命令，发送 /help 查看用法');
        }
    }
  }

  async function onCallback({ chatId, senderId, data, answer }) {
    const settings = effectiveSettings();
    if (!isAllowed(settings, chatId, senderId)) {
      await answer('该会话未授权');
      return;
    }
    const separator = data.indexOf(':');
    const action = separator === -1 ? data : data.slice(0, separator);
    const argument = separator === -1 ? '' : data.slice(separator + 1);
    if (action === 'login') {
      await answer('正在获取二维码…');
      await beginLoginFlow(chatId);
      return;
    }
    if (action === 'update_install') {
      await answer('已提交安装');
      try {
        await runtime.installUpdate?.();
      } catch (error) {
        void sendTo(chatId, `安装更新失败：${esc(error.message)}`);
      }
      return;
    }
    if (['run', 'retry', 'ask'].includes(action)) {
      const state = runtime.organizer?.()?.state?.() || { jobs: [] };
      const found = findJobByRef(state.jobs, argument);
      if (found.error) {
        await answer(found.error.slice(0, 190));
        return;
      }
      if (action === 'ask') {
        await answer();
        await promptTmdbInput(chatId, found.job);
        return;
      }
      await answer(action === 'retry' ? '已提交重新识别' : '已提交重新整理');
      submitJobRun(chatId, found.job, {}, { retryOnly: action === 'retry' });
      return;
    }
    await answer();
  }

  // ------------------------------------------------------------------
  // Emby webhook
  // ------------------------------------------------------------------

  function secretMatches(provided) {
    const secret = webhookSecret();
    if (!cleanText(provided) || !secret) return false;
    const left = crypto.createHash('sha256').update(cleanText(provided)).digest();
    const right = crypto.createHash('sha256').update(secret).digest();
    return crypto.timingSafeEqual(left, right);
  }

  async function readRawBody(request, maxBytes) {
    const chunks = [];
    let size = 0;
    for await (const chunk of request) {
      size += chunk.length;
      if (size > maxBytes) throw new Error(`请求体不能超过 ${maxBytes} 字节`);
      chunks.push(chunk);
    }
    return Buffer.concat(chunks, size);
  }

  async function handleEmbyWebhook(request, response, url) {
    const respond = (code, payload) => {
      response.writeHead(code, { 'content-type': 'application/json; charset=utf-8', 'cache-control': 'no-store' });
      response.end(JSON.stringify(payload));
    };
    if (request.method !== 'POST') return respond(405, { error: 'method not allowed' });
    if (!secretMatches(url.searchParams.get('token') || url.searchParams.get('secret'))) {
      return respond(403, { error: 'invalid webhook token' });
    }
    let body;
    try {
      body = await readRawBody(request, 8 * 1024 * 1024);
    } catch (error) {
      return respond(413, { error: error.message });
    }
    const payload = parseEmbyWebhookBody(String(request.headers['content-type'] || ''), body);
    if (!payload) {
      logBuffer?.push('warning', '[Emby] 收到无法解析的 webhook 请求');
      return respond(400, { error: 'unrecognized payload' });
    }
    const eventName = cleanText(payload.Event ?? payload.event) || 'unknown';
    const described = describeEmbyEvent(payload);
    logBuffer?.push('info', `[Emby] webhook：${eventName}${cleanText(payload.Title) ? `（${cleanText(payload.Title)}）` : ''}`);
    if (described) notify(described.category, described.text);
    return respond(200, { ok: true, handled: Boolean(described) });
  }

  // ------------------------------------------------------------------
  // 对外接口
  // ------------------------------------------------------------------

  async function sendTest() {
    const settings = effectiveSettings();
    if (!settings.configured) {
      throw new Error(settings.mode === 'mtproto'
        ? '请先填写 API ID、API Hash 和 Bot Token'
        : '请先填写 Bot Token');
    }
    if (!settings.chat_ids.length) throw new Error('请先填写 Chat ID（可先给机器人发送 /start 获取）');
    const chatId = settings.chat_ids[0];
    const text = [
      '✅ <b>Telegram 通知渠道连接正常</b>',
      `来自：光鸭云盘工作台 v${esc(version)}（${esc(platform)}）`,
      `模式：${settings.mode === 'mtproto' ? 'MTProto' : 'Bot API'}`,
    ].join('\n');
    if (current && connected) {
      await current.sendMessage(chatId, text, {});
      return { ok: true, bot_username: botUsername };
    }
    const transport = await buildTransport(settings);
    try {
      const me = await transport.getMe();
      await transport.sendMessage(chatId, text, {});
      return { ok: true, bot_username: me.username };
    } finally {
      try { await transport.stop?.(); } catch {}
    }
  }

  function initialize() {
    startIfConfigured();
  }

  async function close() {
    generation += 1;
    if (loginFlow) { clearTimeout(loginFlow.timer); loginFlow = null; }
    const previous = current;
    current = null;
    connected = false;
    if (previous) { try { await previous.stop(); } catch {} }
  }

  return {
    publicSettings,
    updateSettings,
    sendTest,
    observeEvent,
    notifyAuthExpired,
    handleEmbyWebhook,
    webhookSecret,
    initialize,
    close,
  };
}

export const telegramInternals = {
  BOT_COMMANDS,
  DEFAULT_NOTIFY,
  ERROR_CODE_LABELS,
  NOTIFY_CATEGORIES,
};
