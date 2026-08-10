import crypto from 'node:crypto';
import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { fetch as undiciFetch } from 'undici';
import COUNTRY_NAMES_ZH from '../shared/countries-zh.json' with { type: 'json' };

export const NATIVE_ENGINE_VERSION = 'guangya-cloud-native-v3';

export const VIDEO_EXTENSIONS = new Set([
  '.3gp', '.asf', '.avi', '.f4v', '.flv', '.iso', '.m2ts', '.m4v', '.mkv', '.mov', '.mp4', '.mpeg', '.mpg', '.mts', '.rm', '.rmvb', '.strm', '.tp', '.ts', '.vob', '.webm', '.wmv',
]);

export const SUBTITLE_EXTENSIONS = new Set(['.ass', '.idx', '.smi', '.srt', '.ssa', '.sub', '.sup', '.vtt']);
export const AUDIO_EXTENSIONS = new Set(['.aac', '.ac3', '.dts', '.eac3', '.flac', '.m4a', '.mka', '.mp3', '.ogg', '.opus', '.wav']);

const IGNORED_NAMES = new Set(['@eadir', '#recycle', '$recycle.bin', 'system volume information']);
const RELEASE_WORDS = new Set([
  'bluray', 'blu-ray', 'bdrip', 'brrip', 'web', 'webdl', 'web-dl', 'webrip', 'hdtv', 'dvdrip', 'hdrip', 'remux',
  'x264', 'x265', 'h264', 'h265', 'hevc', 'avc', 'av1', '10bit', '8bit', 'hdr', 'hdr10', 'hdr10plus', 'dv', 'dolbyvision',
  'aac', 'ac3', 'eac3', 'ddp', 'ddp5', 'dts', 'dtshd', 'truehd', 'atmos', 'flac', 'mp3', 'proper', 'repack', 'rerip',
  'complete', 'internal', 'subbed', 'dubbed', 'multi', 'dual', '国语', '国英双语', '中英字幕', '中字', '简繁',
]);
const EDITION_PATTERNS = [
  ['Director’s Cut', /director(?:'|’)?s[ ._-]*cut/i],
  ['Extended Cut', /extended(?:[ ._-]*(?:cut|edition|version))?/i],
  ['IMAX', /\bimax\b/i],
  ['Unrated', /\bunrated\b|\bunrate\b/i],
  ['Uncut', /\buncut\b/i],
  ['Remastered', /\bremaster(?:ed)?\b/i],
  ['Theatrical Cut', /theatrical(?:[ ._-]*cut)?/i],
  ['Special Edition', /special[ ._-]*edition/i],
];

export const DEFAULT_ORGANIZER_SETTINGS = Object.freeze({
  language: 'zh-CN',
  image_language: 'zh,null,en',
  include_adult: false,
  minimum_match_score: 0.72,
  word_segment_search: true,
  similarity_match: true,
  recognition_words: '',
  release_groups: '',
  render_words: '',
  capture_groups: '',
  include_media_info: true,
  movie_folder_format: '{title} ({year})',
  movie_file_format: '{title} ({year}){edition}{quality}{part}',
  tv_folder_format: '{title} ({year})',
  season_folder_format: 'Season {season:02}',
  episode_file_format: '{title} - S{season:02}E{episode:02}{episode_end} - {episode_title}',
  movie_path_template: '{category}/{country}/{year}/{title} ({year}) [tmdb-{tmdb_id}]/{title} ({year}){edition}{quality}{part}.{ext}',
  tv_path_template: '{category}/{country}/{year}/{title} ({year}) [tmdb-{tmdb_id}]/Season {season:02}/{title}.S{season:02}E{episode:02}{episode_end}.{ext}',
  movie_category: '电影',
  tv_category: '电视剧',
});

export const DEFAULT_SCRAPE_TYPES = Object.freeze([
  'movie_nfo',
  'tvshow_nfo',
  'poster',
  'fanart',
]);

export const DEFAULT_CATEGORY_RULES = Object.freeze([]);

function normalizeCategoryTerm(value) {
  return cleanText(value).normalize('NFKC').toLocaleLowerCase();
}

function normalizeCategoryTerms(value) {
  const source = Array.isArray(value) ? value : String(value || '').split(/[,，\n]/);
  return [...new Set(source.map(normalizeCategoryTerm).filter(Boolean))].slice(0, 80);
}

function normalizeCategoryPath(value, index) {
  const parts = cleanText(value).replaceAll('\\', '/').split('/').map((part) => part.trim()).filter(Boolean);
  if (!parts.length || parts.length > 8 || parts.some((part) => part === '.' || part === '..' || part.length > 80)) {
    throw new Error(`第 ${index + 1} 条媒体分类名称无效`);
  }
  return parts.join('/');
}

/**
 * User-defined category rules are deliberately small and deterministic: the
 * first enabled rule whose TMDB genre name or id matches wins.
 */
export function normalizeCategoryRules(value) {
  const source = Array.isArray(value) ? value : [];
  return source.slice(0, 100).map((rule, index) => {
    const name = normalizeCategoryPath(rule?.name, index);
    const mediaType = ['movie', 'tv', 'all', ''].includes(cleanText(rule?.media_type).toLowerCase())
      ? (cleanText(rule?.media_type).toLowerCase() || 'all') : 'all';
    const genres = normalizeCategoryTerms(rule?.genres ?? rule?.genre_ids ?? rule?.genre_text);
    const originalLanguages = normalizeCategoryTerms(rule?.original_languages ?? rule?.original_language);
    const originCountries = normalizeCategoryTerms(rule?.origin_countries ?? rule?.origin_country);
    if (!genres.length && !originalLanguages.length && !originCountries.length) {
      throw new Error(`第 ${index + 1} 条媒体分类至少配置一个类型、原始语言或来源地区`);
    }
    return {
      id: cleanText(rule?.id) || `category-${index + 1}`,
      name,
      media_type: mediaType,
      genres,
      original_languages: originalLanguages,
      origin_countries: originCountries,
      enabled: rule?.enabled !== false,
    };
  });
}

export function resolveMediaCategory(metadata = {}, settings = DEFAULT_ORGANIZER_SETTINGS) {
  const mediaType = metadata.media_type === 'tv' ? 'tv' : 'movie';
  const genreNames = (metadata.genres || []).map(normalizeCategoryTerm);
  const genreIds = (metadata.genre_ids || []).map((value) => normalizeCategoryTerm(value));
  const available = new Set([...genreNames, ...genreIds]);
  const originalLanguage = normalizeCategoryTerm(metadata.original_language);
  const originCountries = new Set((metadata.origin_countries || metadata.countries || []).map(normalizeCategoryTerm));
  const rules = normalizeCategoryRules(settings.category_rules || []);
  for (const rule of rules) {
    if (!rule.enabled || (rule.media_type !== 'all' && rule.media_type !== mediaType)) continue;
    const genreMatches = !rule.genres.length
      || rule.genres.some((term) => available.has(term) || genreNames.some((name) => name.includes(term) || term.includes(name)));
    const languageMatches = !rule.original_languages.length || rule.original_languages.includes(originalLanguage);
    const countryMatches = !rule.origin_countries.length || rule.origin_countries.some((country) => originCountries.has(country));
    if (genreMatches && languageMatches && countryMatches) return rule.name;
  }
  return mediaType === 'tv'
    ? (cleanText(settings.tv_category) || DEFAULT_ORGANIZER_SETTINGS.tv_category)
    : (cleanText(settings.movie_category) || DEFAULT_ORGANIZER_SETTINGS.movie_category);
}

export function buildStandardTemplateExamples(settings = DEFAULT_ORGANIZER_SETTINGS) {
  const movieMetadata = { media_type: 'movie', title: '示例电影', original_title: 'Example Movie', year: 2024, tmdb_id: 12345, countries: ['US'], genres: [] };
  const tvMetadata = { media_type: 'tv', title: '示例剧集', original_title: 'Example Series', year: 2024, tmdb_id: 67890, countries: ['CN'], genres: [] };
  const movieCategory = resolveMediaCategory(movieMetadata, settings);
  const tvCategory = resolveMediaCategory(tvMetadata, settings);
  const movieContext = { ...templateContext(movieMetadata, { original: 'Example.Movie.2024.1080p.WEB-DL', media_probed: true, edition: '', quality: '1080p', video_format: '1080p', resource_type: 'WEB-DL', audio_info: '5.1', video_codec: 'HEVC', audio_codec: 'DDP', release_group: 'Example', part: '' }), category: movieCategory, catgroy: movieCategory, ext: 'mkv', fileExt: '.mkv' };
  const tvContext = { ...templateContext(tvMetadata, { original: 'Example.Series.S01E02.2160p.WEB-DL', media_probed: true, season: 1, episode: 2, episode_end: null, video_format: '2160p', source: 'Netflix', release_type: 'WEB-DL', dynamic_range: 'HDR', frame_rate: '60fps', color_depth: '10bit', video_codec: 'HEVC', audio_codec: 'DDP', release_group: 'Example' }, { name: '第二集' }), category: tvCategory, catgroy: tvCategory, ext: 'mkv', fileExt: '.mkv' };
  const movieTemplate = settings.movie_path_template || DEFAULT_ORGANIZER_SETTINGS.movie_path_template;
  const tvTemplate = settings.tv_path_template || DEFAULT_ORGANIZER_SETTINGS.tv_path_template;
  const moviePath = appendMediaInfoSuffix(renderOrganizerPathTemplate(movieTemplate, movieContext), movieTemplate, movieContext, settings.include_media_info !== false);
  const tvPath = appendMediaInfoSuffix(renderOrganizerPathTemplate(tvTemplate, tvContext), tvTemplate, tvContext, settings.include_media_info !== false);
  return {
    movie: { input: '示例电影.2024.1080p.WEB-DL.mkv', path: moviePath, directory: moviePath.split('/').slice(0, -1).join('/'), filename: moviePath.split('/').at(-1) },
    tv: { input: '示例剧集.S01E02.1080p.WEB-DL.mkv', path: tvPath, directory: tvPath.split('/').slice(0, -1).join('/'), filename: tvPath.split('/').at(-1) },
  };
}

export const ORGANIZER_PATH_PRESETS = Object.freeze([
  {
    id: 'reference-media-info',
    name: '参考完整命名（媒体信息后缀）',
    movie: '{category}/{country}/{title} ({year}) {tmdb-{tmdbid}}/{en_title}.{year}.{videoFormat}.{resourceType}.{effect}.{audioInfo}.{videoCodec}.{audioCodec}-{releaseGroup}{fileExt}',
    tv: '{category}/{country}/{title} ({year}) {tmdb-{tmdbid}}/Season {season}/{en_title}.{year}.{season_episode}.{videoFormat}.{source}.{release_type}.{high_quality}.{dolby_vision}.{dynamic_range}.{frame_rate}.{color_depth}.{video_codec}.{audioCodec}-{releaseGroup}{fileExt}',
  },
  {
    id: 'category-country-year',
    name: '分类 / 国家 / 年份',
    movie: DEFAULT_ORGANIZER_SETTINGS.movie_path_template,
    tv: DEFAULT_ORGANIZER_SETTINGS.tv_path_template,
  },
  {
    id: 'media-server',
    name: '媒体服务器常用',
    movie: '{category}/{title} ({year}) [tmdb-{tmdb_id}]/{title} ({year}){edition}{quality}{part}.{ext}',
    tv: '{category}/{title} ({year}) [tmdb-{tmdb_id}]/Season {season:02}/{title}.S{season:02}E{episode:02}{episode_end}.{ext}',
  },
  {
    id: 'compact',
    name: '精简目录',
    movie: '{category}/{title} ({year})/{title}.{year}.{quality}.{ext}',
    tv: '{category}/{title} ({year})/S{season:02}/{title}.S{season:02}E{episode:02}{episode_end}.{ext}',
  },
]);

function cleanText(value) {
  return String(value ?? '').trim();
}

function pathName(value) {
  return cleanText(value).replaceAll('\\', '/').split('/').filter(Boolean).at(-1) || '';
}

function stemOf(value) {
  const name = pathName(value);
  const extension = path.extname(name);
  return extension ? name.slice(0, -extension.length) : name;
}

function normalizeSpaces(value) {
  return cleanText(value)
    .replace(/[._]+/g, ' ')
    .replace(/\s+/g, ' ')
    .replace(/^[\s\-–—]+|[\s\-–—]+$/g, '')
    .trim();
}

function stripTechnicalBrackets(value) {
  return value
    .replace(/^\s*\[[^\]]{1,80}\]\s*/g, ' ')
    .replace(/[\[{(](?:2160p|1080p|720p|480p|4k|uhd|hdr10?\+?|dv|dolby[ ._-]*vision|x26[45]|h26[45]|hevc|av1|web[ ._-]*dl|webrip|bluray|remux|aac|dts|truehd|flac|中字|中英字幕|简繁)[^\]})]*[\]})]/gi, ' ');
}

function releaseQuality(value) {
  const resolution = value.match(/\b(2160p|1080p|720p|480p|4k|uhd)\b/i)?.[1];
  const source = value.match(/\b(BluRay|Blu-Ray|REMUX|WEB[ ._-]?DL|WEBRip|HDTV|BDRip|BRRip|DVDRip)\b/i)?.[1];
  const codec = value.match(/\b(x265|x264|H\.?(?:265|264)|HEVC|AV1)\b/i)?.[1];
  return [resolution, source, codec]
    .filter(Boolean)
    .map((item) => item.replace(/[ ._-]+/g, '-'))
    .filter((item, index, list) => list.findIndex((other) => other.toLowerCase() === item.toLowerCase()) === index)
    .join(' ');
}

function releaseEdition(value) {
  return EDITION_PATTERNS.find(([, pattern]) => pattern.test(value))?.[0] || '';
}

function releasePart(value) {
  const match = value.match(/(?:^|[ ._\-])(CD|DISC|DISK|PART)[ ._\-]?(\d{1,2})(?:$|[ ._\-])/i);
  if (!match) return null;
  const kind = /^part$/i.test(match[1]) ? 'Part' : 'CD';
  return `${kind}${Number(match[2])}`;
}

function userRuleLines(value) {
  return String(value || '').split(/\r?\n/).map((line) => line.trim()).filter((line) => line && !line.startsWith('#')).slice(0, 2_000);
}

const USER_PATTERN_CACHE = new Map();

function userPatternSource(value) {
  let source = cleanText(value);
  let insensitive = false;
  if (source.startsWith('(?i)')) {
    source = source.slice(4);
    insensitive = true;
  }
  return { source, insensitive };
}

function compileUserPattern(value, global = true) {
  const { source, insensitive } = userPatternSource(value);
  const flags = `${global ? 'g' : ''}${insensitive ? 'i' : ''}u`;
  const cacheKey = `${flags}\u0000${source}`;
  const cached = USER_PATTERN_CACHE.get(cacheKey);
  if (cached) {
    cached.lastIndex = 0;
    return cached;
  }
  let pattern;
  try {
    pattern = new RegExp(source, flags);
  } catch {
    const escaped = source.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    pattern = new RegExp(escaped, `${global ? 'g' : ''}iu`);
  }
  if (USER_PATTERN_CACHE.size >= 4_096) USER_PATTERN_CACHE.clear();
  USER_PATTERN_CACHE.set(cacheKey, pattern);
  return pattern;
}

export function validateAuxiliaryRuleBlock(value, label, { replacement = true } = {}) {
  const lines = String(value || '').split(/\r?\n/);
  for (let index = 0; index < lines.length; index += 1) {
    const line = lines[index].trim();
    if (!line || line.startsWith('#')) continue;
    const arrow = replacement ? line.indexOf('=>') : -1;
    const patternText = arrow >= 0 ? line.slice(0, arrow).trim() : line;
    if (!patternText) throw new Error(`${label}第 ${index + 1} 行缺少正则表达式`);
    if (patternText.startsWith('@?{')) throw new Error(`${label}第 ${index + 1} 行使用了尚未支持的 @? 条件规则`);
    const { source, insensitive } = userPatternSource(patternText);
    if (/\\[1-9]|\\k<|\(\?(?:[=!]|<[=!]|P?<)/.test(source)) {
      throw new Error(`${label}第 ${index + 1} 行使用了统一规则语法不支持的前后查找、正则内反向引用或命名捕获`);
    }
    try {
      new RegExp(source, `${insensitive ? 'i' : ''}u`);
    } catch (error) {
      throw new Error(`${label}第 ${index + 1} 行正则无效：${error.message}`);
    }
  }
}

function calculateCapturedNumber(value, expression) {
  let result = Number(value);
  if (!Number.isFinite(result)) return value;
  const operations = String(expression || '').matchAll(/([+\-*/])\s*(\d+(?:\.\d+)?)/g);
  for (const [, operator, rawNumber] of operations) {
    const number = Number(rawNumber);
    if (operator === '+') result += number;
    else if (operator === '-') result -= number;
    else if (operator === '*') result *= number;
    else if (operator === '/' && number !== 0) result /= number;
  }
  return Number.isInteger(result) ? String(result) : String(Number(result.toFixed(4)));
}

function expandRuleReplacement(template, captures) {
  return String(template || '').replace(/\\(\d+)(?:@([+\-*/\d.\s]+))?/g, (_, rawIndex, expression) => {
    const captured = captures[Number(rawIndex)] ?? '';
    return expression ? calculateCapturedNumber(captured, expression) : String(captured);
  });
}

function extractRecognitionDirectives(value, directives) {
  return String(value || '').replace(/\{\[([^\]]+)\]\}/g, (_, body) => {
    for (const entry of String(body).split(';')) {
      const [rawKey, ...rest] = entry.split('=');
      const key = cleanText(rawKey).toLowerCase();
      const rawValue = cleanText(rest.join('='));
      if (!rawValue) continue;
      if (key === 'tmdbid' || key === 'tmdb_id') {
        const parsed = Number(rawValue);
        if (Number.isInteger(parsed) && parsed > 0) directives.tmdb_id = parsed;
      } else if (key === 'type' && ['movie', 'tv'].includes(rawValue.toLowerCase())) directives.media_type = rawValue.toLowerCase();
      else if (key === 's' || key === 'season') {
        const parsed = Number(rawValue);
        if (Number.isInteger(parsed) && parsed >= 0) directives.season = parsed;
      } else if (key === 'e' || key === 'episode') {
        const parsed = Number(rawValue);
        if (Number.isInteger(parsed) && parsed >= 0) directives.episode = parsed;
      }
    }
    return '';
  });
}

function applyRuleBlock(value, block, directives) {
  let current = String(value || '');
  for (const line of userRuleLines(block)) {
    const arrow = line.indexOf('=>');
    const patternText = arrow >= 0 ? line.slice(0, arrow).trim() : line;
    const replacementText = arrow >= 0 ? line.slice(arrow + 2).trim() : '';
    if (!patternText || patternText.startsWith('@?{')) continue;
    const pattern = compileUserPattern(patternText);
    current = current.replace(pattern, (...args) => {
      const expanded = expandRuleReplacement(replacementText, args);
      return extractRecognitionDirectives(expanded, directives);
    });
  }
  return current;
}

export function applyAuxiliaryRecognition(value, settings = {}) {
  const directives = {};
  let normalized = applyRuleBlock(value, settings.recognition_words, directives);
  normalized = applyRuleBlock(normalized, settings.render_words, directives);
  return { value: normalized.replace(/\s{2,}/g, ' ').trim(), directives };
}

function matchTechnical(value, pattern, normalize = (item) => item) {
  const match = pattern.exec(value);
  return match ? normalize(match[1] || match[0]) : '';
}

function knownReleaseGroup(value, settings = {}) {
  const source = String(value || '');
  const groups = userRuleLines(settings.release_groups).sort((left, right) => right.length - left.length);
  const matched = groups.find((group) => source.toLocaleLowerCase().includes(group.toLocaleLowerCase()));
  if (matched) return matched;
  for (const rule of userRuleLines(settings.capture_groups)) {
    const capture = compileUserPattern(rule, false).exec(source);
    if (capture) return cleanText(capture.slice(1).find(Boolean) || capture[0]);
  }
  const trailing = /-([A-Za-z0-9][A-Za-z0-9@._-]{1,48})$/.exec(source)?.[1];
  if (trailing && !RELEASE_WORDS.has(trailing.toLowerCase())) return trailing;
  const leading = /^\[([^\]]{2,48})\]/.exec(source)?.[1];
  return cleanText(leading);
}

function technicalMetadata(value, settings = {}) {
  const videoFormat = matchTechnical(value, /(?:^|[ ._\-])(2160p|1080p|720p|480p|4k|uhd)(?:$|[ ._\-])/i, (item) => /^(?:4k|uhd)$/i.test(item) ? '2160p' : item.toLowerCase().replace('p', 'p'));
  const resourceType = matchTechnical(value, /(?:^|[ ._\-])(REMUX|WEB[ ._\-]?DL|WEBRip|Blu[ ._\-]?Ray|BDRip|HDTV|DVDRip|UHDRip)(?:$|[ ._\-])/i, (item) => item.replace(/[ ._]+/g, '-').replace(/^bluray$/i, 'BluRay'));
  const videoCodec = matchTechnical(value, /(?:^|[ ._\-])(AV1|HEVC|H[ .]?265|x265|AVC|H[ .]?264|x264)(?:$|[ ._\-])/i, (item) => /(?:265|hevc)/i.test(item) ? 'HEVC' : /(?:264|avc)/i.test(item) ? 'AVC' : 'AV1');
  const audioCodec = matchTechnical(value, /(?:^|[ ._\-])(Atmos[ ._\-]*TrueHD|TrueHD|DTS[ ._\-]*HD(?:[ ._\-]*MA)?|DTS|DDP|EAC3|AC3|AAC|FLAC|LPCM|OPUS)(?:[ ._\-]?(?:7\.1|5\.1|2\.0|1\.0))?(?:$|[ ._\-])/i, (item) => item.replace(/[ ._]+/g, '-'));
  const audioInfo = matchTechnical(value, /(?:^|[ ._\-])(?:(?:Atmos[ ._\-]*TrueHD|TrueHD|DTS[ ._\-]*HD(?:[ ._\-]*MA)?|DTS|DDP|EAC3|AC3|AAC|FLAC|LPCM|OPUS)[ ._\-]*)?((?:Atmos[ ._\-]*)?(?:7\.1|5\.1|2\.0|1\.0))(?:$|[ ._\-])/i, (item) => item.replace(/[ _]+/g, ' ').trim());
  const dolbyVision = /(?:^|[ ._\-])(?:DV|DoVi|Dolby[ ._\-]*Vision)(?:$|[ ._\-])/i.test(value) ? 'DV' : '';
  const dynamicRange = matchTechnical(value, /(?:^|[ ._\-])(HDR10\+|HDR10|HDR|HLG|SDR)(?:$|[ ._\-])/i, (item) => item.toUpperCase());
  const frameRate = matchTechnical(value, /(?:^|[ ._\-])((?:23\.976|24|25|29\.97|30|50|59\.94|60|120)(?:fps|p))(?:$|[ ._\-])/i, (item) => item.toLowerCase());
  const colorDepth = matchTechnical(value, /(?:^|[ ._\-])(8bit|10bit|12bit)(?:$|[ ._\-])/i, (item) => item.toLowerCase());
  const source = matchTechnical(value, /(?:^|[ ._\-])(AMZN|Amazon|NF|Netflix|ATVP|AppleTV|DSNP|Disney\+|HMAX|HBO|Hulu|Bilibili|CR|TVING|Viu)(?:$|[ ._\-])/i);
  return {
    video_format: videoFormat,
    resource_type: resourceType,
    source,
    effect: [dolbyVision, dynamicRange].filter(Boolean).join(' '),
    audio_info: audioInfo,
    video_codec: videoCodec,
    audio_codec: audioCodec,
    release_group: knownReleaseGroup(value, settings),
    release_type: resourceType,
    high_quality: /(?:^|[ ._\-])HQ(?:$|[ ._\-])/i.test(value) ? 'HQ' : '',
    dolby_vision: dolbyVision,
    dynamic_range: dynamicRange,
    frame_rate: frameRate,
    color_depth: colorDepth,
  };
}

function tvNumbers(value, mediaTypeHint = '') {
  const patterns = [
    /(?:^|[^A-Za-z0-9])S(\d{1,3})[ ._\-]*E(\d{1,4})(?:[ ._\-]*(?:E|\-E?)(\d{1,4}))?(?:v\d+)?(?:$|[^A-Za-z0-9])/i,
    /(?:^|[^0-9])(\d{1,3})x(\d{1,4})(?:[ ._\-]*(?:x|\-)(\d{1,4}))?(?:$|[^0-9])/i,
    /第\s*(\d{1,3})\s*季[^0-9]{0,12}第?\s*(\d{1,4})(?:\s*[\-~至]\s*(\d{1,4}))?\s*[集話话]/i,
    /Season[ ._\-]*(\d{1,3})[^0-9]{0,12}(?:Episode|EP?)[ ._\-]*(\d{1,4})(?:[ ._\-]*(?:\-|EP?)(\d{1,4}))?/i,
  ];
  for (const pattern of patterns) {
    const match = pattern.exec(value);
    if (!match) continue;
    return {
      season: Number(match[1]),
      episode: Number(match[2]),
      episode_end: match[3] ? Number(match[3]) : null,
      marker_index: match.index,
      marker_length: match[0].length,
    };
  }
  const seasonOnly = /(?:^|[^A-Za-z0-9])S(?:eason)?[ ._\-]?(\d{1,3})(?:$|[^A-Za-z0-9])/i.exec(value)
    || /第\s*(\d{1,3})\s*季/i.exec(value);
  const episodeOnly = /(?:^|[^A-Za-z0-9])EP?[ ._\-]?(\d{1,4})(?:[ ._\-]*(?:\-|EP?)(\d{1,4}))?(?:v\d+)?(?:$|[^A-Za-z0-9])/i.exec(value)
    || /第?\s*(\d{1,4})(?:\s*[\-~至]\s*(\d{1,4}))?\s*[集話话]/i.exec(value);
  if (episodeOnly && (mediaTypeHint === 'tv' || seasonOnly)) {
    return {
      season: seasonOnly ? Number(seasonOnly[1]) : null,
      episode: Number(episodeOnly[1]),
      episode_end: episodeOnly[2] ? Number(episodeOnly[2]) : null,
      marker_index: Math.min(seasonOnly?.index ?? Number.MAX_SAFE_INTEGER, episodeOnly.index),
      marker_length: episodeOnly[0].length,
    };
  }
  if (mediaTypeHint === 'tv') {
    const anime = /(?:^|\s)[\-–—]\s*(\d{1,4})(?:v\d+)?(?:\s|$)/i.exec(value);
    if (anime) {
      return { season: seasonOnly ? Number(seasonOnly[1]) : 1, episode: Number(anime[1]), episode_end: null, marker_index: anime.index, marker_length: anime[0].length };
    }
  }
  if (seasonOnly) {
    return { season: Number(seasonOnly[1]), episode: null, episode_end: null, marker_index: seasonOnly.index, marker_length: seasonOnly[0].length };
  }
  return { season: null, episode: null, episode_end: null, marker_index: -1, marker_length: 0 };
}

function cleanTitle(value) {
  const withoutBrackets = stripTechnicalBrackets(value);
  const tokens = normalizeSpaces(withoutBrackets).split(' ');
  const retained = [];
  for (const token of tokens) {
    const normalized = token.toLowerCase().replace(/[^a-z0-9\u3400-\u9fff-]/g, '');
    if (!normalized) continue;
    if (/^(?:2160|1080|720|480)p$/i.test(normalized) || RELEASE_WORDS.has(normalized)) continue;
    if (/^(?:x|h)?26[45]$/i.test(normalized) || /^(?:aac|ddp|dts|flac)\d*(?:\.\d+)?$/i.test(normalized)) continue;
    retained.push(token);
  }
  return normalizeSpaces(retained.join(' '))
    .replace(/\s+(?:Season|Complete)\s*$/i, '')
    .trim();
}

export function parseMediaName(value, options = {}) {
  const rawOriginal = stemOf(value);
  const auxiliary = applyAuxiliaryRecognition(rawOriginal, options);
  const original = auxiliary.value || rawOriginal;
  const hint = cleanText(options.media_type).toLowerCase();
  const technical = stripTechnicalBrackets(original);
  const numbers = tvNumbers(technical, hint);
  const yearMatch = /(?:^|[^0-9])(19\d{2}|20\d{2}|21\d{2})(?:$|[^0-9pP])/.exec(technical);
  const year = options.year == null ? (yearMatch ? Number(yearMatch[1]) : null) : Number(options.year);
  const cutIndexes = [numbers.marker_index, yearMatch?.index ?? -1].filter((index) => index >= 0);
  let titleSource = cutIndexes.length ? technical.slice(0, Math.min(...cutIndexes)) : technical;
  titleSource = titleSource.replace(/(?:^|[ ._\-])(?:Season|S)[ ._\-]?\d{1,3}(?:$|[ ._\-])/i, ' ');
  let title = cleanTitle(titleSource);
  if (!title || /^(?:season|episode|ep|complete|disc|disk|part)\s*\d*$/i.test(title)) title = cleanTitle(options.fallback_title || '');
  const forcedSeason = options.season === '' || options.season == null ? null : Number(options.season);
  const forcedEpisode = options.episode === '' || options.episode == null ? null : Number(options.episode);
  const forcedEpisodeEnd = options.episode_end === '' || options.episode_end == null ? null : Number(options.episode_end);
  const season = forcedSeason ?? auxiliary.directives.season ?? numbers.season;
  const episode = forcedEpisode ?? auxiliary.directives.episode ?? numbers.episode;
  const episodeEnd = forcedEpisodeEnd ?? numbers.episode_end;
  const mediaType = hint || auxiliary.directives.media_type || ((episode != null || numbers.season != null) ? 'tv' : 'movie');
  return {
    original,
    title,
    year: Number.isInteger(year) ? year : null,
    media_type: mediaType,
    season: Number.isInteger(season) ? season : null,
    episode: Number.isInteger(episode) ? episode : null,
    episode_end: Number.isInteger(episodeEnd) ? episodeEnd : null,
    tmdb_id: Number.isInteger(auxiliary.directives.tmdb_id) ? auxiliary.directives.tmdb_id : null,
    edition: releaseEdition(original),
    quality: releaseQuality(original),
    part: releasePart(original),
    ...technicalMetadata(original, options),
  };
}

export function normalizeSearchTitle(value) {
  return cleanText(value)
    .normalize('NFKD')
    .toLowerCase()
    .replace(/&/g, ' and ')
    .replace(/[^a-z0-9\u3400-\u9fff]+/g, '');
}

function ngrams(value, size = 2) {
  const normalized = normalizeSearchTitle(value);
  if (!normalized) return new Set();
  if (normalized.length <= size) return new Set([normalized]);
  const result = new Set();
  for (let index = 0; index <= normalized.length - size; index += 1) result.add(normalized.slice(index, index + size));
  return result;
}

export function titleSimilarity(left, right) {
  const a = normalizeSearchTitle(left);
  const b = normalizeSearchTitle(right);
  if (!a || !b) return 0;
  if (a === b) return 1;
  if (a.includes(b) || b.includes(a)) return Math.min(a.length, b.length) / Math.max(a.length, b.length) * 0.9;
  const leftGrams = ngrams(a);
  const rightGrams = ngrams(b);
  let intersection = 0;
  for (const gram of leftGrams) if (rightGrams.has(gram)) intersection += 1;
  return (2 * intersection) / (leftGrams.size + rightGrams.size || 1);
}

export function scoreTmdbCandidate(query, candidate) {
  const titles = [candidate.title, candidate.original_title, candidate.name, candidate.original_name].filter(Boolean);
  const titleScore = Math.max(0, ...titles.map((title) => titleSimilarity(query.title, title)));
  const candidateYear = Number(String(candidate.release_date || candidate.first_air_date || '').slice(0, 4)) || null;
  let yearScore = 0.55;
  if (query.year && candidateYear) {
    const difference = Math.abs(Number(query.year) - candidateYear);
    yearScore = difference === 0 ? 1 : difference === 1 ? 0.72 : difference === 2 ? 0.35 : 0;
  }
  const popularityScore = Math.min(1, Math.log10(Math.max(1, Number(candidate.popularity || 0) + 1)) / 3);
  return Math.max(0, Math.min(1, titleScore * 0.79 + yearScore * 0.16 + popularityScore * 0.05));
}

function usefulEntry(name) {
  const normalized = cleanText(name).toLowerCase();
  return Boolean(normalized) && !normalized.startsWith('.') && !normalized.startsWith('~$') && !IGNORED_NAMES.has(normalized);
}

function isSamplePath(filePath) {
  const normalized = filePath.replaceAll('\\', '/').toLowerCase();
  return /(?:^|\/)(?:sample|samples)(?:\/|$)/.test(normalized) || /(?:^|[ ._\-])sample(?:[ ._\-]|$)/.test(stemOf(filePath).toLowerCase());
}

function extraKind(filePath) {
  const normalized = filePath.replaceAll('\\', '/').toLowerCase();
  if (/(?:^|[ ._\-])trailer(?:[ ._\-]|$)/.test(stemOf(filePath).toLowerCase()) || /\/trailers?\//.test(normalized)) return 'trailer';
  if (/\/(?:extras?|featurettes?|behind the scenes|deleted scenes|interviews?)\//.test(normalized)) return 'extra';
  return '';
}

async function walkCandidate(candidatePath) {
  const result = { videos: [], subtitles: [], audio: [], ignored_samples: [] };
  const rootStat = await fsp.lstat(candidatePath).catch((error) => error?.code === 'ENOENT' ? null : Promise.reject(error));
  if (!rootStat) return result;
  const consumeFile = (filePath) => {
    const extension = path.extname(filePath).toLowerCase();
    if (VIDEO_EXTENSIONS.has(extension)) {
      if (isSamplePath(filePath)) result.ignored_samples.push(filePath);
      else result.videos.push(filePath);
    } else if (SUBTITLE_EXTENSIONS.has(extension)) result.subtitles.push(filePath);
    else if (AUDIO_EXTENSIONS.has(extension)) result.audio.push(filePath);
  };
  if (rootStat.isFile()) {
    consumeFile(candidatePath);
    return result;
  }
  if (rootStat.isSymbolicLink() || !rootStat.isDirectory()) return result;
  const pending = [candidatePath];
  while (pending.length) {
    const directory = pending.pop();
    const entries = await fsp.readdir(directory, { withFileTypes: true });
    for (const entry of entries) {
      if (!usefulEntry(entry.name)) continue;
      const child = path.join(directory, entry.name);
      if (entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) pending.push(child);
      else if (entry.isFile()) consumeFile(child);
    }
  }
  result.videos.sort((left, right) => left.localeCompare(right, 'zh-CN', { numeric: true }));
  result.subtitles.sort((left, right) => left.localeCompare(right, 'zh-CN', { numeric: true }));
  result.audio.sort((left, right) => left.localeCompare(right, 'zh-CN', { numeric: true }));
  return result;
}

function mostUsefulTitle(parsedItems, fallback) {
  const counts = new Map();
  for (const item of parsedItems) {
    const title = cleanText(item.title);
    if (!title) continue;
    const key = normalizeSearchTitle(title);
    const current = counts.get(key) || { title, count: 0 };
    current.count += 1;
    if (title.length > current.title.length) current.title = title;
    counts.set(key, current);
  }
  return [...counts.values()].sort((left, right) => right.count - left.count || right.title.length - left.title.length)[0]?.title || fallback;
}

function parentSeason(filePath, candidatePath) {
  const relative = path.relative(candidatePath, path.dirname(filePath));
  const match = relative.match(/(?:^|[\\/])(?:Season|S)[ ._\-]?(\d{1,3})(?:$|[\\/])/i)
    || relative.match(/(?:^|[\\/])第\s*(\d{1,3})\s*季(?:$|[\\/])/i);
  return match ? Number(match[1]) : null;
}

function bestSidecarVideo(sidecar, videos, settings = DEFAULT_ORGANIZER_SETTINGS) {
  if (!videos.length) return null;
  const sidecarStem = normalizeSearchTitle(stemOf(sidecar).replace(/(?:chs|cht|chi|eng|jpn|kor|zh[-_.]?(?:cn|tw)|简体|繁体|繁體|字幕|forced|sdh|default)/gi, ''));
  const exact = videos.find((video) => sidecarStem === normalizeSearchTitle(stemOf(video.source)));
  if (exact) return exact;
  const parsed = parseMediaName(sidecar, { media_type: 'tv', ...auxiliaryParserOptions(settings) });
  if (parsed.episode != null) {
    const episodeMatch = videos.find((video) => video.parsed.episode === parsed.episode && (parsed.season == null || video.parsed.season === parsed.season));
    if (episodeMatch) return episodeMatch;
  }
  const sameDirectory = videos.filter((video) => path.dirname(video.source) === path.dirname(sidecar));
  if (sameDirectory.length === 1) return sameDirectory[0];
  return videos.length === 1 ? videos[0] : null;
}

function auxiliaryParserOptions(settings = {}) {
  return {
    recognition_words: settings.recognition_words || '',
    release_groups: settings.release_groups || '',
    render_words: settings.render_words || '',
    capture_groups: settings.capture_groups || '',
  };
}

export async function analyzeMediaCandidate(candidatePath, overrides = {}, settings = DEFAULT_ORGANIZER_SETTINGS) {
  const absoluteCandidate = path.resolve(candidatePath);
  const files = await walkCandidate(absoluteCandidate);
  const candidateStat = await fsp.lstat(absoluteCandidate).catch((error) => error?.code === 'ENOENT' ? null : Promise.reject(error));
  if (!candidateStat) throw new Error('待整理文件已经不存在');
  if (!files.videos.length) throw new Error('没有找到可整理的视频文件');
  const candidateName = candidateStat.isFile() ? stemOf(absoluteCandidate) : path.basename(absoluteCandidate);
  const explicitType = cleanText(overrides.media_type).toLowerCase();
  const recognition = auxiliaryParserOptions(settings);
  const group = parseMediaName(candidateName, { ...recognition, media_type: explicitType, year: overrides.year, season: overrides.season });
  const preliminary = files.videos.map((source) => {
    const parsed = parseMediaName(source, {
      ...recognition,
      media_type: explicitType || group.media_type,
      fallback_title: group.title,
      year: overrides.year,
      season: overrides.season ?? parentSeason(source, absoluteCandidate) ?? undefined,
      episode: files.videos.length === 1 ? overrides.episode : undefined,
      episode_end: files.videos.length === 1 ? overrides.episode_end : undefined,
    });
    return { source, parsed, extra_kind: extraKind(source) };
  });
  const inferredTv = preliminary.filter((item) => item.parsed.episode != null || item.parsed.season != null).length > preliminary.length / 2;
  const mediaType = explicitType || (inferredTv ? 'tv' : group.media_type || 'movie');
  const title = cleanText(overrides.title) || mostUsefulTitle(preliminary.map((item) => item.parsed), group.title);
  const year = overrides.year == null ? (group.year || preliminary.find((item) => item.parsed.year)?.parsed.year || null) : Number(overrides.year);
  const videos = preliminary.map((item) => ({
    ...item,
    parsed: parseMediaName(item.source, {
      ...recognition,
      media_type: mediaType,
      fallback_title: title,
      year,
      season: overrides.season ?? parentSeason(item.source, absoluteCandidate) ?? undefined,
      episode: files.videos.length === 1 ? overrides.episode : undefined,
      episode_end: files.videos.length === 1 ? overrides.episode_end : undefined,
    }),
  }));
  const sidecars = [...files.subtitles.map((source) => ({ source, kind: 'subtitle' })), ...files.audio.map((source) => ({ source, kind: 'audio' }))]
    .map((sidecar) => ({ ...sidecar, video_source: bestSidecarVideo(sidecar.source, videos, settings)?.source || null }));
  return {
    candidate_path: absoluteCandidate,
    candidate_type: candidateStat.isFile() ? 'file' : 'dir',
    media_type: mediaType,
    title,
    year: Number.isInteger(year) ? year : null,
    tmdb_id: group.tmdb_id || preliminary.find((item) => item.parsed.tmdb_id)?.parsed.tmdb_id || null,
    videos,
    sidecars,
    ignored_samples: files.ignored_samples,
    query: { title, year: Number.isInteger(year) ? year : null, media_type: mediaType, tmdb_id: group.tmdb_id || null },
  };
}

function cloudEntryValue(entry, ...keys) {
  for (const key of keys) {
    if (entry?.[key] !== undefined && entry?.[key] !== null) return entry[key];
  }
  return undefined;
}

export function normalizeOrganizerCloudEntry(entry, logicalPath = '') {
  const name = cleanText(cloudEntryValue(entry, 'fileName', 'name'));
  const resourceType = cloudEntryValue(entry, 'resType', 'type');
  const isDirectory = resourceType === 2 || resourceType === '2' || resourceType === 'folder' || entry?.isDirectory === true || entry?.is_directory === true;
  const rawModified = cloudEntryValue(entry, 'updatedAt', 'updateTime', 'modifiedAt', 'modifyTime', 'utime', 'createdAt', 'createTime', 'ctime', 'modified_ms');
  const parsedModified = Number(rawModified);
  const modifiedMs = Number.isFinite(parsedModified)
    ? parsedModified
    : (Date.parse(String(rawModified || '')) || 0);
  return {
    id: cleanText(cloudEntryValue(entry, 'fileId', 'id')),
    parent_id: cleanText(cloudEntryValue(entry, 'parentId', 'parent_id')),
    name,
    path: cleanText(logicalPath || entry?.path || name).replaceAll('\\', '/').replace(/^\/+/, ''),
    is_directory: isDirectory,
    size: Number(cloudEntryValue(entry, 'fileSize', 'size') || 0),
    modified_ms: String(Math.max(0, modifiedMs)),
    raw: entry?.raw || entry || {},
  };
}

function cloudParentSeason(filePath, candidatePath) {
  const relative = path.posix.relative(candidatePath || '', path.posix.dirname(filePath));
  const match = relative.match(/(?:^|\/)(?:Season|S)[ ._\-]?(\d{1,3})(?:$|\/)/i)
    || relative.match(/(?:^|\/)第\s*(\d{1,3})\s*季(?:$|\/)/i);
  return match ? Number(match[1]) : null;
}

export function cloudCandidateFingerprint(candidate, entries) {
  const normalized = [candidate, ...(Array.isArray(entries) ? entries : [])]
    .filter(Boolean)
    .map((entry) => normalizeOrganizerCloudEntry(entry, entry.path))
    .sort((left, right) => left.path.localeCompare(right.path, 'zh-CN', { numeric: true }));
  const files = normalized.filter((entry) => !entry.is_directory);
  const videos = files.filter((entry) => VIDEO_EXTENSIONS.has(path.posix.extname(entry.name).toLowerCase()));
  const signature = crypto.createHash('sha256')
    .update(JSON.stringify(normalized.map((entry) => [entry.id, entry.parent_id, entry.path, entry.size, entry.modified_ms, entry.is_directory])))
    .digest('hex');
  return {
    signature,
    size: files.reduce((total, entry) => total + entry.size, 0),
    modified_ms: String(Math.max(0, ...normalized.map((entry) => Number(entry.modified_ms) || 0))),
    file_count: files.length,
    video_count: videos.length,
    type: normalizeOrganizerCloudEntry(candidate, candidate?.path).is_directory ? 'dir' : 'file',
  };
}

export function analyzeCloudMediaCandidate({ candidate, entries = [] }, overrides = {}, settings = DEFAULT_ORGANIZER_SETTINGS) {
  const root = normalizeOrganizerCloudEntry(candidate, candidate?.path || candidate?.name);
  const normalized = (root.is_directory ? entries : [candidate])
    .map((entry) => normalizeOrganizerCloudEntry(entry, entry.path || entry.name))
    .filter((entry) => !entry.is_directory && usefulEntry(entry.name));
  const videosRaw = [];
  const subtitles = [];
  const audio = [];
  const ignoredSamples = [];
  for (const entry of normalized) {
    const extension = path.posix.extname(entry.name).toLowerCase();
    if (VIDEO_EXTENSIONS.has(extension)) {
      if (isSamplePath(entry.path)) ignoredSamples.push(entry.path);
      else videosRaw.push(entry);
    } else if (SUBTITLE_EXTENSIONS.has(extension)) subtitles.push(entry);
    else if (AUDIO_EXTENSIONS.has(extension)) audio.push(entry);
  }
  if (!videosRaw.length) throw Object.assign(new Error('没有找到可整理的视频文件'), { code: 'video_required' });
  videosRaw.sort((left, right) => left.path.localeCompare(right.path, 'zh-CN', { numeric: true }));
  const explicitType = cleanText(overrides.media_type).toLowerCase();
  const candidateName = root.is_directory ? root.name : stemOf(root.name);
  const recognition = auxiliaryParserOptions(settings);
  const group = parseMediaName(candidateName, { ...recognition, media_type: explicitType, year: overrides.year, season: overrides.season });
  const preliminary = videosRaw.map((entry) => ({
    entry,
    parsed: parseMediaName(entry.path, {
      ...recognition,
      media_type: explicitType || group.media_type,
      fallback_title: group.title,
      year: overrides.year,
      season: overrides.season ?? cloudParentSeason(entry.path, root.path) ?? undefined,
      episode: videosRaw.length === 1 ? overrides.episode : undefined,
      episode_end: videosRaw.length === 1 ? overrides.episode_end : undefined,
    }),
  }));
  const inferredTv = preliminary.filter((item) => item.parsed.episode != null || item.parsed.season != null).length > preliminary.length / 2;
  const mediaType = explicitType || (inferredTv ? 'tv' : group.media_type || 'movie');
  const title = cleanText(overrides.title) || mostUsefulTitle(preliminary.map((item) => item.parsed), group.title);
  const year = overrides.year == null ? (group.year || preliminary.find((item) => item.parsed.year)?.parsed.year || null) : Number(overrides.year);
  const videos = preliminary.map(({ entry }) => ({
    source: entry.path,
    source_id: entry.id,
    source_parent_id: entry.parent_id,
    source_name: entry.name,
    size: entry.size,
    modified_ms: entry.modified_ms,
    parsed: parseMediaName(entry.path, {
      ...recognition,
      media_type: mediaType,
      fallback_title: title,
      year,
      season: overrides.season ?? cloudParentSeason(entry.path, root.path) ?? undefined,
      episode: videosRaw.length === 1 ? overrides.episode : undefined,
      episode_end: videosRaw.length === 1 ? overrides.episode_end : undefined,
    }),
    extra_kind: extraKind(entry.path),
  }));
  const videoByPath = new Map(videos.map((video) => [video.source, video]));
  const sidecars = [...subtitles.map((entry) => ({ entry, kind: 'subtitle' })), ...audio.map((entry) => ({ entry, kind: 'audio' }))]
    .map(({ entry, kind }) => {
      const matched = bestSidecarVideo(entry.path, videos, settings);
      return {
        source: entry.path,
        source_id: entry.id,
        source_parent_id: entry.parent_id,
        source_name: entry.name,
        size: entry.size,
        modified_ms: entry.modified_ms,
        kind,
        video_source: matched?.source || null,
        video_source_id: matched ? videoByPath.get(matched.source)?.source_id || null : null,
      };
    });
  return {
    candidate_path: root.path,
    candidate_id: root.id,
    candidate_parent_id: root.parent_id,
    candidate_type: root.is_directory ? 'dir' : 'file',
    media_type: mediaType,
    title,
    year: Number.isInteger(year) ? year : null,
    tmdb_id: group.tmdb_id || preliminary.find((item) => item.parsed.tmdb_id)?.parsed.tmdb_id || null,
    videos,
    sidecars,
    ignored_samples: ignoredSamples,
    query: { title, year: Number.isInteger(year) ? year : null, media_type: mediaType, tmdb_id: group.tmdb_id || null },
  };
}

function normalizeTmdbCandidate(item, mediaType, query, imageUrl) {
  const type = item.media_type === 'tv' || item.media_type === 'movie' ? item.media_type : mediaType;
  const title = type === 'tv' ? item.name : item.title;
  const originalTitle = type === 'tv' ? item.original_name : item.original_title;
  const releaseDate = type === 'tv' ? item.first_air_date : item.release_date;
  const candidate = {
    tmdb_id: Number(item.id),
    media_type: type,
    title: cleanText(title),
    original_title: cleanText(originalTitle),
    year: Number(String(releaseDate || '').slice(0, 4)) || null,
    release_date: cleanText(releaseDate),
    overview: cleanText(item.overview),
    vote_average: Number(item.vote_average || 0),
    popularity: Number(item.popularity || 0),
    poster_path: cleanText(item.poster_path),
    poster_url: item.poster_path ? imageUrl(item.poster_path, 'w342') : '',
  };
  candidate.score = Number(scoreTmdbCandidate(query, { ...item, title, original_title: originalTitle }).toFixed(4));
  return candidate;
}

function normalizeTmdbDetails(item, mediaType, imageUrl) {
  const title = mediaType === 'tv' ? item.name : item.title;
  const originalTitle = mediaType === 'tv' ? item.original_name : item.original_title;
  const releaseDate = mediaType === 'tv' ? item.first_air_date : item.release_date;
  const directors = (item.credits?.crew || [])
    .filter((person) => person.job === 'Director' || person.job === 'Series Director')
    .map((person) => person.name)
    .filter(Boolean)
    .slice(0, 12);
  return {
    tmdb_id: Number(item.id),
    imdb_id: cleanText(item.imdb_id || item.external_ids?.imdb_id),
    media_type: mediaType,
    title: cleanText(title),
    original_title: cleanText(originalTitle),
    year: Number(String(releaseDate || '').slice(0, 4)) || null,
    release_date: cleanText(releaseDate),
    overview: cleanText(item.overview),
    tagline: cleanText(item.tagline),
    status: cleanText(item.status),
    runtime: Number(item.runtime || item.episode_run_time?.[0] || 0),
    vote_average: Number(item.vote_average || 0),
    vote_count: Number(item.vote_count || 0),
    genres: (item.genres || []).map((genre) => genre.name).filter(Boolean),
    genre_ids: (item.genres || []).map((genre) => Number(genre.id)).filter(Number.isInteger),
    studios: (item.production_companies || []).map((company) => company.name).filter(Boolean),
    countries: (item.production_countries || []).map((country) => country.iso_3166_1).filter(Boolean),
    original_language: cleanText(item.original_language).toLowerCase(),
    origin_countries: (item.origin_country || item.production_countries || [])
      .map((country) => cleanText(country?.iso_3166_1 || country).toUpperCase())
      .filter(Boolean),
    directors,
    actors: (item.credits?.cast || []).slice(0, 30).map((person) => ({ name: person.name, role: person.character || '', order: person.order ?? 0, thumb: person.profile_path ? imageUrl(person.profile_path, 'w185') : '' })),
    poster_path: cleanText(item.poster_path),
    backdrop_path: cleanText(item.backdrop_path),
    poster_url: item.poster_path ? imageUrl(item.poster_path) : '',
    backdrop_url: item.backdrop_path ? imageUrl(item.backdrop_path) : '',
    seasons: {},
  };
}

export function createTmdbClient({ apiKey, language = 'zh-CN', imageLanguage = 'zh,null,en', includeAdult = false, apiBase = 'https://api.themoviedb.org/3', imageBase = 'https://image.tmdb.org/t/p', fetchImpl = undiciFetch } = {}) {
  const key = cleanText(apiKey);
  const normalizedApiBase = cleanText(apiBase).replace(/\/+$/, '');
  const normalizedImageBase = cleanText(imageBase).replace(/\/+$/, '');
  function imageUrl(imagePath, size = 'original') {
    if (!imagePath) return '';
    if (/^https?:\/\//i.test(imagePath)) return imagePath;
    if (normalizedImageBase.includes('{size}')) return `${normalizedImageBase.replaceAll('{size}', size)}/${String(imagePath).replace(/^\/+/, '')}`;
    return `${normalizedImageBase}/${size}/${String(imagePath).replace(/^\/+/, '')}`;
  }
  async function request(endpoint, parameters = {}, timeoutMs = 20_000) {
    if (!key) throw Object.assign(new Error('请先配置 TMDB API Key 或 Read Access Token'), { code: 'tmdb_not_configured' });
    const url = new URL(`${normalizedApiBase}/${String(endpoint).replace(/^\/+/, '')}`);
    const bearer = key.startsWith('eyJ') || key.length > 80;
    if (!bearer) url.searchParams.set('api_key', key);
    url.searchParams.set('language', cleanText(parameters.language ?? language) || 'zh-CN');
    for (const [name, value] of Object.entries(parameters)) {
      if (name === 'language' || value === undefined || value === null || value === '') continue;
      url.searchParams.set(name, String(value));
    }
    let response;
    try {
      response = await fetchImpl(url, {
        headers: { accept: 'application/json', ...(bearer ? { authorization: `Bearer ${key}` } : {}) },
        signal: AbortSignal.timeout(timeoutMs),
      });
    } catch (error) {
      throw Object.assign(new Error(`TMDB 请求失败：${error.message}`), { code: 'tmdb_unavailable' });
    }
    const text = await response.text();
    let payload;
    try { payload = text ? JSON.parse(text) : {}; } catch { payload = { status_message: text }; }
    if (!response.ok || payload?.success === false) {
      throw Object.assign(new Error(cleanText(payload?.status_message) || `TMDB 请求失败（HTTP ${response.status}）`), { code: response.status === 401 ? 'tmdb_unauthorized' : 'tmdb_error' });
    }
    return payload;
  }
  async function test() {
    await request('configuration', { language: 'en-US' });
    return { success: true, message: 'TMDB 连接成功' };
  }
  async function search(query) {
    const mediaType = query.media_type === 'tv' ? 'tv' : 'movie';
    const parameters = { query: query.title, include_adult: includeAdult, page: 1 };
    if (query.year) parameters[mediaType === 'tv' ? 'first_air_date_year' : 'year'] = query.year;
    let payload = await request(`search/${mediaType}`, parameters);
    if (!(payload.results || []).length && query.year) payload = await request(`search/${mediaType}`, { query: query.title, include_adult: includeAdult, page: 1 });
    return (payload.results || [])
      .slice(0, 20)
      .map((item) => normalizeTmdbCandidate(item, mediaType, query, imageUrl))
      .sort((left, right) => right.score - left.score || right.popularity - left.popularity);
  }
  async function details(mediaType, tmdbId) {
    const type = mediaType === 'tv' ? 'tv' : 'movie';
    const payload = await request(`${type}/${Number(tmdbId)}`, {
      append_to_response: 'credits,external_ids,images',
      include_image_language: imageLanguage,
    });
    return normalizeTmdbDetails(payload, type, imageUrl);
  }
  async function season(tmdbId, seasonNumber) {
    const payload = await request(`tv/${Number(tmdbId)}/season/${Number(seasonNumber)}`, {
      append_to_response: 'images,external_ids',
      include_image_language: imageLanguage,
    });
    return {
      season_number: Number(payload.season_number ?? seasonNumber),
      name: cleanText(payload.name),
      overview: cleanText(payload.overview),
      air_date: cleanText(payload.air_date),
      poster_path: cleanText(payload.poster_path),
      poster_url: payload.poster_path ? imageUrl(payload.poster_path) : '',
      episodes: (payload.episodes || []).map((episode) => ({
        episode_number: Number(episode.episode_number),
        season_number: Number(episode.season_number ?? seasonNumber),
        name: cleanText(episode.name),
        overview: cleanText(episode.overview),
        air_date: cleanText(episode.air_date),
        runtime: Number(episode.runtime || 0),
        vote_average: Number(episode.vote_average || 0),
        still_path: cleanText(episode.still_path),
        still_url: episode.still_path ? imageUrl(episode.still_path) : '',
      })),
    };
  }
  return { request, test, search, details, season, imageUrl };
}

function segmentedSearchTitles(value) {
  const original = cleanText(value);
  const variants = [];
  const add = (candidate) => {
    const normalized = cleanText(candidate).replace(/^[\s:：·\-]+|[\s:：·\-]+$/g, '');
    if (normalized.length >= 2 && normalizeSearchTitle(normalized) !== normalizeSearchTitle(original)
      && !variants.some((item) => normalizeSearchTitle(item) === normalizeSearchTitle(normalized))) variants.push(normalized);
  };
  for (const part of original.split(/[\/|｜]/)) add(part);
  add(original.replace(/[（(【\[].*?[）)】\]]/g, ' '));
  const cjk = original.match(/[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]{2,}/gu)?.join(' ');
  const latin = original.match(/[A-Za-z][A-Za-z0-9'’:&+\-]*(?:\s+[A-Za-z0-9][A-Za-z0-9'’:&+\-]*)*/g)?.join(' ');
  add(cjk);
  add(latin);
  return variants.slice(0, 3);
}

function isExactTmdbCandidate(candidate, query) {
  const titles = [candidate?.title, candidate?.original_title].map(normalizeSearchTitle).filter(Boolean);
  return titles.includes(normalizeSearchTitle(query.title))
    && (!query.year || !candidate?.year || Number(query.year) === Number(candidate.year));
}

export async function resolveTmdbMatch({ analysis, client, settings = DEFAULT_ORGANIZER_SETTINGS, overrides = {} }) {
  const mediaType = cleanText(overrides.media_type || analysis.media_type) === 'tv' ? 'tv' : 'movie';
  const query = {
    title: cleanText(overrides.title) || analysis.title,
    year: overrides.year == null || overrides.year === '' ? analysis.year : Number(overrides.year),
    media_type: mediaType,
  };
  const recognizedTmdbId = analysis.tmdb_id ?? analysis.query?.tmdb_id;
  if (!query.title && overrides.tmdb_id == null && recognizedTmdbId == null) return { ready: false, error_code: 'title_required', message: '无法从文件名提取媒体名称，请输入名称或 TMDB ID', query, candidates: [] };
  let candidates = [];
  let selected = null;
  const forcedId = overrides.tmdb_id == null || overrides.tmdb_id === '' ? Number(recognizedTmdbId) || null : Number(overrides.tmdb_id);
  if (forcedId) {
    const details = await client.details(mediaType, forcedId);
    selected = { tmdb_id: details.tmdb_id, media_type: mediaType, title: details.title, original_title: details.original_title, year: details.year, release_date: details.release_date, overview: details.overview, vote_average: details.vote_average, popularity: 0, poster_path: details.poster_path, poster_url: details.poster_url, score: 1, forced: true };
    candidates = [selected];
  } else {
    candidates = await client.search(query);
    if (!candidates.length && settings.word_segment_search !== false) {
      const extra = [];
      for (const title of segmentedSearchTitles(query.title)) {
        extra.push(...await client.search({ ...query, title }));
        if (extra.length >= 20) break;
      }
      const unique = new Map(extra.map((candidate) => [`${candidate.media_type}:${candidate.tmdb_id}`, candidate]));
      candidates = [...unique.values()].sort((left, right) => right.score - left.score || right.popularity - left.popularity).slice(0, 20);
    }
    const minimumScore = Number(settings.minimum_match_score ?? DEFAULT_ORGANIZER_SETTINGS.minimum_match_score);
    const first = candidates[0];
    const second = candidates[1];
    const exact = first && isExactTmdbCandidate(first, query);
    if (settings.similarity_match === false) selected = candidates.find((candidate) => isExactTmdbCandidate(candidate, query)) || null;
    else if (first && first.score >= minimumScore && (exact || !second || first.score - second.score >= 0.06)) selected = first;
    if (!selected) {
      return {
        ready: false,
        error_code: candidates.length ? 'ambiguous_match' : 'tmdb_not_found',
        message: candidates.length ? '找到多个可能结果，请选择正确的 TMDB 条目' : 'TMDB 未找到匹配结果，请修改名称、年份或直接填写 TMDB ID',
        query,
        candidates,
      };
    }
  }
  const metadata = await client.details(mediaType, selected.tmdb_id);
  if (mediaType === 'tv') {
    const seasons = [...new Set(analysis.videos.map((video) => video.parsed.season).filter((value) => Number.isInteger(value)))];
    if (!seasons.length && overrides.season != null && overrides.season !== '') seasons.push(Number(overrides.season));
    for (const seasonNumber of seasons) {
      try {
        metadata.seasons[String(seasonNumber)] = await client.season(selected.tmdb_id, seasonNumber);
      } catch (error) {
        metadata.seasons[String(seasonNumber)] = { season_number: seasonNumber, name: `Season ${seasonNumber}`, overview: '', air_date: '', poster_path: '', poster_url: '', episodes: [], error: error.message };
      }
    }
  }
  return { ready: true, error_code: null, message: `已匹配 ${metadata.title}${metadata.year ? ` (${metadata.year})` : ''}`, query, candidates, selected, metadata };
}

export function sanitizePathComponent(value, fallback = 'Unknown') {
  let result = cleanText(value)
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, ' ')
    .replace(/\s+/g, ' ')
    .replace(/[. ]+$/g, '')
    .trim();
  if (!result) result = fallback;
  if (/^(?:CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])$/i.test(result)) result = `_${result}`;
  return result.slice(0, 180);
}

export function renderNamingTemplate(template, context) {
  const normalizedContext = Object.fromEntries(Object.entries(context || {}).map(([key, value]) => [key.toLocaleLowerCase(), value]));
  const rendered = cleanText(template).replace(/\{([a-z_]+)(?::(\d+))?\}/gi, (_, key, width) => {
    const value = context?.[key] ?? normalizedContext[String(key).toLocaleLowerCase()] ?? '';
    if (value === '' || value === null || value === undefined) return '';
    return width ? String(value).padStart(Number(width), '0') : String(value);
  });
  return sanitizePathComponent(rendered
    .replace(/\(\s*\)|\[\s*\]/g, '')
    .replace(/\s+-\s+-\s+/g, ' - ')
    .replace(/(?:\s+-\s*)+$/g, '')
    .replace(/\s+/g, ' '));
}

function shortHash(value) {
  return crypto.createHash('sha1').update(String(value)).digest('hex').slice(0, 8);
}

function languageSuffix(filePath) {
  const name = stemOf(filePath).toLowerCase();
  let language = '';
  if (/(?:zh[-_. ]?(?:cn|hans)|chs|sc|简体|簡體|简中)/i.test(name)) language = 'zh-CN';
  else if (/(?:zh[-_. ]?(?:tw|hant)|cht|tc|繁体|繁體|繁中)/i.test(name)) language = 'zh-TW';
  else if (/(?:^|[._ -])(?:eng|en)(?:[._ -]|$)|英文/i.test(name)) language = 'en';
  else if (/(?:^|[._ -])(?:jpn|ja|jp)(?:[._ -]|$)|日文|日语|日語/i.test(name)) language = 'ja';
  else if (/(?:^|[._ -])(?:kor|ko|kr)(?:[._ -]|$)|韩文|韓文|韩语|韓語/i.test(name)) language = 'ko';
  const forced = /(?:^|[._ -])forced(?:[._ -]|$)/i.test(name) ? '.forced' : '';
  const sdh = /(?:^|[._ -])(?:sdh|hi)(?:[._ -]|$)/i.test(name) ? '.sdh' : '';
  return `${language ? `.${language}` : ''}${forced}${sdh}`;
}

async function exists(filePath) {
  return fsp.access(filePath).then(() => true, () => false);
}

async function resolvePlannedTarget(target, source, conflictPolicy, claimed) {
  const normalized = path.resolve(target);
  const key = process.platform === 'win32' ? normalized.toLowerCase() : normalized;
  const alreadyClaimed = claimed.has(key);
  const targetExists = await exists(normalized);
  if (!alreadyClaimed && !targetExists) {
    claimed.add(key);
    return { target: normalized, action: 'create', exists: false };
  }
  if (!alreadyClaimed && conflictPolicy === 'skip') {
    claimed.add(key);
    return { target: normalized, action: 'skip', exists: true };
  }
  if (!alreadyClaimed && conflictPolicy === 'overwrite') {
    claimed.add(key);
    return { target: normalized, action: 'overwrite', exists: true };
  }
  const extension = path.extname(normalized);
  const stem = extension ? normalized.slice(0, -extension.length) : normalized;
  let index = 0;
  let candidate;
  do {
    const suffix = index === 0 ? shortHash(source || normalized) : `${shortHash(source || normalized)}-${index + 1}`;
    candidate = `${stem} [${suffix}]${extension}`;
    index += 1;
  } while (claimed.has(process.platform === 'win32' ? candidate.toLowerCase() : candidate) || await exists(candidate));
  claimed.add(process.platform === 'win32' ? candidate.toLowerCase() : candidate);
  return { target: candidate, action: 'create', exists: false, renamed_for_conflict: true };
}

function templateContext(metadata, parsed, episodeDetails = null) {
  const season = parsed.season ?? '';
  const episode = parsed.episode ?? '';
  const seasonTag = season === '' ? '' : `S${String(season).padStart(2, '0')}`;
  const episodeTag = episode === '' ? '' : `E${String(episode).padStart(2, '0')}`;
  const episodeEnd = parsed.episode_end != null && parsed.episode_end !== parsed.episode ? `-E${String(parsed.episode_end).padStart(2, '0')}` : '';
  const countryCodes = [...new Set([...(metadata.countries || []), ...(metadata.origin_countries || [])].map((value) => cleanText(value).toUpperCase()).filter(Boolean))];
  const countryNames = countryCodes.map((code) => COUNTRY_NAMES_ZH[code] || code);
  const originalFilename = parsed.original || '';
  const seasonYear = Number(String(episodeDetails?.air_date || '').slice(0, 4)) || metadata.year || '';
  const videoCodecFrameRateHighQuality = [parsed.video_codec, parsed.frame_rate, parsed.high_quality].filter(Boolean).join(' ');
  const sourcePlatform = [parsed.resource_type, parsed.source].filter(Boolean).join(' ');
  const effectVersion = [parsed.effect, parsed.edition].filter(Boolean).join(' ');
  const remux = /remux/i.test(parsed.resource_type || parsed.release_type || '') ? 'REMUX' : '';
  const mediaInfo = composeMediaInfo(metadata.media_type, parsed);
  return {
    title: metadata.title,
    original_title: metadata.original_title,
    en_title: metadata.original_title || metadata.title,
    year: metadata.year || '',
    tmdb_id: metadata.tmdb_id,
    tmdbid: metadata.tmdb_id,
    season,
    episode,
    episode_end: episodeEnd,
    season_episode: `${seasonTag}${episodeTag}${episodeEnd}`,
    episode_title: episodeDetails?.name || '',
    edition: parsed.edition ? ` - ${parsed.edition}` : '',
    quality: parsed.quality ? ` - ${parsed.quality}` : '',
    part: parsed.part ? ` - ${parsed.part}` : '',
    country: countryNames[0] || '未知地区',
    country_code: countryCodes[0] || '',
    release_country: countryNames.join('、') || '未知地区',
    original_filename: originalFilename,
    original_name: originalFilename,
    segment: parsed.part || '',
    season_year: seasonYear,
    season_tag: seasonTag,
    episode_tag: episodeTag,
    video_format: parsed.video_format || '',
    videoFormat: parsed.video_format || '',
    resource_type: parsed.resource_type || '',
    resourceType: parsed.resource_type || '',
    source: parsed.source || '',
    effect: parsed.effect || '',
    audio_info: parsed.audio_info || '',
    audioInfo: parsed.audio_info || '',
    video_codec: parsed.video_codec || '',
    videoCodec: parsed.video_codec || '',
    audio_codec: parsed.audio_codec || '',
    audioCodec: parsed.audio_codec || '',
    release_group: parsed.release_group || '',
    releaseGroup: parsed.release_group || '',
    release_type: parsed.release_type || '',
    high_quality: parsed.high_quality || '',
    dolby_vision: parsed.dolby_vision || '',
    dynamic_range: parsed.dynamic_range || '',
    frame_rate: parsed.frame_rate || '',
    color_depth: parsed.color_depth || '',
    source_platform: sourcePlatform,
    version: parsed.edition || '',
    effect_version: effectVersion,
    remux,
    version_number: parsed.part || '',
    video_codec_frame_rate_high_quality: videoCodecFrameRateHighQuality,
    media_info: mediaInfo,
    media_probed: parsed.media_probed === true,
  };
}

function composeMediaInfo(mediaType, parsed = {}) {
  const values = mediaType === 'tv'
    ? [parsed.video_format, parsed.source, parsed.release_type || parsed.resource_type, parsed.high_quality, parsed.dolby_vision, parsed.dynamic_range, parsed.frame_rate, parsed.color_depth, parsed.video_codec, parsed.audio_codec]
    : [parsed.video_format, parsed.resource_type || parsed.release_type, parsed.effect, parsed.audio_info, parsed.video_codec, parsed.audio_codec];
  const body = values.map(cleanText).filter(Boolean).filter((value, index, items) => index === 0 || value !== items[index - 1]).join('.');
  const group = cleanText(parsed.release_group);
  return `${body}${group ? `${body ? '-' : ''}${group}` : ''}`;
}

const TECHNICAL_TEMPLATE_TOKEN = /\{\{?\s*(?:media_info|video_?format|resource_?type|source|effect|audio_?info|video_?codec|audio_?codec|release_?group|release_?type|high_?quality|dolby_?vision|dynamic_?range|frame_?rate|color_?depth)\s*\}?\}/i;

function appendMediaInfoSuffix(relativePath, template, context, enabled) {
  if (!enabled || !context.media_probed || !context.media_info || TECHNICAL_TEMPLATE_TOKEN.test(template)) return relativePath;
  const extension = path.posix.extname(relativePath);
  return `${relativePath.slice(0, extension ? -extension.length : undefined)}.${context.media_info}${extension}`;
}

function episodeDetails(metadata, season, episode) {
  return metadata.seasons?.[String(season)]?.episodes?.find((item) => Number(item.episode_number) === Number(episode)) || null;
}

function ensureWithinTarget(root, candidate) {
  const absoluteRoot = path.resolve(root);
  const absoluteCandidate = path.resolve(candidate);
  const relative = path.relative(absoluteRoot, absoluteCandidate);
  if (relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) throw new Error('整理目标路径超出媒体库根目录');
  return absoluteCandidate;
}

function cleanRenderedSegment(value) {
  return sanitizePathComponent(String(value || '')
    .replace(/\(\s*\)|\[\s*\]/g, '')
    .replace(/\s+-\s+-\s+/g, ' - ')
    .replace(/(?:\s+-\s*)+$/g, '')
    .replace(/\.{2,}/g, '.')
    .replace(/\s+/g, ' '));
}

export function renderOrganizerPathTemplate(template, context) {
  const normalizedContext = Object.fromEntries(Object.entries(context || {}).map(([key, value]) => [key.toLocaleLowerCase(), value]));
  const conditional = cleanText(template).replace(/\{\{@if@\}\}([\s\S]*?)\{\{@endif@\}\}/gi, (_, body) => {
    const keys = [...body.matchAll(/\{\{\s*([a-z_][a-z0-9_]*)\s*\}\}/gi)].map((match) => match[1].toLocaleLowerCase());
    return keys.length && keys.every((key) => normalizedContext[key] !== '' && normalizedContext[key] !== null && normalizedContext[key] !== undefined) ? body : '';
  });
  const aliases = conditional
    .replace(/\{\{\s*([a-z_][a-z0-9_]*)\s*\}\}/gi, '{$1}')
    .replace(/\{catgroy\}/gi, '{category}')
    .replace(/\{tmdbid\}/gi, '{tmdb_id}')
    .replace(/\{Season\s+x\}/gi, '{season_tag}')
    .replace(/\{(?:Episode|Expose)\s+n\}/gi, '{episode_tag}');
  const rendered = aliases.replace(/\{([a-z_]+)(?::(\d+))?\}/gi, (_, key, width) => {
    const value = context?.[key] ?? normalizedContext[String(key).toLocaleLowerCase()] ?? '';
    if (value === '' || value === null || value === undefined) return '';
    return width ? String(value).padStart(Number(width), '0') : String(value);
  });
  const rawParts = rendered.replaceAll('\\', '/').split('/').map((part) => part.trim()).filter(Boolean);
  if (rawParts.some((part) => part === '.' || part === '..')) throw Object.assign(new Error('整理路径模板不能包含相对目录跳转'), { code: 'invalid_path_template' });
  const parts = rawParts.map(cleanRenderedSegment).filter(Boolean);
  if (parts.length < 2) throw Object.assign(new Error('整理路径模板必须至少包含一个目录和一个文件名'), { code: 'invalid_path_template' });
  return parts.join('/');
}

function cloudTargetKey(value) {
  return String(value || '').replaceAll('\\', '/').replace(/^\/+|\/+$/g, '').toLocaleLowerCase();
}

async function resolveCloudTarget(relativePath, sourceIdentity, conflictPolicy, claimed, targetExists) {
  const normalized = String(relativePath || '').replaceAll('\\', '/').replace(/^\/+|\/+$/g, '');
  const key = cloudTargetKey(normalized);
  const alreadyClaimed = claimed.has(key);
  const existing = alreadyClaimed ? null : await targetExists(normalized);
  if (!alreadyClaimed && !existing) {
    claimed.add(key);
    return { target_relative: normalized, action: 'create', exists: false, existing_id: null };
  }
  if (!alreadyClaimed && conflictPolicy === 'skip') {
    claimed.add(key);
    return { target_relative: normalized, action: 'skip', exists: true, existing_id: existing?.id || existing?.fileId || null };
  }
  if (!alreadyClaimed && conflictPolicy === 'overwrite') {
    claimed.add(key);
    return { target_relative: normalized, action: 'overwrite', exists: true, existing_id: existing?.id || existing?.fileId || null };
  }
  const extension = path.posix.extname(normalized);
  const stem = extension ? normalized.slice(0, -extension.length) : normalized;
  const fingerprint = shortHash(sourceIdentity || normalized);
  for (let index = 0; index < 10_000; index += 1) {
    const suffix = index === 0 ? fingerprint : `${fingerprint}-${index + 1}`;
    const candidate = `${stem} [${suffix}]${extension}`;
    const candidateKey = cloudTargetKey(candidate);
    if (claimed.has(candidateKey) || await targetExists(candidate)) continue;
    claimed.add(candidateKey);
    return { target_relative: candidate, action: 'create', exists: false, existing_id: null, renamed_for_conflict: true };
  }
  throw new Error('目标目录同名文件过多，无法生成安全名称');
}

function commonTargetDirectory(paths) {
  const directories = paths.map((value) => path.posix.dirname(value).split('/').filter(Boolean));
  if (!directories.length) return '';
  const common = [];
  for (let index = 0; index < Math.min(...directories.map((parts) => parts.length)); index += 1) {
    const value = directories[0][index];
    if (!directories.every((parts) => parts[index] === value)) break;
    common.push(value);
  }
  return common.join('/');
}

function isSeasonDirectoryName(value, seasons) {
  const normalized = cleanText(value).toLowerCase().replace(/[._-]+/g, ' ').replace(/\s+/g, ' ');
  return seasons.some((season) => {
    const number = String(Number(season));
    const padded = number.padStart(2, '0');
    return normalized === number
      || normalized === padded
      || normalized === `s${number}`
      || normalized === `s${padded}`
      || normalized === `season ${number}`
      || normalized === `season ${padded}`
      || normalized === `第${number}季`
      || normalized === `第 ${number} 季`;
  });
}

function mediaRootForCloudTargets(mediaType, mainVideos) {
  const common = commonTargetDirectory(mainVideos.map((item) => item.target_relative));
  if (mediaType !== 'tv' || !common) return common;
  const seasons = [...new Set(mainVideos.map((item) => item.season).filter((value) => value != null))];
  const parts = common.split('/').filter(Boolean);
  if (parts.length > 1 && isSeasonDirectoryName(parts.at(-1), seasons)) return parts.slice(0, -1).join('/');
  return common;
}

function seasonDirectoryForCloudVideo(item, mediaRoot) {
  const directory = path.posix.dirname(item.target_relative);
  return directory && directory !== '.' && directory !== mediaRoot ? directory : mediaRoot;
}

function cloudPreviewTarget(mapping, relativePath) {
  return [String(mapping.target_path || '').replace(/\/+$/g, ''), relativePath].filter(Boolean).join('/');
}

function enabledScrapeTypes(mapping) {
  if (mapping.scrape !== true) return new Set();
  const values = Array.isArray(mapping.scrape_types) ? mapping.scrape_types : DEFAULT_SCRAPE_TYPES;
  return new Set(values.map((value) => cleanText(value).toLowerCase()));
}

export async function buildCloudNativePreview({ analysis, match, mapping, settings = DEFAULT_ORGANIZER_SETTINGS, mappingSignature, sourceSignature, targetExists = async () => null }) {
  if (!match.ready || !match.metadata) {
    return {
      success: false,
      engine: NATIVE_ENGINE_VERSION,
      mapping_signature: mappingSignature,
      source_signature: sourceSignature,
      query: match.query || analysis.query,
      candidates: match.candidates || [],
      selected: null,
      metadata: null,
      error_code: match.error_code || 'tmdb_required',
      message: match.message || '需要人工选择媒体信息',
      data: { summary: { total: 0, success: 0, failed: 0, warnings: 0, skipped: 0 }, items: [] },
    };
  }
  const metadata = match.metadata;
  const conflictPolicy = mapping.conflict_policy || 'skip';
  const transferType = mapping.transfer_type === 'move' ? 'move' : 'copy';
  const template = metadata.media_type === 'tv'
    ? (settings.tv_path_template || DEFAULT_ORGANIZER_SETTINGS.tv_path_template)
    : (settings.movie_path_template || DEFAULT_ORGANIZER_SETTINGS.movie_path_template);
  const category = resolveMediaCategory(metadata, settings);
  const claimed = new Set();
  const items = [];
  const videoTargets = new Map();
  for (const video of analysis.videos) {
    if (metadata.media_type === 'tv' && !video.extra_kind && (video.parsed.season == null || video.parsed.episode == null)) {
      items.push({ success: false, kind: 'video', source: video.source, source_id: video.source_id, target: '', target_relative: '', operation: transferType, action: 'error', exists: false, error_code: 'episode_required', message: '未识别到季集号，请人工填写季号/集号或调整文件名' });
      continue;
    }
    const details = metadata.media_type === 'tv' ? episodeDetails(metadata, video.parsed.season, video.parsed.episode) : null;
    const extension = path.posix.extname(video.source_name || video.source).replace(/^\./, '').toLowerCase();
    const context = { ...templateContext(metadata, video.parsed, details), category, catgroy: category, ext: extension, fileExt: extension ? `.${extension}` : '' };
    let relative = renderOrganizerPathTemplate(template, context);
    relative = appendMediaInfoSuffix(relative, template, context, settings.include_media_info !== false);
    if (video.extra_kind) {
      const baseDirectory = path.posix.dirname(relative);
      const extraDirectory = video.extra_kind === 'trailer' ? 'trailers' : 'extras';
      relative = path.posix.join(baseDirectory, extraDirectory, cleanRenderedSegment(video.source_name || path.posix.basename(video.source)));
    }
    const planned = await resolveCloudTarget(relative, video.source_id || video.source, conflictPolicy, claimed, targetExists);
    const item = {
      success: true,
      kind: video.extra_kind || 'video',
      source: video.source,
      source_id: video.source_id,
      source_parent_id: video.source_parent_id,
      source_name: video.source_name,
      target: cloudPreviewTarget(mapping, planned.target_relative),
      target_parent_relative: path.posix.dirname(planned.target_relative) === '.' ? '' : path.posix.dirname(planned.target_relative),
      target_name: path.posix.basename(planned.target_relative),
      operation: transferType,
      season: video.parsed.season,
      episode: video.parsed.episode,
      episode_end: video.parsed.episode_end,
      ...planned,
      message: planned.action === 'skip' ? '目标已存在，将跳过' : planned.renamed_for_conflict ? '目标冲突，已追加短标识' : '可执行',
    };
    items.push(item);
    videoTargets.set(video.source_id || video.source, item.target_relative);
  }
  if (mapping.sync_extras !== false) {
    for (const sidecar of analysis.sidecars) {
      const videoTarget = videoTargets.get(sidecar.video_source_id || sidecar.video_source);
      if (!videoTarget) continue;
      const extension = path.posix.extname(sidecar.source_name || sidecar.source).toLowerCase();
      const targetRelative = `${videoTarget.slice(0, -path.posix.extname(videoTarget).length)}${languageSuffix(sidecar.source)}${extension}`;
      const planned = await resolveCloudTarget(targetRelative, sidecar.source_id || sidecar.source, conflictPolicy, claimed, targetExists);
      items.push({
        success: true,
        kind: sidecar.kind,
        source: sidecar.source,
        source_id: sidecar.source_id,
        source_parent_id: sidecar.source_parent_id,
        source_name: sidecar.source_name,
        target: cloudPreviewTarget(mapping, planned.target_relative),
        target_parent_relative: path.posix.dirname(planned.target_relative) === '.' ? '' : path.posix.dirname(planned.target_relative),
        target_name: path.posix.basename(planned.target_relative),
        operation: transferType,
        ...planned,
        message: planned.action === 'skip' ? '目标已存在，将跳过' : '跟随主视频整理',
      });
    }
  }
  const mainVideos = items.filter((item) => item.success && item.kind === 'video');
  const mediaRoot = mediaRootForCloudTargets(metadata.media_type, mainVideos);
  const scrapeTypes = enabledScrapeTypes(mapping);
  const addGenerated = async ({ relative, kind, operation, source = null, generator = null, imageRole = null, season = null, episode = null, message }) => {
    const planned = await resolveCloudTarget(relative, source || `${kind}:${metadata.tmdb_id}:${season ?? ''}:${episode ?? ''}`, conflictPolicy, claimed, targetExists);
    items.push({ success: true, kind, source, source_id: null, target: cloudPreviewTarget(mapping, planned.target_relative), target_parent_relative: path.posix.dirname(planned.target_relative) === '.' ? '' : path.posix.dirname(planned.target_relative), target_name: path.posix.basename(planned.target_relative), operation, generator, image_role: imageRole, season, episode, ...planned, message });
  };
  if (scrapeTypes.size && mediaRoot) {
    if (metadata.media_type === 'movie' && scrapeTypes.has('movie_nfo')) {
      for (const video of mainVideos) await addGenerated({ relative: `${video.target_relative.slice(0, -path.posix.extname(video.target_relative).length)}.nfo`, kind: 'nfo', operation: 'generate', generator: { type: 'movie' }, message: '生成电影 NFO' });
    }
    if (metadata.media_type === 'tv' && scrapeTypes.has('tvshow_nfo')) await addGenerated({ relative: path.posix.join(mediaRoot, 'tvshow.nfo'), kind: 'nfo', operation: 'generate', generator: { type: 'tvshow' }, message: '生成剧集 NFO' });
    if (metadata.media_type === 'tv' && scrapeTypes.has('episode_nfo')) {
      for (const video of mainVideos) await addGenerated({ relative: `${video.target_relative.slice(0, -path.posix.extname(video.target_relative).length)}.nfo`, kind: 'nfo', operation: 'generate', generator: { type: 'episode', season: video.season, episode: video.episode }, season: video.season, episode: video.episode, message: '生成单集 NFO' });
    }
    if (metadata.poster_url && scrapeTypes.has('poster')) await addGenerated({ relative: path.posix.join(mediaRoot, 'poster.jpg'), kind: 'image', operation: 'download', source: metadata.poster_url, imageRole: 'poster', message: '下载海报' });
    if (metadata.backdrop_url && scrapeTypes.has('fanart')) await addGenerated({ relative: path.posix.join(mediaRoot, 'fanart.jpg'), kind: 'image', operation: 'download', source: metadata.backdrop_url, imageRole: 'fanart', message: '下载背景图' });
    if (metadata.media_type === 'tv' && scrapeTypes.has('season_poster')) {
      for (const season of Object.values(metadata.seasons || {})) {
        if (!season.poster_url) continue;
        const seasonVideo = mainVideos.find((item) => Number(item.season) === Number(season.season_number));
        const seasonRoot = seasonVideo ? seasonDirectoryForCloudVideo(seasonVideo, mediaRoot) : mediaRoot;
        await addGenerated({ relative: path.posix.join(seasonRoot, 'poster.jpg'), kind: 'image', operation: 'download', source: season.poster_url, imageRole: 'season-poster', season: season.season_number, message: `下载第 ${season.season_number} 季海报` });
      }
    }
  }
  const failed = items.filter((item) => !item.success).length;
  const skipped = items.filter((item) => item.action === 'skip').length;
  const probeWarnings = Array.isArray(analysis.media_probe_warnings) ? analysis.media_probe_warnings : [];
  const warnings = skipped + analysis.ignored_samples.length + probeWarnings.length + Object.values(metadata.seasons || {}).filter((season) => season.error).length;
  const message = failed ? `有 ${failed} 项无法生成目标，请人工修正` : `已生成 ${items.length} 项云端整理预览${warnings ? `，${warnings} 项提示` : ''}`;
  return {
    success: failed === 0 && mainVideos.length > 0,
    engine: NATIVE_ENGINE_VERSION,
    mapping_signature: mappingSignature,
    source_signature: sourceSignature,
    query: match.query,
    candidates: match.candidates,
    selected: match.selected,
    metadata,
    media_probe_warnings: probeWarnings,
    target_root: mapping.target_path,
    target_root_id: mapping.target_dir_id,
    media_root: cloudPreviewTarget(mapping, mediaRoot),
    media_root_relative: mediaRoot,
    share_relative_path: mediaRoot,
    share_title: `${metadata.title}${metadata.year ? ` (${metadata.year})` : ''}`,
    error_code: failed ? items.find((item) => !item.success)?.error_code || 'preview_failed' : null,
    message,
    ignored_samples: analysis.ignored_samples,
    data: { summary: { total: items.length, success: items.length - failed, failed, warnings, skipped }, items },
  };
}

export async function buildNativePreview({ analysis, match, mapping, settings = DEFAULT_ORGANIZER_SETTINGS, mappingSignature, sourceSignature }) {
  if (!match.ready || !match.metadata) {
    return {
      success: false,
      engine: NATIVE_ENGINE_VERSION,
      mapping_signature: mappingSignature,
      source_signature: sourceSignature,
      query: match.query || analysis.query,
      candidates: match.candidates || [],
      selected: null,
      metadata: null,
      error_code: match.error_code || 'tmdb_required',
      message: match.message || '需要人工选择媒体信息',
      data: { summary: { total: 0, success: 0, failed: 0, warnings: 0 }, items: [] },
    };
  }
  const targetRoot = path.resolve(mapping.target_path);
  const conflictPolicy = mapping.conflict_policy || 'skip';
  const metadata = match.metadata;
  const claimed = new Set();
  const items = [];
  const videoTargets = new Map();
  const baseContext = templateContext(metadata, {});
  const mediaFolder = renderNamingTemplate(metadata.media_type === 'tv' ? settings.tv_folder_format : settings.movie_folder_format, baseContext);
  const mediaRoot = ensureWithinTarget(targetRoot, path.join(targetRoot, mediaFolder));
  for (const video of analysis.videos) {
    const extension = path.extname(video.source).toLowerCase();
    if (video.extra_kind) {
      const extraFolder = video.extra_kind === 'trailer' ? 'trailers' : 'extras';
      const extraName = sanitizePathComponent(stemOf(video.source));
      const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, path.join(mediaRoot, extraFolder, `${extraName}${extension}`)), video.source, conflictPolicy, claimed);
      const item = { success: true, kind: video.extra_kind, source: video.source, operation: mapping.transfer_type, ...planned, message: planned.action === 'skip' ? '目标已存在，将跳过' : '附加视频' };
      items.push(item);
      videoTargets.set(video.source, item.target);
      continue;
    }
    if (metadata.media_type === 'tv' && (video.parsed.season == null || video.parsed.episode == null)) {
      items.push({ success: false, kind: 'video', source: video.source, target: '', operation: mapping.transfer_type, action: 'error', exists: false, error_code: 'episode_required', message: '未识别到季集号，请人工填写季号/集号或调整文件名' });
      continue;
    }
    const details = metadata.media_type === 'tv' ? episodeDetails(metadata, video.parsed.season, video.parsed.episode) : null;
    const context = templateContext(metadata, video.parsed, details);
    let directory = mediaRoot;
    let filename;
    if (metadata.media_type === 'tv') {
      const seasonFolder = renderNamingTemplate(settings.season_folder_format, context);
      directory = ensureWithinTarget(targetRoot, path.join(mediaRoot, seasonFolder));
      filename = renderNamingTemplate(settings.episode_file_format, context);
    } else {
      filename = renderNamingTemplate(settings.movie_file_format, context);
    }
    const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, path.join(directory, `${filename}${extension}`)), video.source, conflictPolicy, claimed);
    const item = {
      success: true,
      kind: 'video',
      source: video.source,
      operation: mapping.transfer_type,
      season: video.parsed.season,
      episode: video.parsed.episode,
      episode_end: video.parsed.episode_end,
      ...planned,
      message: planned.action === 'skip' ? '目标已存在，将跳过' : planned.renamed_for_conflict ? '目标冲突，已追加短标识' : '可执行',
    };
    items.push(item);
    videoTargets.set(video.source, item.target);
  }
  if (mapping.sync_extras !== false) {
    for (const sidecar of analysis.sidecars) {
      const videoTarget = sidecar.video_source ? videoTargets.get(sidecar.video_source) : null;
      if (!videoTarget) continue;
      const extension = path.extname(sidecar.source).toLowerCase();
      const suffix = sidecar.kind === 'subtitle' ? languageSuffix(sidecar.source) : languageSuffix(sidecar.source);
      const target = `${videoTarget.slice(0, -path.extname(videoTarget).length)}${suffix}${extension}`;
      const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, target), sidecar.source, conflictPolicy, claimed);
      items.push({ success: true, kind: sidecar.kind, source: sidecar.source, operation: mapping.transfer_type, ...planned, message: planned.action === 'skip' ? '目标已存在，将跳过' : '跟随主视频整理' });
    }
  }
  if (mapping.scrape) {
    const mainVideos = items.filter((item) => item.success && item.kind === 'video');
    if (metadata.media_type === 'movie') {
      for (const video of mainVideos) {
        const target = `${video.target.slice(0, -path.extname(video.target).length)}.nfo`;
        const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, target), `movie-nfo:${metadata.tmdb_id}`, conflictPolicy, claimed);
        items.push({ success: true, kind: 'nfo', source: null, operation: 'generate', generator: { type: 'movie' }, ...planned, message: '生成电影 NFO' });
      }
    } else {
      const tvNfo = await resolvePlannedTarget(ensureWithinTarget(targetRoot, path.join(mediaRoot, 'tvshow.nfo')), `tv-nfo:${metadata.tmdb_id}`, conflictPolicy, claimed);
      items.push({ success: true, kind: 'nfo', source: null, operation: 'generate', generator: { type: 'tvshow' }, ...tvNfo, message: '生成剧集 NFO' });
      const seasonNumbers = [...new Set(mainVideos.map((item) => item.season).filter((value) => value != null))];
      for (const season of seasonNumbers) {
        const seasonFolder = renderNamingTemplate(settings.season_folder_format, templateContext(metadata, { season }));
        const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, path.join(mediaRoot, seasonFolder, 'season.nfo')), `season-nfo:${metadata.tmdb_id}:${season}`, conflictPolicy, claimed);
        items.push({ success: true, kind: 'nfo', source: null, operation: 'generate', generator: { type: 'season', season }, ...planned, message: `生成第 ${season} 季 NFO` });
      }
      for (const video of mainVideos) {
        const target = `${video.target.slice(0, -path.extname(video.target).length)}.nfo`;
        const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, target), `episode-nfo:${metadata.tmdb_id}:${video.season}:${video.episode}`, conflictPolicy, claimed);
        items.push({ success: true, kind: 'nfo', source: null, operation: 'generate', generator: { type: 'episode', season: video.season, episode: video.episode }, ...planned, message: '生成单集 NFO' });
      }
    }
    if (metadata.poster_url) {
      const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, path.join(mediaRoot, 'poster.jpg')), metadata.poster_url, conflictPolicy, claimed);
      items.push({ success: true, kind: 'image', source: metadata.poster_url, operation: 'download', image_role: 'poster', ...planned, message: '下载海报' });
    }
    if (metadata.backdrop_url) {
      const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, path.join(mediaRoot, 'fanart.jpg')), metadata.backdrop_url, conflictPolicy, claimed);
      items.push({ success: true, kind: 'image', source: metadata.backdrop_url, operation: 'download', image_role: 'fanart', ...planned, message: '下载背景图' });
    }
    if (metadata.media_type === 'tv') {
      for (const season of Object.values(metadata.seasons || {})) {
        if (!season.poster_url) continue;
        const filename = `season${String(season.season_number).padStart(2, '0')}-poster.jpg`;
        const planned = await resolvePlannedTarget(ensureWithinTarget(targetRoot, path.join(mediaRoot, filename)), season.poster_url, conflictPolicy, claimed);
        items.push({ success: true, kind: 'image', source: season.poster_url, operation: 'download', image_role: 'season-poster', season: season.season_number, ...planned, message: `下载第 ${season.season_number} 季海报` });
      }
    }
  }
  const failed = items.filter((item) => !item.success).length;
  const skipped = items.filter((item) => item.action === 'skip').length;
  const warnings = skipped + analysis.ignored_samples.length + Object.values(metadata.seasons || {}).filter((season) => season.error).length;
  const message = failed ? `有 ${failed} 项无法生成目标，请人工修正` : `已生成 ${items.length} 项原生整理预览${warnings ? `，${warnings} 项提示` : ''}`;
  return {
    success: failed === 0 && items.some((item) => item.kind === 'video' && item.success),
    engine: NATIVE_ENGINE_VERSION,
    mapping_signature: mappingSignature,
    source_signature: sourceSignature,
    query: match.query,
    candidates: match.candidates,
    selected: match.selected,
    metadata,
    target_root: targetRoot,
    media_root: mediaRoot,
    error_code: failed ? items.find((item) => !item.success)?.error_code || 'preview_failed' : null,
    message,
    ignored_samples: analysis.ignored_samples,
    data: {
      summary: { total: items.length, success: items.length - failed, failed, warnings, skipped },
      items,
    },
  };
}

export function classifyNativePreview(preview) {
  if (!preview || preview.engine !== NATIVE_ENGINE_VERSION || preview.success !== true) {
    return { ready: false, error_code: preview?.error_code || 'preview_required', message: cleanText(preview?.message) || '当前任务没有可执行的原生整理预览' };
  }
  if (!preview.metadata) return { ready: false, error_code: 'metadata_required', message: '原生整理预览缺少媒体元数据，请重新识别' };
  const items = Array.isArray(preview.data?.items) ? preview.data.items : [];
  const videoItems = items.filter((item) => item.kind === 'video');
  const failed = items.filter((item) => item.success !== true);
  if (!videoItems.length || failed.length) return { ready: false, error_code: failed[0]?.error_code || 'preview_failed', message: failed[0]?.message || '预览中存在不可执行项' };
  return { ready: true, error_code: null, message: cleanText(preview.message) || `已生成 ${items.length} 项整理目标` };
}

function xmlEscape(value) {
  return String(value ?? '').replace(/[<>&"']/g, (character) => ({ '<': '&lt;', '>': '&gt;', '&': '&amp;', '"': '&quot;', "'": '&apos;' })[character]);
}

function xmlNode(name, value, indent = '  ') {
  if (value === null || value === undefined || value === '') return '';
  return `${indent}<${name}>${xmlEscape(value)}</${name}>\n`;
}

function commonNfo(metadata, indent = '  ') {
  let body = '';
  body += xmlNode('title', metadata.title, indent);
  body += xmlNode('originaltitle', metadata.original_title, indent);
  body += xmlNode('year', metadata.year, indent);
  body += xmlNode('premiered', metadata.release_date, indent);
  body += xmlNode('plot', metadata.overview, indent);
  body += xmlNode('outline', metadata.overview, indent);
  body += xmlNode('tagline', metadata.tagline, indent);
  body += xmlNode('runtime', metadata.runtime, indent);
  body += xmlNode('rating', metadata.vote_average, indent);
  body += xmlNode('votes', metadata.vote_count, indent);
  body += `${indent}<uniqueid type="tmdb" default="true">${xmlEscape(metadata.tmdb_id)}</uniqueid>\n`;
  if (metadata.imdb_id) body += `${indent}<uniqueid type="imdb">${xmlEscape(metadata.imdb_id)}</uniqueid>\n`;
  for (const genre of metadata.genres || []) body += xmlNode('genre', genre, indent);
  for (const studio of metadata.studios || []) body += xmlNode('studio', studio, indent);
  for (const director of metadata.directors || []) body += xmlNode('director', director, indent);
  for (const actor of metadata.actors || []) {
    body += `${indent}<actor>\n${xmlNode('name', actor.name, `${indent}  `)}${xmlNode('role', actor.role, `${indent}  `)}${xmlNode('thumb', actor.thumb, `${indent}  `)}${indent}</actor>\n`;
  }
  return body;
}

export function renderNfo(generator, metadata) {
  const declaration = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n';
  if (generator.type === 'movie') return `${declaration}<movie>\n${commonNfo(metadata)}</movie>\n`;
  if (generator.type === 'tvshow') return `${declaration}<tvshow>\n${commonNfo(metadata)}</tvshow>\n`;
  if (generator.type === 'season') {
    const season = metadata.seasons?.[String(generator.season)] || {};
    let body = xmlNode('title', season.name || `Season ${generator.season}`);
    body += xmlNode('seasonnumber', generator.season);
    body += xmlNode('plot', season.overview);
    body += xmlNode('premiered', season.air_date);
    return `${declaration}<season>\n${body}</season>\n`;
  }
  const episode = episodeDetails(metadata, generator.season, generator.episode) || {};
  let body = xmlNode('title', episode.name || `Episode ${generator.episode}`);
  body += xmlNode('showtitle', metadata.title);
  body += xmlNode('season', generator.season);
  body += xmlNode('episode', generator.episode);
  body += xmlNode('aired', episode.air_date);
  body += xmlNode('plot', episode.overview);
  body += xmlNode('runtime', episode.runtime);
  body += xmlNode('rating', episode.vote_average);
  body += `<uniqueid type="tmdb" default="true">${xmlEscape(metadata.tmdb_id)}</uniqueid>\n`;
  return `${declaration}<episodedetails>\n${body}</episodedetails>\n`;
}

async function backupExistingFile(target) {
  const stat = await fsp.lstat(target).catch((error) => error?.code === 'ENOENT' ? null : Promise.reject(error));
  if (!stat) return null;
  if (stat.isDirectory()) throw new Error('目标路径已被目录占用，不能覆盖');
  const backup = `${target}.guangya-backup-${crypto.randomUUID()}`;
  await fsp.rename(target, backup);
  return backup;
}

async function commitTemporaryFile(temporary, target, { overwrite = false } = {}) {
  if (overwrite) {
    await fsp.rename(temporary, target);
    return;
  }
  await fsp.link(temporary, target);
  try {
    await fsp.rm(temporary);
  } catch (error) {
    await removeIfExists(target);
    throw error;
  }
}

async function atomicWrite(target, content, { overwrite = false } = {}) {
  await fsp.mkdir(path.dirname(target), { recursive: true });
  const temporary = path.join(path.dirname(target), `.${path.basename(target)}.guangya-${crypto.randomUUID()}.part`);
  let backup = null;
  try {
    await fsp.writeFile(temporary, content);
    if (overwrite) backup = await backupExistingFile(target);
    else if (await exists(target)) throw new Error('目标在执行期间已存在，请重新生成预览');
    await commitTemporaryFile(temporary, target, { overwrite });
    if (backup) await removeIfExists(backup);
  } catch (error) {
    await fsp.rm(temporary, { force: true }).catch(() => {});
    if (backup) {
      await removeIfExists(target);
      await fsp.rename(backup, target).catch(() => {});
    }
    throw error;
  }
}

async function downloadImage(url, target, fetchImpl, options = {}) {
  const response = await fetchImpl(url, { headers: { accept: 'image/*' }, signal: AbortSignal.timeout(30_000) });
  if (!response.ok) throw new Error(`图片下载失败（HTTP ${response.status}）`);
  const contentLength = Number(response.headers.get('content-length') || 0);
  if (contentLength > 25 * 1024 * 1024) throw new Error('图片超过 25 MB 安全限制');
  const bytes = Buffer.from(await response.arrayBuffer());
  if (bytes.length > 25 * 1024 * 1024) throw new Error('图片超过 25 MB 安全限制');
  if (!bytes.length) throw new Error('图片响应为空');
  await atomicWrite(target, bytes, options);
}

async function removeIfExists(filePath) {
  await fsp.rm(filePath, { force: true }).catch(() => {});
}

async function transferFile(item, transaction) {
  const target = item.target;
  if (item.action === 'skip') {
    transaction.skipped.push(target);
    return;
  }
  await fsp.mkdir(path.dirname(target), { recursive: true });
  if (item.action === 'overwrite' && await exists(target)) {
    const backup = await backupExistingFile(target);
    transaction.backups.push({ target, backup });
  }
  const temporary = path.join(path.dirname(target), `.${path.basename(target)}.guangya-${crypto.randomUUID()}.part`);
  try {
    if (item.operation === 'move') {
      let stagedMove = false;
      try {
        await fsp.rename(item.source, temporary);
        stagedMove = true;
        await commitTemporaryFile(temporary, target, { overwrite: item.action === 'overwrite' });
        transaction.moved.push({ source: item.source, target });
      } catch (error) {
        if (stagedMove) {
          await fsp.rename(temporary, item.source).catch(() => {});
          throw error;
        }
        if (error?.code !== 'EXDEV') throw error;
        await removeIfExists(temporary);
        await fsp.copyFile(item.source, temporary, fs.constants.COPYFILE_EXCL);
        await commitTemporaryFile(temporary, target, { overwrite: item.action === 'overwrite' });
        transaction.created.push(target);
        transaction.delete_after_commit.push(item.source);
      }
    } else if (item.operation === 'link') {
      await fsp.link(item.source, temporary);
      await commitTemporaryFile(temporary, target, { overwrite: item.action === 'overwrite' });
      transaction.created.push(target);
    } else if (item.operation === 'softlink') {
      await fsp.symlink(path.resolve(item.source), target, 'file');
      transaction.created.push(target);
    } else {
      await fsp.copyFile(item.source, temporary, fs.constants.COPYFILE_EXCL);
      await commitTemporaryFile(temporary, target, { overwrite: item.action === 'overwrite' });
      transaction.created.push(target);
    }
    transaction.transferred.push(target);
  } catch (error) {
    const stagedSourceAtRisk = item.operation === 'move' && await exists(temporary) && !(await exists(item.source));
    if (!stagedSourceAtRisk) await removeIfExists(temporary);
    const recoveryHint = stagedSourceAtRisk ? `；源文件保留在临时路径 ${temporary}` : '';
    throw new Error(`${path.basename(item.source)} 整理失败：${error.message}${recoveryHint}`);
  }
}

async function rollback(transaction) {
  for (const moved of [...transaction.moved].reverse()) {
    if (await exists(moved.target) && !(await exists(moved.source))) {
      await fsp.mkdir(path.dirname(moved.source), { recursive: true }).catch(() => {});
      await fsp.rename(moved.target, moved.source).catch(() => {});
    }
  }
  for (const target of [...transaction.created].reverse()) await removeIfExists(target);
  for (const backup of [...transaction.backups].reverse()) {
    await removeIfExists(backup.target);
    if (await exists(backup.backup)) await fsp.rename(backup.backup, backup.target).catch(() => {});
  }
}

async function cleanupEmptyParents(source, boundary) {
  const root = path.resolve(boundary);
  let current = path.dirname(path.resolve(source));
  while (current === root || current.startsWith(`${root}${path.sep}`)) {
    const entries = await fsp.readdir(current).catch(() => null);
    if (!entries || entries.length) break;
    await fsp.rmdir(current).catch(() => {});
    if (current === root) break;
    current = path.dirname(current);
  }
}

export async function executeNativePreview(preview, { fetchImpl = undiciFetch, sourceBoundary = null } = {}) {
  const classification = classifyNativePreview(preview);
  if (!classification.ready) throw new Error(classification.message);
  const items = preview.data.items;
  const transaction = { created: [], moved: [], backups: [], delete_after_commit: [], transferred: [], skipped: [] };
  try {
    for (const item of items.filter((entry) => ['video', 'subtitle', 'audio', 'trailer', 'extra'].includes(entry.kind))) await transferFile(item, transaction);
  } catch (error) {
    await rollback(transaction);
    throw error;
  }
  const warnings = [];
  const deletedAfterCommit = [];
  for (const source of transaction.delete_after_commit) {
    try {
      await fsp.rm(source, { force: true });
      deletedAfterCommit.push(source);
    } catch (error) {
      warnings.push(`${path.basename(source)}：目标已写入，但跨盘移动的源文件删除失败：${error.message}`);
    }
  }
  for (const backup of transaction.backups) await removeIfExists(backup.backup);
  if (sourceBoundary) {
    for (const moved of [...transaction.moved, ...deletedAfterCommit.map((source) => ({ source }))]) await cleanupEmptyParents(moved.source, sourceBoundary);
  }
  let scraped = 0;
  for (const item of items.filter((entry) => entry.kind === 'nfo' || entry.kind === 'image')) {
    if (item.action === 'skip') {
      transaction.skipped.push(item.target);
      continue;
    }
    try {
      const options = { overwrite: item.action === 'overwrite' };
      if (item.kind === 'nfo') await atomicWrite(item.target, renderNfo(item.generator, preview.metadata), options);
      else await downloadImage(item.source, item.target, fetchImpl, options);
      scraped += 1;
    } catch (error) {
      warnings.push(`${path.basename(item.target)}：${error.message}`);
    }
  }
  return {
    success: true,
    transferred: transaction.transferred.length,
    skipped: transaction.skipped.length,
    scraped,
    warnings,
    targets: transaction.transferred,
  };
}

export const organizerCoreInternals = {
  bestSidecarVideo,
  cleanTitle,
  languageSuffix,
  releaseEdition,
  releasePart,
  releaseQuality,
  tvNumbers,
};
