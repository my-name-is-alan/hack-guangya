import { countryNameZh } from './countries.js';

export const organizerStatusMeta = Object.freeze({
  recognizing: { label: '识别中', color: 'processing' },
  ready: { label: '待执行', color: 'blue' },
  running: { label: '整理中', color: 'processing' },
  completed: { label: '已完成', color: 'success' },
  completed_warning: { label: '完成有提示', color: 'warning' },
  needs_review: { label: '需人工确认', color: 'warning' },
  failed: { label: '失败', color: 'error' },
});

function rulePatternSource(value) {
  let source = String(value || '').trim();
  let insensitive = false;
  if (source.startsWith('(?i)')) {
    source = source.slice(4);
    insensitive = true;
  }
  return { source, insensitive };
}

export function validateOrganizerRuleBlock(value, label, { replacement = true } = {}) {
  const normalized = String(value || '').replace(/\r\n?/g, '\n').trim();
  if (normalized.length > 100_000) throw new Error(`${label}不能超过 100000 个字符`);
  const lines = normalized ? normalized.split('\n') : [];
  if (lines.length > 2_000) throw new Error(`${label}不能超过 2000 行`);
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index].trim();
    if (!line || line.startsWith('#')) continue;
    const arrow = replacement ? line.indexOf('=>') : -1;
    const patternText = arrow >= 0 ? line.slice(0, arrow).trim() : line;
    if (!patternText) throw new Error(`${label}第 ${index + 1} 行缺少正则表达式`);
    if (patternText.startsWith('@?{')) throw new Error(`${label}第 ${index + 1} 行使用了尚未支持的 @? 条件规则`);
    const { source, insensitive } = rulePatternSource(patternText);
    if (/\\[1-9]|\\k<|\(\?(?:[=!]|<[=!]|P?<)/.test(source)) {
      throw new Error(`${label}第 ${index + 1} 行使用了统一规则语法不支持的前后查找、正则内反向引用或命名捕获`);
    }
    try {
      new RegExp(source, `${insensitive ? 'i' : ''}u`);
    } catch (error) {
      throw new Error(`${label}第 ${index + 1} 行正则无效：${error.message}`);
    }
  }
  return normalized;
}

function normalizedRuleTerms(value, casing = 'lower') {
  const source = Array.isArray(value) ? value : String(value || '').split(/[,，\n]/);
  const normalized = source.map((item) => String(item || '').normalize('NFKC').trim()).filter(Boolean).map((item) => {
    if (casing === 'upper') return item.toUpperCase();
    return item.toLowerCase();
  });
  return [...new Set(normalized)].slice(0, 80);
}

export function normalizeOrganizerCategoryFormRules(value) {
  if (!Array.isArray(value)) return [];
  if (value.length > 100) throw new Error('二级分类不能超过 100 条');
  const usedIds = new Set();
  return value.map((rule, index) => {
    const parts = String(rule?.name || '').replaceAll('\\', '/').split('/').map((part) => part.trim()).filter(Boolean);
    if (!parts.length || parts.length > 8 || parts.some((part) => part === '.' || part === '..' || part.length > 80)) {
      throw new Error(`第 ${index + 1} 条媒体分类名称无效`);
    }
    const genres = normalizedRuleTerms(rule?.genres);
    const originalLanguages = normalizedRuleTerms(rule?.original_languages);
    const originCountries = normalizedRuleTerms(rule?.origin_countries, 'upper');
    if (!genres.length && !originalLanguages.length && !originCountries.length) {
      throw new Error(`第 ${index + 1} 条媒体分类至少配置一个类型、原始语言或来源地区`);
    }
    let id = String(rule?.id || `category-${index + 1}`).trim() || `category-${index + 1}`;
    if (usedIds.has(id)) id = `category-${index + 1}`;
    while (usedIds.has(id)) id = `${id}-${index + 1}`;
    usedIds.add(id);
    const mediaType = ['movie', 'tv', 'all'].includes(String(rule?.media_type || '').toLowerCase())
      ? String(rule.media_type).toLowerCase() : 'all';
    return {
      id,
      name: parts.join('/'),
      media_type: mediaType,
      genres,
      original_languages: originalLanguages,
      origin_countries: originCountries,
      enabled: rule?.enabled !== false,
    };
  });
}

export function organizerStatus(status) {
  return organizerStatusMeta[status] || { label: String(status || '未知'), color: 'default' };
}

export function organizerPreviewItems(job) {
  const items = job?.preview?.data?.items;
  return Array.isArray(items) ? items : [];
}

export function organizerPreviewTarget(job) {
  const items = organizerPreviewItems(job).filter((item) => item?.success && item?.target);
  const targets = items.filter((item) => item.kind === 'video').map((item) => String(item.target));
  const fallback = items.map((item) => String(item.target));
  const selected = targets.length ? targets : fallback;
  if (!selected.length) return '';
  return selected.length === 1 ? selected[0] : `${selected[0]} 等 ${selected.length} 项`;
}

export function organizerCandidates(job) {
  return Array.isArray(job?.preview?.candidates) ? job.preview.candidates : [];
}

export function organizerMatchedTitle(job) {
  const metadata = job?.preview?.metadata;
  if (!metadata?.title) return '';
  return `${metadata.title}${metadata.year ? ` (${metadata.year})` : ''}`;
}

export function organizerMediaLabel(value) {
  if (value === 'movie') return '电影';
  if (value === 'tv') return '电视剧';
  return '自动识别';
}

export function organizerTransferLabel(value) {
  return {
    copy: '云盘内复制',
    move: '云盘内移动',
  }[value] || '云盘内复制';
}

export function organizerConflictLabel(value) {
  return {
    skip: '跳过已有文件',
    overwrite: '覆盖已有文件',
    rename: '保留两份',
  }[value] || '跳过已有文件';
}

export function organizerItemKindLabel(value) {
  return {
    video: '视频',
    subtitle: '字幕',
    audio: '外置音轨',
    trailer: '预告片',
    extra: '附加视频',
    nfo: 'NFO',
    image: '图片',
  }[value] || String(value || '文件');
}

export function organizerItemActionLabel(item) {
  if (!item?.success) return '不可执行';
  if (item.action === 'skip') return '跳过';
  if (item.action === 'overwrite') return '覆盖';
  if (item.operation === 'generate') return '生成';
  if (item.operation === 'download') return '下载';
  return organizerTransferLabel(item.operation);
}

function renderTemplateExample(template, context) {
  const normalizedContext = Object.fromEntries(Object.entries(context || {}).map(([key, value]) => [key.toLowerCase(), value]));
  const conditional = String(template || '').replace(/\{\{@if@\}\}([\s\S]*?)\{\{@endif@\}\}/gi, (_, body) => {
    const keys = [...body.matchAll(/\{\{\s*([a-z_][a-z0-9_]*)\s*\}\}/gi)].map((match) => match[1].toLowerCase());
    return keys.length && keys.every((key) => normalizedContext[key] !== '' && normalizedContext[key] !== null && normalizedContext[key] !== undefined) ? body : '';
  });
  const aliases = conditional
    .replace(/\{\{\s*([a-z_][a-z0-9_]*)\s*\}\}/gi, '{$1}')
    .replace(/\{catgroy\}/gi, '{category}')
    .replace(/\{tmdbid\}/gi, '{tmdb_id}')
    .replace(/\{Season\s+x\}/gi, '{season_tag}')
    .replace(/\{(?:Episode|Expose)\s+n\}/gi, '{episode_tag}');
  const rendered = aliases.replace(/\{([a-z_]+)(?::(\d+))?\}/gi, (_, key, width) => {
    const value = context[String(key).toLowerCase()] ?? '';
    return width && value !== '' ? String(value).padStart(Number(width), '0') : String(value);
  });
  return rendered.replaceAll('\\', '/').split('/').map((part) => part.trim()).filter(Boolean).join('/');
}

const TECHNICAL_TEMPLATE_TOKEN = /\{\{?\s*(?:media_info|video_?format|resource_?type|source|effect|audio_?info|video_?codec|audio_?codec|release_?group|release_?type|high_?quality|dolby_?vision|dynamic_?range|frame_?rate|color_?depth)\s*\}?\}/i;

function appendExampleMediaInfo(value, template, mediaInfo, enabled) {
  if (!enabled || !mediaInfo || TECHNICAL_TEMPLATE_TOKEN.test(template)) return value;
  const lastSlash = value.lastIndexOf('/');
  const lastDot = value.lastIndexOf('.');
  const extensionIndex = lastDot > lastSlash ? lastDot : value.length;
  return `${value.slice(0, extensionIndex)}.${mediaInfo}${value.slice(extensionIndex)}`;
}

function splitExamplePath(value) {
  const parts = String(value || '').split('/').filter(Boolean);
  return {
    path: value,
    directory: parts.slice(0, -1).join('/'),
    filename: parts.at(-1) || '',
  };
}

export function organizerTemplateExamples(movieTemplate, tvTemplate, movieCategory = '电影', tvCategory = '电视剧', includeMediaInfo = true) {
  const movieMediaInfo = '1080p.WEB-DL.HDR.5.1.HEVC.DDP-Example';
  const movie = splitExamplePath(appendExampleMediaInfo(renderTemplateExample(movieTemplate, {
    category: movieCategory, country: countryNameZh('US'), country_code: 'US', release_country: countryNameZh('US'), year: 2024, title: '示例电影', original_title: 'Example Movie', original_filename: 'Example.Movie.2024.1080p.WEB-DL', tmdb_id: 12345,
    en_title: 'Example Movie', edition: '', quality: ' - 1080p', part: '', ext: 'mkv', fileext: '.mkv',
    season: '', episode: '', season_tag: '', episode_tag: '', season_episode: '', episode_end: '', episode_title: '',
    video_format: '1080p', videoformat: '1080p', resource_type: 'WEB-DL', resourcetype: 'WEB-DL', source: 'AMZN',
    effect: 'HDR', audio_info: '5.1', audioinfo: '5.1', video_codec: 'HEVC', videocodec: 'HEVC',
    audio_codec: 'DDP', audiocodec: 'DDP', release_group: 'Example', releasegroup: 'Example', media_info: movieMediaInfo,
  }), movieTemplate, movieMediaInfo, includeMediaInfo));
  movie.input = '示例电影.2024.1080p.WEB-DL.mkv';
  const tvMediaInfo = '2160p.Netflix.WEB-DL.HDR.60fps.10bit.HEVC.DDP-Example';
  const tv = splitExamplePath(appendExampleMediaInfo(renderTemplateExample(tvTemplate, {
    category: tvCategory, country: countryNameZh('CN'), country_code: 'CN', release_country: countryNameZh('CN'), year: 2024, season_year: 2024, title: '示例剧集', original_title: 'Example Series', original_filename: 'Example.Series.S01E02.2160p.WEB-DL', tmdb_id: 67890,
    en_title: 'Example Series', edition: '', quality: ' - 2160p', part: '', ext: 'mkv', fileext: '.mkv',
    season: 1, episode: 2, season_tag: 'S01', episode_tag: 'E02', season_episode: 'S01E02', episode_end: '', episode_title: '第二集',
    video_format: '2160p', videoformat: '2160p', resource_type: 'WEB-DL', resourcetype: 'WEB-DL', source: 'Netflix',
    effect: 'HDR', audio_info: '5.1', audioinfo: '5.1', video_codec: 'HEVC', videocodec: 'HEVC',
    audio_codec: 'DDP', audiocodec: 'DDP', release_group: 'Example', releasegroup: 'Example', release_type: 'WEB-DL', dynamic_range: 'HDR', frame_rate: '60fps', color_depth: '10bit', media_info: tvMediaInfo,
  }), tvTemplate, tvMediaInfo, includeMediaInfo));
  tv.input = '示例剧集.S01E02.1080p.WEB-DL.mkv';
  return { movie, tv };
}

const COMMON_TEMPLATE_TOKENS = [
  ['title', '标题 / 剧名'], ['en_title', '英文标题'], ['original_title', '原语种标题'], ['original_filename', '原文件名'],
  ['year', '年份'], ['segment', '段 / 节'], ['video_format', '分辨率'], ['video_codec', '视频编码'], ['frame_rate', '帧率'],
  ['video_codec_frame_rate_high_quality', '视频编码、帧率与高品质'], ['audio_codec', '音频编码'], ['audio_info', '音频声道 / Atmos'],
  ['resource_type', '来源类型'], ['source', '流媒体'], ['source_platform', '来源与流媒体'], ['effect', '特效'], ['version', '版本'],
  ['effect_version', '特效与版本'], ['remux', 'REMUX'], ['version_number', '版本号'], ['dolby_vision', '杜比视界'],
  ['dynamic_range', '动态范围'], ['high_quality', '高品质'], ['color_depth', '色彩深度'], ['release_country', '发行地区'],
  ['release_group', '发布组'], ['fileext', '扩展名'], ['country', '中文国家'], ['country_code', '国家代码'], ['category', '二级分类'],
  ['tmdbid', 'TMDB ID'], ['media_info', '完整媒体信息后缀'],
  ['@if@', 'if 前缀'], ['@endif@', 'if 后缀'],
];

export const ORGANIZER_MOVIE_TEMPLATE_TOKENS = Object.freeze(COMMON_TEMPLATE_TOKENS.map(([key, label]) => ({ key, label, token: key.startsWith('@') ? `{{${key}}}` : `{${key}}` })));
export const ORGANIZER_TV_TEMPLATE_TOKENS = Object.freeze([
  ...ORGANIZER_MOVIE_TEMPLATE_TOKENS,
  ...[['season_year', '季年份'], ['season', '季数'], ['episode', '集数'], ['season_episode', '季集'], ['episode_title', '剧集标题']]
    .map(([key, label]) => ({ key, label, token: `{${key}}` })),
]);
