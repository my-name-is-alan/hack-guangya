import crypto from 'node:crypto';
import { execFile } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { fetch as undiciFetch } from 'undici';
import {
  DEFAULT_ORGANIZER_SETTINGS,
  DEFAULT_CATEGORY_RULES,
  DEFAULT_SCRAPE_TYPES,
  NATIVE_ENGINE_VERSION,
  ORGANIZER_PATH_PRESETS,
  VIDEO_EXTENSIONS,
  analyzeCloudMediaCandidate,
  buildCloudNativePreview,
  buildStandardTemplateExamples,
  classifyNativePreview,
  cloudCandidateFingerprint,
  createTmdbClient,
  normalizeOrganizerCloudEntry,
  parseMediaName,
  planCloudScrapeCandidates,
  renderNfo,
  renderOrganizerPathTemplate,
  normalizeCategoryRules,
  resolveTmdbMatch,
  validateAuxiliaryRuleBlock,
} from './organizer-core.mjs';
import { createProxiedFetch, normalizeProxyUrl } from './network-preferences.mjs';

const TRANSFER_TYPES = new Set(['copy', 'move']);
const MEDIA_TYPES = new Set(['', 'movie', 'tv']);
const CONFLICT_POLICIES = new Set(['skip', 'overwrite', 'rename']);
const SCRAPE_TYPES = new Set(['movie_nfo', 'tvshow_nfo', 'episode_nfo', 'poster', 'fanart', 'season_poster']);
const ACTIVE_STATUSES = new Set(['recognizing', 'ready', 'running', 'needs_review']);
const POLL_INTERVAL_MS = 15_000;
const MAX_CLOUD_ITEMS = 20_000;
const MAX_CLOUD_DEPTH = 64;
const MAX_SCRAPE_CANDIDATES = 1_000;
const SCRAPE_SCAN_CONCURRENCY = 3;
const SCRAPE_RECOGNITION_CONCURRENCY = 4;
const SCRAPE_EXECUTION_CONCURRENCY = 3;
const ORGANIZER_RENAME_CONCURRENCY = 4;

function nowSeconds() {
  return Math.floor(Date.now() / 1000);
}

function cleanText(value) {
  return String(value ?? '').trim();
}

async function mapWithConcurrency(items, concurrency, worker) {
  const source = Array.from(items || []);
  const results = new Array(source.length);
  let cursor = 0;
  async function run() {
    while (cursor < source.length) {
      const index = cursor;
      cursor += 1;
      results[index] = await worker(source[index], index);
    }
  }
  await Promise.all(Array.from({ length: Math.min(Math.max(1, concurrency), source.length) }, run));
  return results;
}

function booleanValue(value, fallback = false) {
  return value === undefined ? fallback : Boolean(value);
}

function normalizeTransferType(value) {
  const normalized = cleanText(value).toLowerCase() || 'copy';
  if (!TRANSFER_TYPES.has(normalized)) throw new Error('云盘内整理方式必须是复制或移动');
  return normalized;
}

function normalizeMediaType(value) {
  const normalized = cleanText(value).toLowerCase();
  if (!MEDIA_TYPES.has(normalized)) throw new Error('媒体类型必须是自动、电影或电视剧');
  return normalized;
}

function normalizeConflictPolicy(value) {
  const normalized = cleanText(value).toLowerCase() || 'skip';
  if (!CONFLICT_POLICIES.has(normalized)) throw new Error('冲突策略必须是跳过、覆盖或保留两份');
  return normalized;
}

function normalizeSettleSeconds(value) {
  const parsed = Number(value ?? 30);
  if (!Number.isInteger(parsed) || parsed < 5 || parsed > 3600) throw new Error('静默等待必须是 5 到 3600 秒之间的整数');
  return parsed;
}

function normalizeOptionalInteger(value, label, minimum = 0) {
  if (value === '' || value === null || value === undefined) return null;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < minimum) throw new Error(`${label}必须是大于等于 ${minimum} 的整数`);
  return parsed;
}

function normalizeMatchScore(value) {
  const parsed = Number(value ?? DEFAULT_ORGANIZER_SETTINGS.minimum_match_score);
  if (!Number.isFinite(parsed) || parsed < 0.4 || parsed > 0.98) throw new Error('自动匹配阈值必须在 0.40 到 0.98 之间');
  return Number(parsed.toFixed(2));
}

function normalizeRuleText(value, label) {
  const normalized = String(value ?? '').replace(/\r\n?/g, '\n').trim();
  if (normalized.length > 100_000) throw new Error(`${label}不能超过 100000 个字符`);
  if (normalized && normalized.split('\n').length > 2_000) throw new Error(`${label}不能超过 2000 行`);
  return normalized;
}

function normalizeLanguage(value, fallback = 'zh-CN') {
  const normalized = cleanText(value) || fallback;
  if (!/^[a-z]{2}(?:-[A-Z]{2})?$/.test(normalized)) throw new Error('TMDB 语言格式不正确，例如 zh-CN 或 en-US');
  return normalized;
}

function normalizeImageLanguage(value) {
  const normalized = cleanText(value) || DEFAULT_ORGANIZER_SETTINGS.image_language;
  if (!/^[a-z]{2}(?:-[A-Z]{2})?(?:,(?:[a-z]{2}(?:-[A-Z]{2})?|null))*$/.test(normalized)) throw new Error('图片语言格式不正确，例如 zh-CN,null,en');
  return normalized;
}

function normalizePathTemplate(value, fallback, label) {
  const normalized = cleanText(value) || fallback;
  if (normalized.length > 500) throw new Error(`${label}不能超过 500 个字符`);
  if (!/\{title\}/i.test(normalized)) throw new Error(`${label}必须包含 {title}`);
  renderOrganizerPathTemplate(normalized, {
    category: '分类', country: 'CN', year: 2026, title: '示例', original_title: 'Example', tmdb_id: 1,
    season: 1, episode: 1, episode_end: '', episode_title: '第一集', edition: '', quality: '', part: '', ext: 'mkv',
    fileExt: '.mkv', season_tag: 'S01', episode_tag: 'E01', season_episode: 'S01E01',
    video_format: '2160p', videoFormat: '2160p', resource_type: 'WEB-DL', resourceType: 'WEB-DL',
    source: 'Netflix', effect: 'HDR', audio_info: '5.1', audioInfo: '5.1', video_codec: 'HEVC', videoCodec: 'HEVC',
    audio_codec: 'DDP', audioCodec: 'DDP', release_group: 'Example', releaseGroup: 'Example', release_type: 'WEB-DL',
  });
  return normalized;
}

function normalizeCategory(value, fallback, label) {
  const normalized = cleanText(value) || fallback;
  if (normalized.length > 80 || /[\\/]/.test(normalized)) throw new Error(`${label}不能包含路径分隔符且不能超过 80 个字符`);
  return normalized;
}

function normalizeScrapeTypes(value, enabled) {
  if (!enabled) return [];
  const source = Array.isArray(value) && value.length ? value : DEFAULT_SCRAPE_TYPES;
  const normalized = [...new Set(source.map((item) => cleanText(item).toLowerCase()).filter(Boolean))];
  const invalid = normalized.find((item) => !SCRAPE_TYPES.has(item));
  if (invalid) throw new Error(`不支持的刮削类型：${invalid}`);
  return normalized;
}

function normalizeCloudPath(value, fallback = '/') {
  const normalized = cleanText(value).replaceAll('\\', '/').replace(/\/{2,}/g, '/');
  if (!normalized || normalized === '/') return fallback;
  return `/${normalized.replace(/^\/+|\/+$/g, '')}`;
}

function normalizeMirrorUrl(value, fallback, label) {
  const normalized = cleanText(value) || fallback;
  if (!/^https?:\/\//i.test(normalized)) throw new Error(`${label}必须以 http:// 或 https:// 开头`);
  if (normalized.length > 500 || /[?#]/.test(normalized)) throw new Error(`${label}格式不正确或过长`);
  return normalized.replace(/\/+$/, '');
}

function normalizeScrapeTargets(value) {
  const source = Array.isArray(value) ? value : [];
  return source.slice(0, 50).map((target, index) => {
    const name = cleanText(target?.name) || `媒体库 ${index + 1}`;
    const dirId = cleanText(target?.dir_id ?? target?.target_dir_id);
    const cloudPath = normalizeCloudPath(target?.path ?? target?.target_path, '/');
    if (!dirId) throw new Error(`第 ${index + 1} 个刮削目标未选择云盘目录`);
    return { id: cleanText(target?.id) || crypto.randomUUID(), name: name.slice(0, 80), dir_id: dirId, path: cloudPath };
  });
}

function configuredOutputTarget(targets, targetDirId) {
  const normalizedId = cleanText(targetDirId);
  return (Array.isArray(targets) ? targets : []).find((target) => cleanText(target?.dir_id ?? target?.target_dir_id) === normalizedId) || null;
}

function bindConfiguredOutputTarget(mapping, targets, { allowLegacy = false } = {}) {
  const target = configuredOutputTarget(targets, mapping.target_dir_id);
  if (!target) {
    if (allowLegacy) return mapping;
    throw new Error('目标 B 目录必须从“刮削输出”中已配置的媒体库目标选择');
  }
  return {
    ...mapping,
    target_dir_id: cleanText(target.dir_id ?? target.target_dir_id),
    target_path: normalizeCloudPath(target.path ?? target.target_path, '/'),
  };
}

export function normalizeOrganizerMappingInput(input = {}, current = {}) {
  const sourceDirId = cleanText(input.source_dir_id ?? current.source_dir_id);
  const targetDirId = cleanText(input.target_dir_id ?? current.target_dir_id);
  if (!sourceDirId) throw new Error('请选择光鸭云盘来源 A 目录（不允许使用云盘根目录）');
  if (!targetDirId) throw new Error('请选择光鸭云盘目标 B 目录（不允许使用云盘根目录）');
  if (sourceDirId === targetDirId) throw new Error('来源 A 目录与目标 B 目录不能相同');
  const sourcePath = normalizeCloudPath(input.source_path ?? current.source_path);
  const targetPath = normalizeCloudPath(input.target_path ?? current.target_path);
  if (sourcePath === targetPath || targetPath.startsWith(`${sourcePath}/`) || sourcePath.startsWith(`${targetPath}/`)) {
    throw new Error('来源 A 与目标 B 目录不能互相包含，避免循环整理');
  }
  const transferType = normalizeTransferType(input.transfer_type ?? current.transfer_type);
  const conflictPolicy = normalizeConflictPolicy(input.conflict_policy ?? current.conflict_policy);
  const riskAcknowledged = booleanValue(
    input.share_risk_acknowledged ?? input.acknowledge_share_risk,
    Boolean(current.share_risk_acknowledged),
  );
  if ((transferType === 'move' || conflictPolicy === 'overwrite') && !riskAcknowledged) {
    throw new Error('移动或覆盖可能使已有分享失效，请先确认分享失效风险');
  }
  const scrape = booleanValue(input.scrape, Boolean(current.scrape));
  return {
    source_dir_id: sourceDirId,
    target_dir_id: targetDirId,
    source_path: sourcePath,
    target_path: targetPath,
    enabled: booleanValue(input.enabled, current.enabled !== false),
    scan_existing: booleanValue(input.scan_existing, current.scan_existing !== false),
    monitor_mode: 'cloud_polling',
    transfer_type: transferType,
    media_type: normalizeMediaType(input.media_type ?? current.media_type),
    scrape,
    scrape_types: normalizeScrapeTypes(input.scrape_types ?? current.scrape_types, scrape),
    sync_extras: booleanValue(input.sync_extras, current.sync_extras !== false),
    conflict_policy: conflictPolicy,
    auto_execute: booleanValue(input.auto_execute, Boolean(current.auto_execute)),
    share_after_organize: booleanValue(input.share_after_organize, Boolean(current.share_after_organize)),
    share_risk_acknowledged: riskAcknowledged,
    settle_seconds: normalizeSettleSeconds(input.settle_seconds ?? current.settle_seconds),
  };
}

function ensureColumn(database, table, column, definition) {
  const columns = database.prepare(`PRAGMA table_info(${table})`).all();
  if (!columns.some((item) => item.name === column)) database.exec(`ALTER TABLE ${table} ADD COLUMN ${column} ${definition}`);
}

function initializeSchema(database) {
  database.exec(`
    CREATE TABLE IF NOT EXISTS organizer_settings (
      id INTEGER PRIMARY KEY CHECK (id = 1),
      tmdb_api_key TEXT NOT NULL DEFAULT '',
      language TEXT NOT NULL DEFAULT 'zh-CN',
      image_language TEXT NOT NULL DEFAULT 'zh,null,en',
      include_adult INTEGER NOT NULL DEFAULT 0,
      minimum_match_score REAL NOT NULL DEFAULT 0.72,
      word_segment_search INTEGER NOT NULL DEFAULT 1,
      similarity_match INTEGER NOT NULL DEFAULT 1,
      recognition_words TEXT NOT NULL DEFAULT '',
      release_groups TEXT NOT NULL DEFAULT '',
      render_words TEXT NOT NULL DEFAULT '',
      capture_groups TEXT NOT NULL DEFAULT '',
      include_media_info INTEGER NOT NULL DEFAULT 1,
      movie_path_template TEXT NOT NULL DEFAULT '',
      tv_path_template TEXT NOT NULL DEFAULT '',
      movie_category TEXT NOT NULL DEFAULT '电影',
      tv_category TEXT NOT NULL DEFAULT '电视剧',
      tmdb_api_base TEXT NOT NULL DEFAULT '',
      tmdb_image_base TEXT NOT NULL DEFAULT '',
      category_rules TEXT NOT NULL DEFAULT '[]',
      scrape_targets TEXT NOT NULL DEFAULT '[]',
      default_scrape_types TEXT NOT NULL DEFAULT '["movie_nfo","tvshow_nfo","poster","fanart"]',
      updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS organizer_mappings (
      id TEXT PRIMARY KEY,
      source_path TEXT NOT NULL,
      target_path TEXT NOT NULL DEFAULT '',
      source_dir_id TEXT NOT NULL DEFAULT '',
      target_dir_id TEXT NOT NULL DEFAULT '',
      enabled INTEGER NOT NULL DEFAULT 1,
      scan_existing INTEGER NOT NULL DEFAULT 1,
      monitor_mode TEXT NOT NULL DEFAULT 'cloud_polling',
      transfer_type TEXT NOT NULL DEFAULT 'copy',
      media_type TEXT NOT NULL DEFAULT '',
      scrape INTEGER NOT NULL DEFAULT 0,
      scrape_types TEXT NOT NULL DEFAULT '[]',
      sync_extras INTEGER NOT NULL DEFAULT 1,
      conflict_policy TEXT NOT NULL DEFAULT 'skip',
      auto_execute INTEGER NOT NULL DEFAULT 0,
      share_after_organize INTEGER NOT NULL DEFAULT 0,
      share_risk_acknowledged INTEGER NOT NULL DEFAULT 0,
      settle_seconds INTEGER NOT NULL DEFAULT 30,
      watch_error TEXT,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS organizer_jobs (
      id TEXT PRIMARY KEY,
      mapping_id TEXT NOT NULL,
      source_path TEXT NOT NULL,
      source_id TEXT NOT NULL DEFAULT '',
      source_parent_id TEXT NOT NULL DEFAULT '',
      source_size INTEGER NOT NULL DEFAULT 0,
      source_modified_ms TEXT NOT NULL DEFAULT '0',
      source_file_count INTEGER NOT NULL DEFAULT 0,
      source_signature TEXT NOT NULL DEFAULT '',
      share_after_requested INTEGER NOT NULL DEFAULT 0,
      status TEXT NOT NULL,
      media_type TEXT,
      tmdb_id TEXT,
      season INTEGER,
      episode INTEGER,
      episode_end INTEGER,
      query_title TEXT,
      query_year INTEGER,
      preview_json TEXT,
      result_json TEXT,
      error_code TEXT,
      message TEXT,
      created_at INTEGER NOT NULL,
      updated_at INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS organizer_jobs_mapping_status ON organizer_jobs(mapping_id, status, updated_at);
    CREATE INDEX IF NOT EXISTS organizer_jobs_source_id ON organizer_jobs(mapping_id, source_id, updated_at);
  `);
  for (const [column, definition] of Object.entries({
    word_segment_search: 'INTEGER NOT NULL DEFAULT 1',
    similarity_match: 'INTEGER NOT NULL DEFAULT 1',
    recognition_words: "TEXT NOT NULL DEFAULT ''",
    release_groups: "TEXT NOT NULL DEFAULT ''",
    render_words: "TEXT NOT NULL DEFAULT ''",
    capture_groups: "TEXT NOT NULL DEFAULT ''",
    include_media_info: 'INTEGER NOT NULL DEFAULT 1',
    movie_path_template: "TEXT NOT NULL DEFAULT ''",
    tv_path_template: "TEXT NOT NULL DEFAULT ''",
    movie_category: "TEXT NOT NULL DEFAULT '电影'",
    tv_category: "TEXT NOT NULL DEFAULT '电视剧'",
    tmdb_api_base: "TEXT NOT NULL DEFAULT ''",
    tmdb_image_base: "TEXT NOT NULL DEFAULT ''",
    category_rules: "TEXT NOT NULL DEFAULT '[]'",
    scrape_targets: "TEXT NOT NULL DEFAULT '[]'",
    default_scrape_types: "TEXT NOT NULL DEFAULT '[\"movie_nfo\",\"tvshow_nfo\",\"poster\",\"fanart\"]'",
  })) ensureColumn(database, 'organizer_settings', column, definition);
  for (const [column, definition] of Object.entries({
    source_dir_id: "TEXT NOT NULL DEFAULT ''",
    target_dir_id: "TEXT NOT NULL DEFAULT ''",
    scrape_types: "TEXT NOT NULL DEFAULT '[]'",
    share_after_organize: 'INTEGER NOT NULL DEFAULT 0',
    share_risk_acknowledged: 'INTEGER NOT NULL DEFAULT 0',
  })) ensureColumn(database, 'organizer_mappings', column, definition);
  for (const [column, definition] of Object.entries({
    source_id: "TEXT NOT NULL DEFAULT ''",
    source_parent_id: "TEXT NOT NULL DEFAULT ''",
    source_signature: "TEXT NOT NULL DEFAULT ''",
    share_after_requested: 'INTEGER NOT NULL DEFAULT 0',
    episode: 'INTEGER',
    episode_end: 'INTEGER',
    query_title: 'TEXT',
    query_year: 'INTEGER',
    result_json: 'TEXT',
    error_code: 'TEXT',
  })) ensureColumn(database, 'organizer_jobs', column, definition);
  database.prepare("UPDATE organizer_jobs SET status = 'failed', error_code = 'service_restarted', message = '服务上次退出，任务可重新识别', updated_at = ? WHERE status IN ('recognizing', 'running')").run(nowSeconds());
  database.prepare("UPDATE organizer_mappings SET enabled = 0, scrape = 0, watch_error = '旧版本地整理配置已停用，请重新选择光鸭云盘 A/B 目录', updated_at = ? WHERE source_dir_id = '' OR target_dir_id = ''").run(nowSeconds());
  database.prepare("UPDATE organizer_jobs SET status = 'needs_review', error_code = 'engine_migrated', message = '整理引擎已切换为光鸭云盘内原生整理，请重新识别', updated_at = ? WHERE status = 'ready' AND (preview_json IS NULL OR preview_json NOT LIKE ?)")
    .run(nowSeconds(), `%\"engine\":\"${NATIVE_ENGINE_VERSION}\"%`);
}

function parseJson(value, fallback = null) {
  if (!value) return fallback;
  try { return JSON.parse(value); } catch { return fallback; }
}

function mappingFromRow(row) {
  if (!row) return null;
  return {
    ...row,
    enabled: Boolean(row.enabled),
    scan_existing: Boolean(row.scan_existing),
    scrape: Boolean(row.scrape),
    scrape_types: parseJson(row.scrape_types, []),
    sync_extras: Boolean(row.sync_extras),
    auto_execute: Boolean(row.auto_execute),
    share_after_organize: Boolean(row.share_after_organize),
    share_risk_acknowledged: Boolean(row.share_risk_acknowledged),
    settle_seconds: Number(row.settle_seconds || 30),
    monitor_mode: 'cloud_polling',
    transfer_type: normalizeTransferType(row.transfer_type),
    conflict_policy: normalizeConflictPolicy(row.conflict_policy),
  };
}

function jobFromRow(row) {
  if (!row) return null;
  return {
    ...row,
    source_size: Number(row.source_size || 0),
    source_file_count: Number(row.source_file_count || 0),
    share_after_requested: Boolean(row.share_after_requested),
    tmdb_id: row.tmdb_id == null || row.tmdb_id === '' ? null : Number(row.tmdb_id),
    season: row.season == null ? null : Number(row.season),
    episode: row.episode == null ? null : Number(row.episode),
    episode_end: row.episode_end == null ? null : Number(row.episode_end),
    query_year: row.query_year == null ? null : Number(row.query_year),
    preview: parseJson(row.preview_json),
    result: parseJson(row.result_json),
    preview_json: undefined,
    result_json: undefined,
  };
}

function isUsefulCloudCandidate(entry) {
  const normalized = normalizeOrganizerCloudEntry(entry);
  if (!normalized.id || !normalized.name || normalized.name.startsWith('.') || normalized.name.startsWith('~$')) return false;
  return normalized.is_directory || VIDEO_EXTENSIONS.has(path.posix.extname(normalized.name).toLowerCase());
}

function mappingSelect() {
  return `SELECT id, source_path, target_path, source_dir_id, target_dir_id, enabled, scan_existing, monitor_mode,
    transfer_type, media_type, scrape, scrape_types, sync_extras, conflict_policy, auto_execute,
    share_after_organize, share_risk_acknowledged, settle_seconds, watch_error, created_at, updated_at FROM organizer_mappings`;
}

function jobSelect() {
  return `SELECT id, mapping_id, source_path, source_id, source_parent_id, source_size, source_modified_ms,
    source_file_count, source_signature, share_after_requested, status, media_type, tmdb_id, season, episode,
    episode_end, query_title, query_year, preview_json, result_json, error_code, message, created_at, updated_at FROM organizer_jobs`;
}

function ffprobeExecutableCandidates(env = process.env) {
  const executable = process.platform === 'win32' ? 'ffprobe.exe' : 'ffprobe';
  return [...new Set([
    cleanText(env.FFPROBE_PATH),
    path.resolve('src-tauri/resources', executable),
    path.resolve('resources', executable),
    executable,
  ].filter(Boolean))];
}

function runFfprobe(executable, input, timeoutMs = 45_000) {
  return new Promise((resolve, reject) => {
    const child = execFile(executable, ['-v', 'error', '-show_streams', '-show_format', '-of', 'json', input], {
      windowsHide: true,
      timeout: timeoutMs,
      maxBuffer: 4 * 1024 * 1024,
    }, (error, stdout) => {
      if (error) reject(error);
      else resolve(stdout);
    });
    child.stdin?.end();
  });
}

function rationalFrameRate(value) {
  const [rawNumerator, rawDenominator = '1'] = cleanText(value).split('/');
  const numerator = Number(rawNumerator);
  const denominator = Number(rawDenominator);
  if (!Number.isFinite(numerator) || !Number.isFinite(denominator) || denominator <= 0 || numerator <= 0) return '';
  const rate = numerator / denominator;
  const common = [23.976, 24, 25, 29.97, 30, 48, 50, 59.94, 60, 120].find((item) => Math.abs(item - rate) < 0.02);
  return `${common ?? Number(rate.toFixed(3))}fps`;
}

function normalizedVideoFormat(height, width) {
  const value = Number(height || 0);
  if (value >= 4000 || Number(width || 0) >= 7600) return '4320p';
  if (value >= 2000 || Number(width || 0) >= 3800) return '2160p';
  if (value >= 1350) return '1440p';
  if (value >= 1000) return '1080p';
  if (value >= 700) return '720p';
  if (value >= 540) return '576p';
  if (value >= 460) return '480p';
  return value > 0 ? `${value}p` : '';
}

function normalizedVideoCodec(value) {
  return ({ h264: 'AVC', avc: 'AVC', hevc: 'HEVC', h265: 'HEVC', av1: 'AV1', vp9: 'VP9', mpeg2video: 'MPEG2' })[cleanText(value).toLowerCase()] || cleanText(value).toUpperCase();
}

function normalizedAudioCodec(value, profile = '') {
  const codec = cleanText(value).toLowerCase();
  if (codec === 'eac3') return 'DDP';
  if (codec === 'ac3') return 'DD';
  if (codec === 'truehd') return 'TrueHD';
  if (codec === 'dts' && /master|ma/i.test(profile)) return 'DTS-HD MA';
  return ({ dts: 'DTS', aac: 'AAC', flac: 'FLAC', opus: 'OPUS', mp3: 'MP3', pcm_s16le: 'LPCM', pcm_s24le: 'LPCM' })[codec] || cleanText(value).toUpperCase();
}

function normalizedAudioLayout(stream = {}) {
  const layout = cleanText(stream.channel_layout).toLowerCase();
  const channels = Number(stream.channels || 0);
  if (/7\.1/.test(layout) || channels === 8) return '7.1';
  if (/6\.1/.test(layout) || channels === 7) return '6.1';
  if (/5\.1/.test(layout) || channels === 6) return '5.1';
  if (/stereo/.test(layout) || channels === 2) return '2.0';
  if (/mono/.test(layout) || channels === 1) return '1.0';
  return channels > 0 ? `${channels}.0` : '';
}

function colorDepth(stream = {}) {
  const explicit = Number(stream.bits_per_raw_sample || stream.bits_per_sample || 0);
  if (explicit >= 8) return `${explicit}bit`;
  const match = cleanText(stream.pix_fmt).match(/(?:p|le|be)(10|12|16)(?:le|be)?$/i);
  return match ? `${match[1]}bit` : '';
}

function sideDataText(stream = {}) {
  return JSON.stringify([stream.side_data_list, stream.tags, stream.profile].filter(Boolean)).toLowerCase();
}

export function parseFfprobeTechnicalData(payload = {}) {
  const streams = Array.isArray(payload.streams) ? payload.streams : [];
  const video = streams.find((stream) => stream.codec_type === 'video') || {};
  const audio = streams.find((stream) => stream.codec_type === 'audio') || {};
  const videoText = sideDataText(video);
  const audioText = sideDataText(audio);
  const dolbyVision = /dolby.?vision|dovi/.test(videoText) ? 'DV' : '';
  const transfer = cleanText(video.color_transfer).toLowerCase();
  const dynamicRange = /hdr10\+|dynamic hdr|smpte2094/.test(videoText) ? 'HDR10+'
    : transfer === 'smpte2084' ? 'HDR10' : transfer === 'arib-std-b67' ? 'HLG' : '';
  const layout = normalizedAudioLayout(audio);
  const atmos = /atmos|joc/.test(audioText);
  const result = {
    video_format: normalizedVideoFormat(video.height, video.width),
    video_codec: normalizedVideoCodec(video.codec_name),
    frame_rate: rationalFrameRate(video.avg_frame_rate || video.r_frame_rate),
    color_depth: colorDepth(video),
    dolby_vision: dolbyVision,
    dynamic_range: dynamicRange,
    audio_codec: normalizedAudioCodec(audio.codec_name, audio.profile),
    audio_info: [atmos ? 'Atmos' : '', layout].filter(Boolean).join(' '),
  };
  result.effect = [result.dolby_vision, result.dynamic_range].filter(Boolean).join(' ');
  return Object.fromEntries(Object.entries(result).filter(([, value]) => value));
}

async function probeTechnicalData(input, env = process.env) {
  let unavailable = true;
  let lastError = null;
  for (const executable of ffprobeExecutableCandidates(env)) {
    if (executable.includes(path.sep) && !fs.existsSync(executable)) continue;
    try {
      const stdout = await runFfprobe(executable, input);
      return parseFfprobeTechnicalData(JSON.parse(stdout));
    } catch (error) {
      lastError = error;
      if (error?.code !== 'ENOENT') unavailable = false;
    }
  }
  if (unavailable) throw new Error('未找到 FFprobe，请重新执行安装或打包准备');
  throw new Error(lastError?.killed ? 'FFprobe 获取媒体信息超时' : 'FFprobe 无法读取该媒体文件');
}

async function enrichAnalysisWithMediaInfo(analysis, cloud, env) {
  const warnings = [];
  for (const video of analysis.videos) {
    try {
      if (typeof cloud.getDownloadUrl !== 'function') throw new Error('当前运行环境没有提供媒体读取地址');
      const downloadUrl = await cloud.getDownloadUrl(video.source_id);
      Object.assign(video.parsed, await probeTechnicalData(downloadUrl, env));
      video.parsed.media_probed = true;
    } catch (error) {
      warnings.push(`${video.source_name || video.source}：${error.message}`);
    }
  }
  analysis.media_probe_warnings = warnings;
  return analysis;
}

export function createOrganizerService({ database, cloud, publish = () => {}, env = process.env, fetchImpl = undiciFetch, getNetworkPreferences = () => ({}) }) {
  initializeSchema(database);
  const envApiKey = cleanText(env.TMDB_API_KEY || env.TMDB_READ_ACCESS_TOKEN);
  const envLanguage = cleanText(env.TMDB_LANGUAGE);
  const envImageLanguage = cleanText(env.TMDB_IMAGE_LANGUAGE);
  const pendingTimers = new Map();
  const runningCandidates = new Set();
  const executingJobs = new Set();
  const mutatingMappings = new Set();
  const shareOverrides = new Map();
  let pollTimer = null;
  let pollRunning = false;

  function assertCloudAdapter() {
    const required = ['isAuthenticated', 'listChildren', 'createDirectory', 'copyEntry', 'moveEntry', 'renameEntry', 'deleteEntry', 'uploadBuffer'];
    const missing = required.filter((key) => typeof cloud?.[key] !== 'function');
    if (missing.length) throw new Error(`云端整理适配器不完整：${missing.join(', ')}`);
  }

  function storedSettings() {
    return database.prepare(`SELECT tmdb_api_key, language, image_language, include_adult, minimum_match_score,
      word_segment_search, similarity_match, recognition_words, release_groups, render_words, capture_groups, include_media_info,
      movie_path_template, tv_path_template, movie_category, tv_category, tmdb_api_base, tmdb_image_base,
      category_rules, scrape_targets, default_scrape_types, updated_at FROM organizer_settings WHERE id = 1`).get()
      || { tmdb_api_key: '', ...DEFAULT_ORGANIZER_SETTINGS, updated_at: 0 };
  }

  function effectiveSettings() {
    const stored = storedSettings();
    const configuredNetwork = getNetworkPreferences?.() || {};
    const configuredApiBase = cleanText(env.TMDB_API_BASE) || cleanText(stored.tmdb_api_base) || 'https://api.themoviedb.org/3';
    const configuredImageBase = cleanText(env.TMDB_IMAGE_BASE) || cleanText(stored.tmdb_image_base) || 'https://image.tmdb.org/t/p';
    const categoryRules = normalizeCategoryRules(parseJson(stored.category_rules, DEFAULT_CATEGORY_RULES));
    const scrapeTargets = normalizeScrapeTargets(parseJson(stored.scrape_targets, []));
    const defaultScrapeTypes = normalizeScrapeTypes(parseJson(stored.default_scrape_types, DEFAULT_SCRAPE_TYPES), true);
    return {
      api_key: envApiKey || cleanText(stored.tmdb_api_key),
      language: normalizeLanguage(envLanguage || stored.language, DEFAULT_ORGANIZER_SETTINGS.language),
      image_language: normalizeImageLanguage(envImageLanguage || stored.image_language),
      include_adult: Boolean(stored.include_adult),
      minimum_match_score: normalizeMatchScore(stored.minimum_match_score),
      word_segment_search: stored.word_segment_search !== 0,
      similarity_match: stored.similarity_match !== 0,
      recognition_words: normalizeRuleText(stored.recognition_words, '自定义识别词'),
      release_groups: normalizeRuleText(stored.release_groups, '自定义制作组'),
      render_words: normalizeRuleText(stored.render_words, '自定义渲染词'),
      capture_groups: normalizeRuleText(stored.capture_groups, '自定义捕获组'),
      include_media_info: stored.include_media_info !== 0,
      movie_path_template: cleanText(stored.movie_path_template) || DEFAULT_ORGANIZER_SETTINGS.movie_path_template,
      tv_path_template: cleanText(stored.tv_path_template) || DEFAULT_ORGANIZER_SETTINGS.tv_path_template,
      movie_category: cleanText(stored.movie_category) || DEFAULT_ORGANIZER_SETTINGS.movie_category,
      tv_category: cleanText(stored.tv_category) || DEFAULT_ORGANIZER_SETTINGS.tv_category,
      tmdb_api_base: normalizeMirrorUrl(configuredApiBase, 'https://api.themoviedb.org/3', 'TMDB API 镜像'),
      tmdb_image_base: normalizeMirrorUrl(configuredImageBase, 'https://image.tmdb.org/t/p', 'TMDB 图片镜像'),
      category_rules: categoryRules,
      scrape_targets: scrapeTargets,
      default_scrape_types: defaultScrapeTypes,
      // TMDB and all other external integrations share one proxy setting.
      tmdb_proxy: cleanText(configuredNetwork.proxy_url || configuredNetwork.tmdb_proxy),
      api_key_managed_by_environment: Boolean(envApiKey),
      language_managed_by_environment: Boolean(envLanguage),
      image_language_managed_by_environment: Boolean(envImageLanguage),
      tmdb_api_base_managed_by_environment: Boolean(cleanText(env.TMDB_API_BASE)),
      tmdb_image_base_managed_by_environment: Boolean(cleanText(env.TMDB_IMAGE_BASE)),
    };
  }

  function publicSettings() {
    const settings = effectiveSettings();
    return {
      provider: 'tmdb', engine: NATIVE_ENGINE_VERSION, configured: Boolean(settings.api_key), api_key_configured: Boolean(settings.api_key),
      api_key_managed_by_environment: settings.api_key_managed_by_environment,
      language_managed_by_environment: settings.language_managed_by_environment,
      image_language_managed_by_environment: settings.image_language_managed_by_environment,
      language: settings.language, image_language: settings.image_language, include_adult: settings.include_adult,
      minimum_match_score: settings.minimum_match_score, word_segment_search: settings.word_segment_search,
      similarity_match: settings.similarity_match, recognition_words: settings.recognition_words,
      release_groups: settings.release_groups, render_words: settings.render_words, capture_groups: settings.capture_groups,
      include_media_info: settings.include_media_info,
      movie_path_template: settings.movie_path_template,
      tv_path_template: settings.tv_path_template, movie_category: settings.movie_category, tv_category: settings.tv_category,
      tmdb_api_base: settings.tmdb_api_base, tmdb_image_base: settings.tmdb_image_base,
      tmdb_api_base_managed_by_environment: settings.tmdb_api_base_managed_by_environment,
      tmdb_image_base_managed_by_environment: settings.tmdb_image_base_managed_by_environment,
      category_rules: settings.category_rules, scrape_targets: settings.scrape_targets,
      template_examples: buildStandardTemplateExamples(settings),
      path_presets: ORGANIZER_PATH_PRESETS,
      default_scrape_types: settings.default_scrape_types,
      scrape_type_options: [
        { value: 'movie_nfo', label: '电影 NFO' }, { value: 'tvshow_nfo', label: '剧集 NFO' },
        { value: 'episode_nfo', label: '单集 NFO' }, { value: 'poster', label: '海报' },
        { value: 'fanart', label: '背景图' }, { value: 'season_poster', label: '季海报' },
      ],
    };
  }

  function tmdbClient(settings = effectiveSettings()) {
    const proxy = normalizeProxyUrl(settings.tmdb_proxy, 'TMDB 代理');
    return createTmdbClient({
      apiKey: settings.api_key,
      language: settings.language,
      imageLanguage: settings.image_language,
      includeAdult: settings.include_adult,
      apiBase: settings.tmdb_api_base,
      imageBase: settings.tmdb_image_base,
      fetchImpl: createProxiedFetch(proxy, fetchImpl),
    });
  }

  function listMappings() {
    // One-off selections use transient manual mappings so they can reuse the
    // native preview/transfer pipeline, but they must never appear as polling
    // configurations in Settings.
    return database.prepare(`${mappingSelect()} WHERE id NOT LIKE 'manual:%' ORDER BY created_at`).all().map(mappingFromRow);
  }

  function getMapping(id) {
    return mappingFromRow(database.prepare(`${mappingSelect()} WHERE id = ?`).get(id));
  }

  function listJobs(limit = 100) {
    const normalized = Math.max(1, Math.min(500, Number(limit) || 100));
    return database.prepare(`${jobSelect()} ORDER BY updated_at DESC LIMIT ?`).all(normalized).map(jobFromRow);
  }

  function getJob(id) {
    return jobFromRow(database.prepare(`${jobSelect()} WHERE id = ?`).get(id));
  }

  function state() {
    const jobs = listJobs();
    return { settings: publicSettings(), mappings: listMappings(), jobs, counts: jobs.reduce((result, job) => { result[job.status] = (result[job.status] || 0) + 1; return result; }, {}) };
  }

  function emit(event, detail = {}) {
    publish({ type: 'organizer', event, ...detail });
  }

  function updateJob(id, changes) {
    const entries = Object.entries(changes).filter(([, value]) => value !== undefined);
    if (!entries.length) return;
    database.prepare(`UPDATE organizer_jobs SET ${entries.map(([key]) => `${key} = ?`).join(', ')}, updated_at = ? WHERE id = ?`)
      .run(...entries.map(([, value]) => value), nowSeconds(), id);
  }

  function updateSettings(input = {}) {
    const stored = storedSettings();
    const apiKeyInput = cleanText(input.api_key);
    const apiKey = input.clear_api_key === true ? '' : (apiKeyInput || cleanText(stored.tmdb_api_key));
    if (!envApiKey && !apiKey) throw new Error('请填写 TMDB API Key 或 Read Access Token');
    const language = normalizeLanguage(input.language ?? stored.language);
    const imageLanguage = normalizeImageLanguage(input.image_language ?? stored.image_language);
    const includeAdult = booleanValue(input.include_adult, Boolean(stored.include_adult));
    const minimumMatchScore = normalizeMatchScore(input.minimum_match_score ?? stored.minimum_match_score);
    const wordSegmentSearch = booleanValue(input.word_segment_search, stored.word_segment_search !== 0);
    const similarityMatch = booleanValue(input.similarity_match, stored.similarity_match !== 0);
    const recognitionWords = normalizeRuleText(input.recognition_words ?? stored.recognition_words, '自定义识别词');
    const releaseGroups = normalizeRuleText(input.release_groups ?? stored.release_groups, '自定义制作组');
    const renderWords = normalizeRuleText(input.render_words ?? stored.render_words, '自定义渲染词');
    const captureGroups = normalizeRuleText(input.capture_groups ?? stored.capture_groups, '自定义捕获组');
    const includeMediaInfo = booleanValue(input.include_media_info, stored.include_media_info !== 0);
    validateAuxiliaryRuleBlock(recognitionWords, '自定义识别词');
    validateAuxiliaryRuleBlock(renderWords, '自定义渲染词');
    validateAuxiliaryRuleBlock(captureGroups, '自定义捕获组', { replacement: false });
    const moviePathTemplate = normalizePathTemplate(input.movie_path_template, stored.movie_path_template || DEFAULT_ORGANIZER_SETTINGS.movie_path_template, '电影路径模板');
    const tvPathTemplate = normalizePathTemplate(input.tv_path_template, stored.tv_path_template || DEFAULT_ORGANIZER_SETTINGS.tv_path_template, '电视剧路径模板');
    const movieCategory = normalizeCategory(input.movie_category, stored.movie_category || DEFAULT_ORGANIZER_SETTINGS.movie_category, '电影分类名');
    const tvCategory = normalizeCategory(input.tv_category, stored.tv_category || DEFAULT_ORGANIZER_SETTINGS.tv_category, '电视剧分类名');
    const apiBase = normalizeMirrorUrl(input.tmdb_api_base ?? input.tmdb_url_base ?? input.tmdb_api_proxy, stored.tmdb_api_base || 'https://api.themoviedb.org/3', 'TMDB API 镜像');
    const imageBase = normalizeMirrorUrl(input.tmdb_image_base ?? input.tmdb_image_url ?? input.tmdb_image_proxy, stored.tmdb_image_base || 'https://image.tmdb.org/t/p', 'TMDB 图片镜像');
    const categoryRules = normalizeCategoryRules(input.category_rules ?? parseJson(stored.category_rules, DEFAULT_CATEGORY_RULES));
    const previousScrapeTargets = normalizeScrapeTargets(parseJson(stored.scrape_targets, []));
    const scrapeTargets = normalizeScrapeTargets(input.scrape_targets ?? previousScrapeTargets);
    const defaultScrapeTypes = normalizeScrapeTypes(input.default_scrape_types ?? parseJson(stored.default_scrape_types, DEFAULT_SCRAPE_TYPES), true);
    const nextTargetIds = new Set(scrapeTargets.map((target) => target.dir_id));
    const removedTargetIds = new Set(previousScrapeTargets.map((target) => target.dir_id).filter((id) => !nextTargetIds.has(id)));
    const usedRemovedTarget = listMappings().find((mapping) => removedTargetIds.has(mapping.target_dir_id));
    if (usedRemovedTarget) throw new Error(`刮削输出“${usedRemovedTarget.target_path}”仍被整理监控使用，请先修改对应监控的输出目标`);
    database.prepare(`INSERT INTO organizer_settings
      (id, tmdb_api_key, language, image_language, include_adult, minimum_match_score, word_segment_search, similarity_match,
       recognition_words, release_groups, render_words, capture_groups, include_media_info, movie_path_template, tv_path_template, movie_category, tv_category,
       tmdb_api_base, tmdb_image_base, category_rules, scrape_targets, default_scrape_types, updated_at)
      VALUES (1, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(id) DO UPDATE SET tmdb_api_key=excluded.tmdb_api_key, language=excluded.language,
        image_language=excluded.image_language, include_adult=excluded.include_adult,
        minimum_match_score=excluded.minimum_match_score, word_segment_search=excluded.word_segment_search,
        similarity_match=excluded.similarity_match, recognition_words=excluded.recognition_words,
        release_groups=excluded.release_groups, render_words=excluded.render_words, capture_groups=excluded.capture_groups,
        include_media_info=excluded.include_media_info,
        movie_path_template=excluded.movie_path_template,
        tv_path_template=excluded.tv_path_template, movie_category=excluded.movie_category,
        tv_category=excluded.tv_category, tmdb_api_base=excluded.tmdb_api_base, tmdb_image_base=excluded.tmdb_image_base,
        category_rules=excluded.category_rules, scrape_targets=excluded.scrape_targets, default_scrape_types=excluded.default_scrape_types, updated_at=excluded.updated_at`)
      .run(apiKey, language, imageLanguage, Number(includeAdult), minimumMatchScore, Number(wordSegmentSearch), Number(similarityMatch),
        recognitionWords, releaseGroups, renderWords, captureGroups, Number(includeMediaInfo), moviePathTemplate, tvPathTemplate, movieCategory, tvCategory,
        apiBase, imageBase, JSON.stringify(categoryRules), JSON.stringify(scrapeTargets), JSON.stringify(defaultScrapeTypes), nowSeconds());
    const updateMappingTargetPath = database.prepare('UPDATE organizer_mappings SET target_path = ?, updated_at = ? WHERE target_dir_id = ?');
    for (const target of scrapeTargets) updateMappingTargetPath.run(target.path, nowSeconds(), target.dir_id);
    emit('settings-updated');
    return publicSettings();
  }

  async function testConnection(input = {}) {
    const current = effectiveSettings();
    const apiKey = cleanText(input.api_key) || current.api_key;
    if (!apiKey) throw new Error('请填写 TMDB API Key 或 Read Access Token');
    const apiBase = normalizeMirrorUrl(input.tmdb_api_base ?? input.tmdb_url_base ?? input.tmdb_api_proxy, current.tmdb_api_base, 'TMDB API 镜像');
    const imageBase = normalizeMirrorUrl(input.tmdb_image_base ?? input.tmdb_image_url ?? input.tmdb_image_proxy, current.tmdb_image_base, 'TMDB 图片镜像');
    return createTmdbClient({
      apiKey,
      language: normalizeLanguage(input.language ?? current.language),
      imageLanguage: normalizeImageLanguage(input.image_language ?? current.image_language),
      includeAdult: booleanValue(input.include_adult, current.include_adult),
      apiBase,
      imageBase,
      fetchImpl: createProxiedFetch(normalizeProxyUrl(current.tmdb_proxy, 'TMDB 代理'), fetchImpl),
    }).test();
  }

  async function listCloudChildren(parentId) {
    const records = await cloud.listChildren(String(parentId || ''));
    return (Array.isArray(records) ? records : []).map((entry) => normalizeOrganizerCloudEntry(entry));
  }

  async function validateMapping(mapping) {
    if (!cloud.isAuthenticated()) throw new Error('请先登录光鸭云盘');
    const [source, target] = await Promise.all([listCloudChildren(mapping.source_dir_id), listCloudChildren(mapping.target_dir_id)]);
    if (!Array.isArray(source) || !Array.isArray(target)) throw new Error('无法读取云盘 A/B 目录');
  }

  function saveMapping(mapping) {
    database.prepare(`INSERT INTO organizer_mappings
      (id, source_path, target_path, source_dir_id, target_dir_id, enabled, scan_existing, monitor_mode, transfer_type,
       media_type, scrape, scrape_types, sync_extras, conflict_policy, auto_execute, share_after_organize,
       share_risk_acknowledged, settle_seconds, watch_error, created_at, updated_at)
      VALUES (?, ?, ?, ?, ?, ?, ?, 'cloud_polling', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?)
      ON CONFLICT(id) DO UPDATE SET source_path=excluded.source_path, target_path=excluded.target_path,
       source_dir_id=excluded.source_dir_id, target_dir_id=excluded.target_dir_id, enabled=excluded.enabled,
       scan_existing=excluded.scan_existing, monitor_mode='cloud_polling', transfer_type=excluded.transfer_type,
       media_type=excluded.media_type, scrape=excluded.scrape, scrape_types=excluded.scrape_types,
       sync_extras=excluded.sync_extras, conflict_policy=excluded.conflict_policy, auto_execute=excluded.auto_execute,
       share_after_organize=excluded.share_after_organize, share_risk_acknowledged=excluded.share_risk_acknowledged,
       settle_seconds=excluded.settle_seconds, watch_error=NULL, updated_at=excluded.updated_at`)
      .run(mapping.id, mapping.source_path, mapping.target_path, mapping.source_dir_id, mapping.target_dir_id,
        Number(mapping.enabled), Number(mapping.scan_existing), mapping.transfer_type, mapping.media_type, Number(mapping.scrape),
        JSON.stringify(mapping.scrape_types), Number(mapping.sync_extras), mapping.conflict_policy, Number(mapping.auto_execute),
        Number(mapping.share_after_organize), Number(mapping.share_risk_acknowledged), mapping.settle_seconds,
        mapping.created_at, mapping.updated_at);
  }

  async function addMapping(input = {}) {
    if (!publicSettings().configured) throw new Error('请先配置 TMDB API Key');
    const settings = publicSettings();
    const normalized = bindConfiguredOutputTarget(
      normalizeOrganizerMappingInput(input, { enabled: true, scan_existing: true, transfer_type: 'copy', scrape: false, scrape_types: settings.default_scrape_types, sync_extras: true, conflict_policy: 'skip', auto_execute: false, share_after_organize: false, settle_seconds: 30 }),
      settings.scrape_targets,
    );
    await validateMapping(normalized);
    if (listMappings().some((item) => item.source_dir_id === normalized.source_dir_id)) throw new Error('该云盘 A 目录已经存在整理监控');
    const timestamp = nowSeconds();
    const mapping = { ...normalized, id: crypto.randomUUID(), created_at: timestamp, updated_at: timestamp };
    saveMapping(mapping);
    if (mapping.enabled && mapping.scan_existing) await scanMapping(mapping.id);
    emit('mapping-added', { mapping_id: mapping.id });
    return getMapping(mapping.id);
  }

  function assertMappingIdle(id) {
    const running = database.prepare("SELECT id FROM organizer_jobs WHERE mapping_id = ? AND status = 'running' LIMIT 1").get(id);
    if (running || [...executingJobs].some((jobId) => getJob(jobId)?.mapping_id === id) || [...runningCandidates].some((key) => key.startsWith(`${id}::`))) throw new Error('该云盘目录正在识别或整理，完成后才能修改或删除');
  }

  async function withMappingMutation(id, operation) {
    if (mutatingMappings.has(id)) throw new Error('该云盘目录配置正在变更，请稍后重试');
    mutatingMappings.add(id);
    try { assertMappingIdle(id); return await operation(); } finally { mutatingMappings.delete(id); }
  }

  async function updateMapping(id, input = {}) {
    return withMappingMutation(id, async () => {
      const current = getMapping(id);
      if (!current) throw new Error('整理监控不存在');
      const settings = publicSettings();
      const targetWasSubmitted = input.target_dir_id !== undefined || input.target_path !== undefined;
      const normalized = bindConfiguredOutputTarget(normalizeOrganizerMappingInput(input, current), settings.scrape_targets, { allowLegacy: !targetWasSubmitted });
      if (normalized.enabled && !settings.configured) throw new Error('请先配置 TMDB API Key');
      await validateMapping(normalized);
      if (listMappings().some((item) => item.id !== id && item.source_dir_id === normalized.source_dir_id)) throw new Error('该云盘 A 目录已经存在整理监控');
      saveMapping({ ...current, ...normalized, id, updated_at: nowSeconds() });
      clearPendingForMapping(id);
      emit('mapping-updated', { mapping_id: id });
      return getMapping(id);
    });
  }

  async function removeMapping(id) {
    return withMappingMutation(id, async () => {
      if (!getMapping(id)) throw new Error('整理监控不存在');
      clearPendingForMapping(id);
      database.prepare('DELETE FROM organizer_jobs WHERE mapping_id = ?').run(id);
      database.prepare('DELETE FROM organizer_mappings WHERE id = ?').run(id);
      emit('mapping-removed', { mapping_id: id });
      return {};
    });
  }

  async function removeJob(id, input = {}) {
    const job = getJob(id);
    if (!job) throw new Error('整理任务不存在');
    if (['recognizing', 'running'].includes(job.status) || executingJobs.has(id)) throw new Error('任务正在整理，不能删除');
    const deleteSource = input.delete_source === true;
    const deleteTarget = input.delete_target === true;
    let deletedSource = 0;
    let deletedTarget = 0;
    const warnings = [];
    if (deleteTarget) {
      const targetIds = [...new Set((Array.isArray(job.result?.targets) ? job.result.targets : []).map(cleanText).filter(Boolean))];
      for (const targetId of targetIds) {
        await cloud.deleteEntry(targetId);
        deletedTarget += 1;
      }
      if (!targetIds.length) warnings.push('该记录没有可安全定位的媒体库文件，未删除媒体库内容');
    }
    if (deleteSource) {
      const sourceEntries = await listCloudChildren(job.source_parent_id);
      const source = sourceEntries.find((entry) => entry.id === job.source_id);
      if (source) {
        await cloud.deleteEntry(source.id);
        deletedSource = 1;
      } else {
        warnings.push('源文件已移动或不存在，未重复删除');
      }
    }
    database.prepare('DELETE FROM organizer_jobs WHERE id = ?').run(id);
    if (String(job.mapping_id).startsWith('manual:')) {
      database.prepare("DELETE FROM organizer_mappings WHERE id = ? AND NOT EXISTS (SELECT 1 FROM organizer_jobs WHERE mapping_id = ?)")
        .run(job.mapping_id, job.mapping_id);
    }
    emit('job-removed', { job_id: id, mapping_id: job.mapping_id, deleted_source: deletedSource, deleted_target: deletedTarget });
    return { deleted_source: deletedSource, deleted_target: deletedTarget, warnings };
  }

  function manualMappingFor(target, source) {
    const timestamp = nowSeconds();
    const mapping = {
      id: `manual:${crypto.randomUUID()}`,
      source_path: normalizeCloudPath(source.parent_path || source.path || '/', '/'),
      target_path: normalizeCloudPath(target.path, '/'),
      source_dir_id: cleanText(source.parent_id),
      target_dir_id: cleanText(target.dir_id),
      enabled: true,
      scan_existing: false,
      monitor_mode: 'manual',
      transfer_type: normalizeTransferType(source.transfer_type || 'copy'),
      media_type: normalizeMediaType(source.media_type || ''),
      scrape: true,
      scrape_types: normalizeScrapeTypes(source.scrape_types, true),
      sync_extras: source.sync_extras !== false,
      conflict_policy: normalizeConflictPolicy(source.conflict_policy || 'skip'),
      auto_execute: false,
      share_after_organize: false,
      share_risk_acknowledged: source.share_risk_acknowledged === true,
      settle_seconds: 5,
      created_at: timestamp,
      updated_at: timestamp,
    };
    if (!mapping.source_dir_id || !mapping.target_dir_id) throw new Error('选中文件缺少来源目录或刮削目标目录');
    if ((mapping.transfer_type === 'move' || mapping.conflict_policy === 'overwrite') && !mapping.share_risk_acknowledged) {
      throw new Error('移动/覆盖可能使已有分享失效，请先确认风险');
    }
    return mapping;
  }

  async function scrapeSelected(input = {}) {
    if (!publicSettings().configured) throw new Error('请先配置 TMDB API Key');
    const selected = Array.isArray(input.files) ? input.files : (Array.isArray(input.items) ? input.items : []);
    if (!selected.length) throw new Error('请先选择至少一个视频文件或目录');
    const settings = effectiveSettings();
    const targets = settings.scrape_targets;
    const requestedTargetId = cleanText(input.target_id || input.targetId);
    const target = targets.find((item) => item.id === requestedTargetId) || (targets.length === 1 ? targets[0] : null);
    if (!target) throw new Error(targets.length ? '请选择一个已配置的刮削目标目录' : '请先在设置 > 整理 > 刮削偏好中配置媒体库目标');
    const jobs = [];
    const failures = [];
    const queued = [];
    const candidateIds = new Set();
    await listCloudChildren(target.dir_id);

    const plannedSelections = await mapWithConcurrency(selected.slice(0, 100), SCRAPE_SCAN_CONCURRENCY, async (source) => {
      try {
        const normalizedSource = {
          ...source,
          id: cleanText(source.id || source.file_id),
          parent_id: cleanText(source.parent_id || source.parentId),
          parent_path: source.parent_path || source.parentPath || source.path,
          transfer_type: input.transfer_type || input.transferType || 'copy',
          media_type: input.media_type || input.mediaType || source.media_type || source.mediaType || '',
          scrape_types: input.scrape_types || input.scrapeTypes || settings.default_scrape_types,
          share_risk_acknowledged: input.share_risk_acknowledged === true,
        };
        if (!normalizedSource.id) throw new Error('选中项缺少文件 ID');
        const baseMapping = manualMappingFor(target, normalizedSource);
        const loaded = await loadCloudCandidate(baseMapping, normalizedSource.id);
        if (!loaded) throw new Error('选中的云端项目已经不存在');
        const candidates = planCloudScrapeCandidates(loaded, settings);
        if (!candidates.length) throw new Error('没有找到可刮削的视频文件');
        return { normalizedSource, baseMapping, loaded, candidates };
      } catch (error) {
        failures.push({ id: cleanText(source.id || source.file_id), message: error.message });
        return null;
      }
    });

    for (const planned of plannedSelections.filter(Boolean)) {
      const { normalizedSource, baseMapping, loaded } = planned;
      for (const candidate of planned.candidates) {
        try {
          if (candidateIds.has(candidate.id)) continue;
          if (candidateIds.size >= MAX_SCRAPE_CANDIDATES) {
            failures.push({ id: candidate.id, name: candidate.name, message: `拆分后超过 ${MAX_SCRAPE_CANDIDATES} 个媒体候选，请缩小刮削范围` });
            break;
          }
          candidateIds.add(candidate.id);
          const rootPath = cleanText(loaded.candidate.path || loaded.candidate.name).replaceAll('\\', '/').replace(/^\/+|\/+$/g, '');
          const candidatePath = cleanText(candidate.path || candidate.name).replaceAll('\\', '/').replace(/^\/+|\/+$/g, '');
          const parentLogical = path.posix.dirname(candidatePath);
          const relativeParent = candidate.id === loaded.candidate.id ? '' : path.posix.relative(rootPath, parentLogical);
          const parentPath = candidate.id === loaded.candidate.id
            ? baseMapping.source_path
            : normalizeCloudPath([baseMapping.source_path, loaded.candidate.name, relativeParent === '.' ? '' : relativeParent].filter(Boolean).join('/'));
          let suggestedTitle = cleanText(candidate.suggested_title);
          if (!suggestedTitle && candidate.reason === 'season-folder' && candidate.id === loaded.candidate.id) {
            const parentName = path.posix.basename(cleanText(normalizedSource.parent_path || normalizedSource.path).replaceAll('\\', '/'));
            if (parentName && !/^(?:downloads?|media|library|movies?|tv|shows?|series|下载|媒体库?|电影|电视剧|剧集)$/i.test(parentName)) {
              suggestedTitle = parseMediaName(parentName, { ...settings, media_type: 'tv' }).title;
            }
          }
          const candidateSource = {
            ...normalizedSource,
            id: candidate.id,
            parent_id: candidate.parent_id || baseMapping.source_dir_id,
            parent_path: parentPath,
            path: candidatePath,
            name: candidate.name,
            media_type: normalizedSource.media_type || candidate.suggested_media_type || '',
          };
          const mapping = manualMappingFor(target, candidateSource);
          saveMapping(mapping);
          const id = crypto.randomUUID();
          const timestamp = nowSeconds();
          const sourceName = cleanText(candidate.name) || candidate.id;
          database.prepare(`INSERT INTO organizer_jobs
            (id, mapping_id, source_path, source_id, source_parent_id, source_size, source_modified_ms, source_file_count,
             source_signature, share_after_requested, status, media_type, query_title, message, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, 0, '0', 0, '', 0, 'recognizing', ?, ?, '刮削已提交，等待后台识别', ?, ?)`)
            .run(id, mapping.id, `${mapping.source_path.replace(/\/$/, '')}/${sourceName}`, candidate.id,
              mapping.source_dir_id, mapping.media_type || null, suggestedTitle || null, timestamp, timestamp);
          emit('job-updated', { job_id: id, mapping_id: mapping.id, status: 'recognizing' });
          jobs.push(getJob(id));
          queued.push({ id, mapping });
        } catch (error) {
          failures.push({ id: candidate.id, name: candidate.name, message: error.message });
        }
      }
    }
    void (async () => {
      const recognized = await mapWithConcurrency(queued, SCRAPE_RECOGNITION_CONCURRENCY, async (task) => {
        try {
          const result = await previewJob(task.id, {}, false);
          return result?.status === 'ready' ? result : null;
        } catch (error) {
          updateJob(task.id, { status: 'failed', error_code: 'scrape_failed', message: error.message });
          emit('job-updated', { job_id: task.id, mapping_id: task.mapping.id, status: 'failed', message: error.message });
          return null;
        }
      });
      const targetGroups = new Map();
      for (const job of recognized.filter(Boolean)) {
        const key = `${job.preview?.target_root_id || ''}::${job.preview?.media_root_relative || job.id}`;
        const group = targetGroups.get(key) || [];
        group.push(job.id);
        targetGroups.set(key, group);
      }
      await mapWithConcurrency(targetGroups.values(), SCRAPE_EXECUTION_CONCURRENCY, async (group) => {
        for (const id of group) {
          try { await executeJob(id); }
          catch (error) {
            const job = getJob(id);
            updateJob(id, { status: 'failed', error_code: 'scrape_failed', message: error.message });
            emit('job-updated', { job_id: id, mapping_id: job?.mapping_id || '', status: 'failed', message: error.message });
          }
        }
      });
    })();
    return { jobs, failures, planned: jobs.length, selected: selected.length, state: state() };
  }

  function clearPendingForMapping(id) {
    for (const [key, pending] of pendingTimers) {
      if (!key.startsWith(`${id}::`)) continue;
      clearTimeout(pending.timer);
      pendingTimers.delete(key);
    }
  }

  async function loadCloudCandidate(mapping, candidateId) {
    const roots = await listCloudChildren(mapping.source_dir_id);
    const root = roots.find((entry) => entry.id === candidateId);
    if (!root) return null;
    root.parent_id = mapping.source_dir_id;
    root.path = root.name;
    const entries = [];
    if (root.is_directory) {
      const pending = [{ id: root.id, logicalPath: root.name, depth: 0 }];
      while (pending.length) {
        const current = pending.shift();
        if (current.depth >= MAX_CLOUD_DEPTH) throw new Error(`云盘目录层级超过 ${MAX_CLOUD_DEPTH} 层，已停止扫描`);
        for (const child of await listCloudChildren(current.id)) {
          child.parent_id = current.id;
          child.path = path.posix.join(current.logicalPath, child.name);
          entries.push(child);
          if (entries.length > MAX_CLOUD_ITEMS) throw new Error(`单个整理候选超过 ${MAX_CLOUD_ITEMS} 项，请缩小 A 目录范围`);
          if (child.is_directory) pending.push({ id: child.id, logicalPath: child.path, depth: current.depth + 1 });
        }
      }
    }
    return { candidate: root, entries, fingerprint: cloudCandidateFingerprint(root, entries) };
  }

  function scheduleCandidate(mapping, candidate, { immediate = false, signature = '', shareAfter = null } = {}) {
    if (!candidate?.id || !isUsefulCloudCandidate(candidate)) return false;
    const key = `${mapping.id}::${candidate.id}`;
    const existing = pendingTimers.get(key);
    if (existing && existing.signature === signature && shareAfter == null) return false;
    if (existing) clearTimeout(existing.timer);
    if (shareAfter != null) shareOverrides.set(key, Boolean(shareAfter));
    const timer = setTimeout(() => {
      pendingTimers.delete(key);
      const requested = shareOverrides.has(key) ? shareOverrides.get(key) : null;
      shareOverrides.delete(key);
      void processCandidate(mapping.id, candidate.id, requested).catch((error) => emit('job-error', { mapping_id: mapping.id, source_id: candidate.id, message: error.message }));
    }, immediate ? 0 : mapping.settle_seconds * 1000);
    pendingTimers.set(key, { timer, signature });
    return true;
  }

  async function scanMapping(id, { immediate = true, shareAfter = null, candidateName = '', candidateId = '' } = {}) {
    const mapping = getMapping(id);
    if (!mapping) throw new Error('整理监控不存在');
    if (!mapping.enabled) throw new Error('请先启用整理监控');
    if (!cloud.isAuthenticated()) throw new Error('请先登录光鸭云盘');
    const roots = await listCloudChildren(mapping.source_dir_id);
    let queued = 0;
    for (const candidate of roots) {
      if (!isUsefulCloudCandidate(candidate)) continue;
      if (candidateId && candidate.id !== candidateId) continue;
      if (candidateName && candidate.name !== candidateName) continue;
      const loaded = await loadCloudCandidate(mapping, candidate.id);
      if (!loaded || loaded.fingerprint.video_count < 1) continue;
      if (scheduleCandidate(mapping, candidate, { immediate, signature: loaded.fingerprint.signature, shareAfter })) queued += 1;
    }
    database.prepare('UPDATE organizer_mappings SET watch_error = NULL, updated_at = ? WHERE id = ?').run(nowSeconds(), id);
    emit('scan-started', { mapping_id: id, queued });
    return { queued };
  }

  async function pollMappings() {
    if (pollRunning || !cloud.isAuthenticated()) return;
    pollRunning = true;
    try {
      for (const mapping of listMappings().filter((item) => item.enabled)) {
        try { await scanMapping(mapping.id, { immediate: false }); }
        catch (error) {
          database.prepare('UPDATE organizer_mappings SET watch_error = ?, updated_at = ? WHERE id = ?').run(error.message, nowSeconds(), mapping.id);
          emit('mapping-error', { mapping_id: mapping.id, message: error.message });
        }
      }
    } finally { pollRunning = false; }
  }

  function mappingSignature(mapping, settings = effectiveSettings()) {
    return crypto.createHash('sha256').update(JSON.stringify([
      mapping.source_dir_id, mapping.target_dir_id, mapping.transfer_type, mapping.media_type, mapping.conflict_policy,
      mapping.scrape, mapping.scrape_types, mapping.sync_extras, mapping.share_after_organize,
      settings.language, settings.image_language, settings.movie_path_template, settings.tv_path_template,
      settings.movie_category, settings.tv_category, settings.tmdb_api_base, settings.tmdb_image_base,
      settings.word_segment_search, settings.similarity_match, settings.recognition_words,
      settings.release_groups, settings.render_words, settings.capture_groups,
      settings.include_media_info,
      settings.category_rules, settings.scrape_targets, settings.default_scrape_types,
    ])).digest('hex');
  }

  function resolvedJobOverrides(job, mapping, input = {}) {
    return {
      media_type: normalizeMediaType(input.media_type ?? job.media_type ?? mapping.media_type),
      tmdb_id: input.clear_tmdb_id === true ? null : normalizeOptionalInteger(input.tmdb_id ?? job.tmdb_id, 'TMDB ID', 1),
      title: input.clear_title === true ? '' : cleanText(input.title ?? job.query_title),
      year: input.clear_year === true ? null : normalizeOptionalInteger(input.year ?? job.query_year, '年份', 1800),
      season: input.clear_season === true ? null : normalizeOptionalInteger(input.season ?? job.season, '季号', 0),
      episode: input.clear_episode === true ? null : normalizeOptionalInteger(input.episode ?? job.episode, '集号', 0),
      episode_end: input.clear_episode_end === true ? null : normalizeOptionalInteger(input.episode_end ?? job.episode_end, '结束集号', 0),
    };
  }

  function createTargetResolver(mapping) {
    const cache = new Map();
    async function list(parentId, force = false) {
      if (!force && cache.has(parentId)) return cache.get(parentId);
      const value = await listCloudChildren(parentId);
      cache.set(parentId, value);
      return value;
    }
    async function resolve(relativePath, force = false) {
      const parts = cleanText(relativePath).replaceAll('\\', '/').split('/').filter(Boolean);
      let parentId = mapping.target_dir_id;
      let entry = null;
      for (let index = 0; index < parts.length; index += 1) {
        const children = await list(parentId, force);
        entry = children.find((item) => item.name === parts[index])
          || children.find((item) => item.name.toLocaleLowerCase() === parts[index].toLocaleLowerCase());
        if (!entry) return null;
        entry.parent_id = parentId;
        if (index < parts.length - 1 && !entry.is_directory) return null;
        parentId = entry.id;
      }
      return entry;
    }
    async function ensureDirectory(relativePath) {
      const parts = cleanText(relativePath).replaceAll('\\', '/').split('/').filter(Boolean);
      let parentId = mapping.target_dir_id;
      for (const name of parts) {
        let children = await list(parentId);
        let entry = children.find((item) => item.name === name)
          || children.find((item) => item.name.toLocaleLowerCase() === name.toLocaleLowerCase());
        if (entry && !entry.is_directory) throw new Error(`目标路径包含同名文件：${name}`);
        if (!entry) {
          let created = null;
          let createError = null;
          try { created = normalizeOrganizerCloudEntry(await cloud.createDirectory(parentId, name)); }
          catch (error) { createError = error; }
          cache.delete(parentId);
          children = await list(parentId, true);
          entry = (created?.id && children.find((item) => item.id === created.id))
            || children.find((item) => item.name === name)
            || children.find((item) => item.name.toLocaleLowerCase() === name.toLocaleLowerCase());
          if (!entry?.id || !entry.is_directory) {
            if (createError) throw createError;
            throw new Error(`创建云端目录后无法定位：${name}`);
          }
        }
        parentId = entry.id;
      }
      return parentId;
    }
    function invalidate(parentId) { cache.delete(parentId); }
    return { resolve, ensureDirectory, list, invalidate };
  }

  async function previewJob(id, input = {}, executeAfterPreview = false) {
    if (executingJobs.has(id)) throw new Error('该任务正在整理，请等待完成');
    const job = getJob(id);
    if (!job) throw new Error('整理任务不存在');
    const mapping = getMapping(job.mapping_id);
    if (!mapping) throw new Error('整理监控不存在');
    const loaded = await loadCloudCandidate(mapping, job.source_id);
    if (!loaded) throw new Error('待整理云端项目已经不存在');
    const overrides = resolvedJobOverrides(job, mapping, input);
    updateJob(id, {
      status: 'recognizing', source_size: loaded.fingerprint.size, source_modified_ms: loaded.fingerprint.modified_ms,
      source_file_count: loaded.fingerprint.file_count, source_signature: loaded.fingerprint.signature,
      media_type: overrides.media_type || null, tmdb_id: overrides.tmdb_id == null ? null : String(overrides.tmdb_id),
      season: overrides.season, episode: overrides.episode, episode_end: overrides.episode_end,
      query_title: overrides.title || null, query_year: overrides.year, error_code: null, result_json: null,
      message: '光鸭正在解析云盘文件名并匹配 TMDB',
    });
    emit('job-updated', { job_id: id, mapping_id: job.mapping_id, status: 'recognizing' });
    try {
      const settings = effectiveSettings();
      const analysis = analyzeCloudMediaCandidate(loaded, overrides, settings);
      if (settings.include_media_info) await enrichAnalysisWithMediaInfo(analysis, cloud, env);
      if (!overrides.title) overrides.title = analysis.title;
      if (!overrides.year) overrides.year = analysis.year;
      if (!overrides.media_type) overrides.media_type = analysis.media_type;
      const match = await resolveTmdbMatch({ analysis, client: tmdbClient(settings), settings, overrides });
      const resolver = createTargetResolver(mapping);
      const preview = await buildCloudNativePreview({
        analysis, match, mapping, settings, mappingSignature: mappingSignature(mapping, settings),
        sourceSignature: loaded.fingerprint.signature, targetExists: (relative) => resolver.resolve(relative),
      });
      const classification = classifyNativePreview(preview);
      updateJob(id, {
        status: classification.ready ? 'ready' : 'needs_review', media_type: match.query?.media_type || analysis.media_type,
        tmdb_id: match.selected?.tmdb_id == null ? (overrides.tmdb_id == null ? null : String(overrides.tmdb_id)) : String(match.selected.tmdb_id),
        season: overrides.season, episode: overrides.episode, episode_end: overrides.episode_end,
        query_title: match.query?.title || analysis.title || null, query_year: match.query?.year || analysis.year || null,
        preview_json: JSON.stringify(preview), error_code: classification.error_code, message: classification.message,
      });
      emit('job-updated', { job_id: id, mapping_id: job.mapping_id, status: classification.ready ? 'ready' : 'needs_review' });
      if (classification.ready && executeAfterPreview && getMapping(job.mapping_id)?.enabled) return executeJob(id);
      return getJob(id);
    } catch (error) {
      const reviewCodes = new Set(['tmdb_not_configured', 'tmdb_not_found', 'ambiguous_match', 'title_required', 'episode_required', 'video_required']);
      const errorCode = cleanText(error.code) || 'recognition_failed';
      const status = reviewCodes.has(errorCode) ? 'needs_review' : 'failed';
      updateJob(id, { status, error_code: errorCode, message: error.message });
      emit('job-updated', { job_id: id, mapping_id: job.mapping_id, status, message: error.message });
      return getJob(id);
    }
  }

  async function locateOperationResult(resolver, parentId, sourceId, sourceName, beforeIds, operation) {
    resolver.invalidate(parentId);
    const children = await resolver.list(parentId, true);
    const entry = operation === 'move'
      ? children.find((item) => item.id === sourceId)
      : children.find((item) => item.name === sourceName && !beforeIds.has(item.id));
    if (!entry?.id) throw new Error(`云端${operation === 'move' ? '移动' : '复制'}已完成，但无法定位目标资源`);
    entry.parent_id = parentId;
    return entry;
  }

  async function rollbackTransfers(transaction, resolver) {
    const warnings = [];
    for (const step of [...transaction].reverse()) {
      try {
        if (step.operation === 'copy') await cloud.deleteEntry(step.created_id);
        else {
          await cloud.moveEntry(step.created_id, step.source_parent_id);
          if (step.target_name !== step.source_name) await cloud.renameEntry(step.created_id, step.source_name);
        }
        if (step.backup) await cloud.renameEntry(step.backup.id, step.backup.original_name);
        resolver.invalidate(step.target_parent_id);
        resolver.invalidate(step.source_parent_id);
      } catch (error) { warnings.push(`${step.source_name} 回滚失败：${error.message}`); }
    }
    return warnings;
  }

  async function executeTransfers(preview, mapping, resolver) {
    const transferItems = preview.data.items.filter((item) => item.success && item.source_id && ['video', 'subtitle', 'audio', 'trailer', 'extra'].includes(item.kind));
    const transaction = [];
    const unattachedBackups = new Map();
    let transferred = 0;
    let skipped = 0;
    try {
      const groups = new Map();
      for (const item of transferItems) {
        const key = item.target_parent_relative;
        const group = groups.get(key) || [];
        group.push(item);
        groups.set(key, group);
      }
      for (const [targetParentRelative, items] of groups) {
        const parentId = await resolver.ensureDirectory(targetParentRelative);
        const batchOperation = mapping.transfer_type === 'move' ? cloud.moveEntries : cloud.copyEntries;
        if (items.length > 1 && typeof batchOperation === 'function') {
          const prepared = [];
          for (const item of items) {
            const existing = await resolver.resolve(item.target_relative, true);
            if (item.action === 'skip' && existing) { skipped += 1; continue; }
            if (item.action === 'create' && existing) throw new Error(`预览后目标已出现同名项目：${item.target}`);
            let backup = null;
            if (existing) {
              const backupName = `.__gy_org_backup_${crypto.randomUUID().replaceAll('-', '')}`;
              await cloud.renameEntry(existing.id, backupName);
              backup = { id: existing.id, original_name: existing.name, backup_name: backupName };
              unattachedBackups.set(backup.id, backup);
              resolver.invalidate(parentId);
            }
            prepared.push({ item, backup });
          }
          if (!prepared.length) continue;
          const before = new Set((await resolver.list(parentId, true)).map((entry) => entry.id));
          await batchOperation.call(cloud, prepared.map(({ item }) => item.source_id), parentId);
          resolver.invalidate(parentId);
          const after = await resolver.list(parentId, true);
          const availableCopies = after.filter((entry) => !before.has(entry.id));
          for (const { item, backup } of prepared) {
            const copyIndex = mapping.transfer_type === 'copy'
              ? availableCopies.findIndex((entry) => entry.name === item.source_name)
              : -1;
            const created = mapping.transfer_type === 'move'
              ? after.find((entry) => entry.id === item.source_id)
              : (copyIndex >= 0 ? availableCopies.splice(copyIndex, 1)[0] : null);
            if (!created?.id) throw new Error(`云端${mapping.transfer_type === 'move' ? '移动' : '复制'}已完成，但无法定位目标资源：${item.source_name}`);
            transaction.push({ operation: mapping.transfer_type, created_id: created.id, source_parent_id: item.source_parent_id, source_name: item.source_name, target_parent_id: parentId, target_name: item.target_name, backup });
            if (backup) unattachedBackups.delete(backup.id);
          }
          await mapWithConcurrency(transaction.slice(-prepared.length).filter((step) => step.source_name !== step.target_name), ORGANIZER_RENAME_CONCURRENCY,
            (step) => cloud.renameEntry(step.created_id, step.target_name));
          resolver.invalidate(parentId);
          transferred += prepared.length;
          continue;
        }
        for (const item of items) {
        let existing = await resolver.resolve(item.target_relative, true);
        if (item.action === 'skip' && existing) { skipped += 1; continue; }
        if (item.action === 'create' && existing) throw new Error(`预览后目标已出现同名项目：${item.target}`);
        let backup = null;
        if (existing) {
          const backupName = `.__gy_org_backup_${crypto.randomUUID().replaceAll('-', '')}`;
          await cloud.renameEntry(existing.id, backupName);
          backup = { id: existing.id, original_name: existing.name, backup_name: backupName };
          unattachedBackups.set(backup.id, backup);
          resolver.invalidate(parentId);
        }
        const before = new Set((await resolver.list(parentId, true)).map((entry) => entry.id));
        if (mapping.transfer_type === 'move') await cloud.moveEntry(item.source_id, parentId);
        else await cloud.copyEntry(item.source_id, parentId);
        const created = await locateOperationResult(resolver, parentId, item.source_id, item.source_name, before, mapping.transfer_type);
        if (created.name !== item.target_name) await cloud.renameEntry(created.id, item.target_name);
        resolver.invalidate(parentId);
        transaction.push({ operation: mapping.transfer_type, created_id: created.id, source_parent_id: item.source_parent_id, source_name: item.source_name, target_parent_id: parentId, target_name: item.target_name, backup });
        if (backup) unattachedBackups.delete(backup.id);
        transferred += 1;
      }
      }
    } catch (error) {
      const rollbackWarnings = await rollbackTransfers(transaction, resolver);
      for (const backup of unattachedBackups.values()) {
        try { await cloud.renameEntry(backup.id, backup.original_name); }
        catch (rollbackError) { rollbackWarnings.push(`恢复覆盖备份 ${backup.original_name} 失败：${rollbackError.message}`); }
      }
      throw new Error(`${error.message}${rollbackWarnings.length ? `；${rollbackWarnings.join('；')}` : ''}`);
    }
    for (const step of transaction) {
      if (!step.backup) continue;
      try { await cloud.deleteEntry(step.backup.id); }
      catch (error) { throw new Error(`新文件已落库，但清理覆盖备份失败：${error.message}`); }
    }
    return { transferred, skipped, targets: transaction.map((item) => item.created_id) };
  }

  async function generatedBytes(item, preview) {
    if (item.operation === 'generate') return { bytes: Buffer.from(renderNfo(item.generator, preview.metadata), 'utf8'), contentType: 'application/xml; charset=utf-8' };
    const settings = effectiveSettings();
    const response = await createProxiedFetch(normalizeProxyUrl(settings.tmdb_proxy, 'TMDB 代理'), fetchImpl)(item.source, { signal: AbortSignal.timeout(30_000) });
    if (!response.ok) throw new Error(`下载刮削图片失败（HTTP ${response.status}）`);
    const contentLength = Number(response.headers.get('content-length') || 0);
    if (contentLength > 25 * 1024 * 1024) throw new Error('刮削图片超过 25 MB，已跳过');
    const bytes = Buffer.from(await response.arrayBuffer());
    if (bytes.length > 25 * 1024 * 1024) throw new Error('刮削图片超过 25 MB，已跳过');
    return { bytes, contentType: response.headers.get('content-type') || 'image/jpeg' };
  }

  async function executeScrape(preview, resolver) {
    const generated = preview.data.items.filter((item) => item.success && !item.source_id && ['nfo', 'image'].includes(item.kind));
    let scraped = 0;
    let skipped = 0;
    const warnings = [];
    for (const item of generated) {
      let backup = null;
      let parentId = '';
      try {
        parentId = await resolver.ensureDirectory(item.target_parent_relative);
        const existing = await resolver.resolve(item.target_relative, true);
        if (item.action === 'skip' && existing) { skipped += 1; continue; }
        if (item.action === 'create' && existing) throw new Error('预览后目标已出现同名文件');
        if (existing) {
          const backupName = `.__gy_org_meta_${crypto.randomUUID().replaceAll('-', '')}`;
          await cloud.renameEntry(existing.id, backupName);
          backup = { id: existing.id, original_name: existing.name };
          resolver.invalidate(parentId);
        }
        const payload = await generatedBytes(item, preview);
        await cloud.uploadBuffer(parentId, item.target_name, payload.bytes, payload.contentType);
        resolver.invalidate(parentId);
        if (backup) await cloud.deleteEntry(backup.id);
        scraped += 1;
      } catch (error) {
        if (backup) {
          try { await cloud.renameEntry(backup.id, backup.original_name); resolver.invalidate(parentId); }
          catch (rollbackError) { warnings.push(`${item.target}：${error.message}；恢复旧元数据也失败：${rollbackError.message}`); continue; }
        }
        warnings.push(`${item.target}：${error.message}`);
      }
    }
    return { scraped, skipped, warnings };
  }

  async function executeJobInner(id) {
    const job = getJob(id);
    if (!job) throw new Error('整理任务不存在');
    if (job.status === 'completed' || job.status === 'completed_warning') throw new Error('该任务已经整理完成');
    const mapping = getMapping(job.mapping_id);
    if (!mapping) throw new Error('整理监控不存在');
    const classification = classifyNativePreview(job.preview);
    if (!classification.ready) throw new Error('当前任务没有可执行预览，请先重新识别');
    const settings = effectiveSettings();
    if (job.preview.mapping_signature !== mappingSignature(mapping, settings)) throw new Error('整理配置在预览后发生变化，请先重新识别');
    const loaded = await loadCloudCandidate(mapping, job.source_id);
    if (!loaded) throw new Error('待整理云端项目已经不存在');
    if (loaded.fingerprint.signature !== job.preview.source_signature || loaded.fingerprint.signature !== job.source_signature) throw new Error('待整理云端内容在预览后发生变化，请先重新识别');
    updateJob(id, { status: 'running', error_code: null, message: '光鸭正在执行云盘 A → B 原生整理' });
    emit('job-updated', { job_id: id, mapping_id: job.mapping_id, status: 'running' });
    try {
      const resolver = createTargetResolver(mapping);
      const transfer = await executeTransfers(job.preview, mapping, resolver);
      const scrape = await executeScrape(job.preview, resolver);
      const warnings = [...scrape.warnings];
      if (mapping.transfer_type === 'move') warnings.push('云端移动会使来源资源的已有分享失效；光鸭没有复用来源分享');
      let share = null;
      const shouldShare = job.share_after_requested || mapping.share_after_organize;
      if (shouldShare) {
        try {
          const targetId = await resolver.ensureDirectory(job.preview.share_relative_path);
          if (typeof cloud.shareAfterOrganize !== 'function') throw new Error('当前运行端未接入整理后分享');
          share = await cloud.shareAfterOrganize({ mappingId: mapping.id, remoteTargetId: targetId, title: job.preview.share_title, targetType: 'folder' });
        } catch (error) { warnings.push(`整理已完成，但创建 B 目录新分享失败：${error.message}`); }
      }
      const result = { success: true, transferred: transfer.transferred, skipped: transfer.skipped + scrape.skipped, scraped: scrape.scraped, warnings, targets: transfer.targets, share };
      const status = warnings.length ? 'completed_warning' : 'completed';
      const message = `云盘整理完成：转移 ${result.transferred} 项，刮削 ${result.scraped} 项${share ? '，已从 B 目录重新分享' : ''}${warnings.length ? `；${warnings.length} 项提示` : ''}`;
      updateJob(id, { status, error_code: warnings.length ? 'completed_warning' : null, result_json: JSON.stringify(result), message });
      emit('job-updated', { job_id: id, mapping_id: job.mapping_id, status });
      return getJob(id);
    } catch (error) {
      updateJob(id, { status: 'failed', error_code: 'transfer_failed', message: error.message });
      emit('job-updated', { job_id: id, mapping_id: job.mapping_id, status: 'failed', message: error.message });
      return getJob(id);
    }
  }

  async function executeJob(id) {
    if (executingJobs.has(id)) throw new Error('该任务正在整理，请勿重复执行');
    const job = getJob(id);
    if (!job) throw new Error('整理任务不存在');
    if (mutatingMappings.has(job.mapping_id)) throw new Error('云盘目录配置正在变更，请稍后执行整理');
    executingJobs.add(id);
    try { return await executeJobInner(id); } finally { executingJobs.delete(id); }
  }

  async function shareJob(id) {
    if (executingJobs.has(id)) throw new Error('该任务正在整理，请等待完成');
    const job = getJob(id);
    if (!job) throw new Error('整理任务不存在');
    if (!['completed', 'completed_warning'].includes(job.status)) throw new Error('只有已完成的整理任务才能创建分享');
    const mapping = getMapping(job.mapping_id);
    if (!mapping) throw new Error('整理监控不存在');
    const relativePath = cleanText(job.preview?.share_relative_path);
    if (!relativePath) throw new Error('整理预览缺少最终媒体目录，无法创建分享');
    if (typeof cloud.shareAfterOrganize !== 'function') throw new Error('当前运行端未接入整理后分享');
    const resolver = createTargetResolver(mapping);
    const target = await resolver.resolve(relativePath, true);
    if (!target?.id || !target.is_directory) throw new Error('最终媒体目录已经不存在，无法创建分享');
    const share = await cloud.shareAfterOrganize({
      mappingId: mapping.id,
      remoteTargetId: target.id,
      title: cleanText(job.preview?.share_title) || '整理后的媒体',
      targetType: 'folder',
    });
    const result = { ...(job.result || {}), success: true, share };
    updateJob(id, { result_json: JSON.stringify(result) });
    emit('job-updated', { job_id: id, mapping_id: mapping.id, status: job.status, shared: true });
    return share;
  }

  async function runJob(id, input = {}) {
    const job = getJob(id);
    if (!job) throw new Error('整理任务不存在');
    if (job.status === 'completed' || job.status === 'completed_warning') throw new Error('该任务已经整理完成');
    const mapping = getMapping(job.mapping_id);
    if (!mapping) throw new Error('整理监控不存在');
    const hasOverrides = ['media_type', 'tmdb_id', 'title', 'year', 'season', 'episode', 'episode_end'].some((key) => input[key] !== undefined)
      || Object.keys(input).some((key) => key.startsWith('clear_') && input[key] === true);
    const loaded = await loadCloudCandidate(mapping, job.source_id);
    const sourceChanged = !loaded || loaded.fingerprint.signature !== job.preview?.source_signature;
    const configChanged = job.preview?.mapping_signature !== mappingSignature(mapping);
    if (hasOverrides || sourceChanged || configChanged || !classifyNativePreview(job.preview).ready) return previewJob(id, input, true);
    return executeJob(id);
  }

  async function retryJob(id, input = {}) {
    const job = getJob(id);
    if (!job) throw new Error('整理任务不存在');
    if (job.status === 'running') throw new Error('该任务正在整理，请等待完成');
    return previewJob(id, input, false);
  }

  async function rearchiveJob(id, input = {}) {
    const job = getJob(id);
    if (!job) throw new Error('整理任务不存在');
    if (['recognizing', 'running'].includes(job.status) || executingJobs.has(id)) throw new Error('该任务正在整理，请等待完成');
    const mapping = getMapping(job.mapping_id);
    if (!mapping) throw new Error('整理监控不存在');
    const active = database.prepare(`${jobSelect()} WHERE mapping_id = ? AND source_id = ? AND id <> ?
      AND status IN ('recognizing','ready','running','needs_review') ORDER BY updated_at DESC LIMIT 1`)
      .get(job.mapping_id, job.source_id, id);
    if (active) throw new Error('该源文件已有待处理的归档任务，请先处理后再重新归档');
    updateJob(id, { status: 'recognizing', error_code: null, message: '重新归档已提交，正在后台识别' });
    emit('job-updated', {
      job_id: id,
      mapping_id: job.mapping_id,
      status: 'recognizing',
      rearchive: true,
    });
    void (async () => {
      try {
        const recognized = await previewJob(id, input, false);
        if (recognized?.status === 'ready') await executeJob(id);
      } catch (error) {
        updateJob(id, { status: 'failed', error_code: 'rearchive_failed', message: error.message });
        emit('job-updated', { job_id: id, mapping_id: job.mapping_id, status: 'failed', message: error.message });
      }
    })();
    return getJob(id);
  }

  async function processCandidate(mappingId, candidateId, shareAfterOverride = null) {
    if (mutatingMappings.has(mappingId)) return null;
    const mapping = getMapping(mappingId);
    if (!mapping || !mapping.enabled) return null;
    const key = `${mappingId}::${candidateId}`;
    if (runningCandidates.has(key)) return null;
    runningCandidates.add(key);
    try {
      const first = await loadCloudCandidate(mapping, candidateId);
      if (!first || first.fingerprint.video_count < 1) return null;
      await new Promise((resolve) => setTimeout(resolve, 2000));
      const stable = await loadCloudCandidate(mapping, candidateId);
      if (!stable || stable.fingerprint.signature !== first.fingerprint.signature) {
        scheduleCandidate(mapping, first.candidate, { immediate: false, signature: stable?.fingerprint.signature || '', shareAfter: shareAfterOverride });
        return null;
      }
      const duplicate = database.prepare(`${jobSelect()} WHERE mapping_id = ? AND source_id = ? AND source_signature = ?
        AND status IN ('recognizing','ready','running','completed','completed_warning','needs_review') ORDER BY updated_at DESC LIMIT 1`)
        .get(mappingId, candidateId, first.fingerprint.signature);
      if (duplicate) {
        if (shareAfterOverride && !duplicate.share_after_requested) database.prepare('UPDATE organizer_jobs SET share_after_requested = 1, updated_at = ? WHERE id = ?').run(nowSeconds(), duplicate.id);
        return getJob(duplicate.id);
      }
      const id = crypto.randomUUID();
      const timestamp = nowSeconds();
      const displayPath = `${mapping.source_path.replace(/\/$/, '')}/${first.candidate.name}`;
      database.prepare(`INSERT INTO organizer_jobs
        (id, mapping_id, source_path, source_id, source_parent_id, source_size, source_modified_ms, source_file_count,
         source_signature, share_after_requested, status, media_type, message, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'recognizing', ?, '等待光鸭云盘原生识别', ?, ?)`)
        .run(id, mappingId, displayPath, candidateId, mapping.source_dir_id, first.fingerprint.size,
          first.fingerprint.modified_ms, first.fingerprint.file_count, first.fingerprint.signature,
          Number(shareAfterOverride == null ? mapping.share_after_organize : shareAfterOverride), mapping.media_type || null, timestamp, timestamp);
      emit('job-updated', { job_id: id, mapping_id: mappingId, status: 'recognizing' });
      await previewJob(id, {}, mapping.auto_execute);
      return getJob(id);
    } finally { runningCandidates.delete(key); }
  }

  async function notifyUpload({ mappingId, remoteFileId = '', relativePath = '', shareAfter = false }) {
    const mapping = getMapping(mappingId);
    if (!mapping || !mapping.enabled) throw new Error('上传任务关联的云盘整理监控不存在或未启用');
    const topName = cleanText(relativePath).replaceAll('\\', '/').split('/').filter(Boolean)[0] || '';
    const result = await scanMapping(mappingId, { immediate: false, shareAfter, candidateName: topName, candidateId: topName ? '' : cleanText(remoteFileId) });
    if (!result.queued) throw new Error('上传已入库，但尚未在整理 A 目录定位到对应媒体项目');
    return result;
  }

  async function initialize() {
    assertCloudAdapter();
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = setInterval(() => void pollMappings(), POLL_INTERVAL_MS);
    if (cloud.isAuthenticated()) {
      for (const mapping of listMappings().filter((item) => item.enabled)) {
        try { await validateMapping(mapping); if (mapping.scan_existing) await scanMapping(mapping.id); }
        catch (error) {
          database.prepare('UPDATE organizer_mappings SET watch_error = ?, updated_at = ? WHERE id = ?').run(error.message, nowSeconds(), mapping.id);
          emit('mapping-error', { mapping_id: mapping.id, message: error.message });
        }
      }
    }
  }

  async function close() {
    if (pollTimer) clearInterval(pollTimer);
    pollTimer = null;
    for (const pending of pendingTimers.values()) clearTimeout(pending.timer);
    pendingTimers.clear();
  }

  return { state, updateSettings, testConnection, addMapping, updateMapping, removeMapping, removeJob, scanMapping, runJob, retryJob, rearchiveJob, shareJob, notifyUpload, scrapeSelected, initialize, close };
}

export const organizerInternals = {
  ACTIVE_STATUSES,
  SCRAPE_TYPES,
  initializeSchema,
  normalizeConflictPolicy,
  normalizeMappingInput: normalizeOrganizerMappingInput,
  normalizeMediaType,
  normalizeScrapeTypes,
  normalizeSettleSeconds,
  normalizeTransferType,
  bindConfiguredOutputTarget,
  parseFfprobeTechnicalData,
};
