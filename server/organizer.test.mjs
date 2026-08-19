import assert from 'node:assert/strict';
import http from 'node:http';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import test from 'node:test';
import {
  NATIVE_ENGINE_VERSION,
  DEFAULT_ORGANIZER_SETTINGS,
  classifyNativePreview,
  collectReplacedCloudFiles,
  compareMediaVersions,
  executeNativePreview,
  analyzeCloudMediaCandidate,
  findExistingCloudVersions,
  normalizeUpgradeCriteria,
  parseMediaName,
  renderNfo,
  normalizeOrganizerCloudEntry,
  cloudCandidateFingerprint,
  planCloudScrapeCandidates,
  resolveMediaCategory,
  renderOrganizerPathTemplate,
  resolveTmdbMatch,
  scoreTmdbCandidate,
  titleSimilarity,
} from './organizer-core.mjs';
import { createOrganizerService, parseFfprobeTechnicalData } from './organizer.mjs';

test('洗版比较器按配置的优先级维度逐项分胜负', () => {
  assert.deepEqual(normalizeUpgradeCriteria(['size', 'resolution']), ['size', 'resolution']);
  assert.deepEqual(normalizeUpgradeCriteria([]), ['resolution', 'dynamic_range', 'release_group', 'size']);
  assert.deepEqual(normalizeUpgradeCriteria(['bogus']), ['resolution', 'dynamic_range', 'release_group', 'size']);

  const parse = (name) => ({ parsed: parseMediaName(name), size: 0 });
  // 分辨率优先：2160p 胜 1080p
  assert.deepEqual(
    compareMediaVersions(parse('Movie.2020.2160p.WEB-DL.mkv'), parse('Movie.2020.1080p.BluRay.mkv'), {}),
    { winner: 'next', criterion: 'resolution' },
  );
  // 同分辨率时动态范围决定：DV 胜 HDR10，HDR10 胜 SDR
  assert.deepEqual(
    compareMediaVersions(parse('Movie.2020.2160p.DV.mkv'), parse('Movie.2020.2160p.HDR10.mkv'), {}),
    { winner: 'next', criterion: 'dynamic_range' },
  );
  assert.deepEqual(
    compareMediaVersions(parse('Movie.2020.2160p.mkv'), parse('Movie.2020.2160p.HDR.mkv'), {}),
    { winner: 'existing', criterion: 'dynamic_range' },
  );
  // 制作组名单顺序：FRDS 优先于 WiKi；未配置名单时跳过该维度
  assert.deepEqual(
    compareMediaVersions(
      parse('Movie.2020.1080p.BluRay-FRDS.mkv'),
      parse('Movie.2020.1080p.BluRay-WiKi.mkv'),
      { releaseGroups: 'FRDS\nWiKi' },
    ),
    { winner: 'next', criterion: 'release_group' },
  );
  // 大小兜底
  assert.deepEqual(
    compareMediaVersions(
      { parsed: parseMediaName('Movie.2020.1080p.mkv'), size: 200 },
      { parsed: parseMediaName('Movie.2020.1080p.mkv'), size: 100 },
      {},
    ),
    { winner: 'next', criterion: 'size' },
  );
  // 全部持平 = 同版本
  assert.deepEqual(compareMediaVersions(parse('A.2020.1080p.mkv'), parse('B.2020.1080p.mkv'), {}), { winner: 'tie', criterion: null });
  // 自定义顺序：大小在前时优先比大小
  assert.deepEqual(
    compareMediaVersions(
      { parsed: parseMediaName('Movie.2020.1080p.mkv'), size: 500 },
      { parsed: parseMediaName('Movie.2020.2160p.mkv'), size: 100 },
      { criteria: ['size', 'resolution'] },
    ),
    { winner: 'next', criterion: 'size' },
  );
});

test('同一版本识别：电影按 part、剧集按季集匹配，并连带伴随文件进入替换清单', () => {
  const movieEntries = [
    { id: 'old-video', name: 'Movie (2020) - 1080p BluRay x264.mkv', is_directory: false, size: 100 },
    { id: 'old-sub', name: 'Movie (2020) - 1080p BluRay x264.chs.srt', is_directory: false, size: 1 },
    { id: 'old-nfo', name: 'Movie (2020) - 1080p BluRay x264.nfo', is_directory: false, size: 1 },
    { id: 'poster', name: 'poster.jpg', is_directory: false, size: 1 },
    { id: 'cd2', name: 'Movie (2020) - 1080p BluRay x264 - CD2.mkv', is_directory: false, size: 100 },
    { id: 'dir', name: 'extras', is_directory: true, size: 0 },
  ];
  const movieVersions = findExistingCloudVersions({
    mediaType: 'movie',
    parsed: parseMediaName('Movie.2020.2160p.WEB-DL.mkv'),
    entries: movieEntries,
  });
  assert.deepEqual(movieVersions.map((version) => version.entry.id), ['old-video']);
  const replaces = collectReplacedCloudFiles(movieVersions, movieEntries);
  assert.deepEqual(replaces.map((entry) => entry.id).sort(), ['old-nfo', 'old-sub', 'old-video']);

  const tvEntries = [
    { id: 'e1', name: 'Show - S01E01 - 1080p.mkv', is_directory: false, size: 10 },
    { id: 'e2', name: 'Show - S01E02 - 1080p.mkv', is_directory: false, size: 10 },
  ];
  const tvVersions = findExistingCloudVersions({
    mediaType: 'tv',
    parsed: parseMediaName('Show.S01E02.2160p.mkv'),
    entries: tvEntries,
  });
  assert.deepEqual(tvVersions.map((version) => version.entry.id), ['e2']);
});

test('集偏移把识别出的集号统一平移', () => {
  const analysis = analyzeCloudMediaCandidate({
    candidate: { fileId: 'dir', fileName: 'Show (2024)', resType: 2, path: 'Show (2024)' },
    entries: [
      { fileId: 'v1', fileName: 'Show.S01E01.1080p.mkv', resType: 1, path: 'Show (2024)/Show.S01E01.1080p.mkv', fileSize: 10 },
      { fileId: 'v2', fileName: 'Show.S01E02.1080p.mkv', resType: 1, path: 'Show (2024)/Show.S01E02.1080p.mkv', fileSize: 10 },
    ],
  }, { media_type: 'tv', episode_offset: 12 });
  assert.deepEqual(analysis.videos.map((video) => video.parsed.episode), [13, 14]);
});

test('FFprobe streams become the technical naming suffix fields', () => {
  assert.deepEqual(parseFfprobeTechnicalData({ streams: [
    { codec_type: 'video', codec_name: 'hevc', width: 3840, height: 2160, avg_frame_rate: '60000/1001', pix_fmt: 'yuv420p10le', color_transfer: 'smpte2084' },
    { codec_type: 'audio', codec_name: 'eac3', channels: 6, channel_layout: '5.1(side)', tags: { title: 'Dolby Atmos JOC' } },
  ] }), {
    video_format: '2160p', video_codec: 'HEVC', frame_rate: '59.94fps', color_depth: '10bit',
    dynamic_range: 'HDR10', audio_codec: 'DDP', audio_info: 'Atmos 5.1', effect: 'HDR10',
  });
});

async function waitFor(check, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = check();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 25));
  }
  throw new Error('等待整理任务状态超时');
}

async function startFakeTmdb() {
  const requests = [];
  const server = http.createServer((request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    requests.push({ path: url.pathname, apiKey: url.searchParams.get('api_key'), authorization: request.headers.authorization || '' });
    if (url.pathname.startsWith('/image/')) {
      response.writeHead(200, { 'content-type': 'image/jpeg' });
      response.end(Buffer.from([0xff, 0xd8, 0xff, 0xd9]));
      return;
    }
    response.setHeader('content-type', 'application/json');
    if (url.pathname === '/3/configuration') {
      response.end(JSON.stringify({ images: { secure_base_url: 'https://image.tmdb.org/t/p/' } }));
      return;
    }
    if (url.pathname === '/3/search/movie') {
      response.end(JSON.stringify({ results: [{
        id: 603,
        title: 'The Matrix',
        original_title: 'The Matrix',
        release_date: '1999-03-30',
        overview: 'A simulated world.',
        vote_average: 8.2,
        popularity: 95,
        poster_path: '/matrix-poster.jpg',
      }] }));
      return;
    }
    if (url.pathname === '/3/movie/603') {
      response.end(JSON.stringify({
        id: 603,
        imdb_id: 'tt0133093',
        title: 'The Matrix',
        original_title: 'The Matrix',
        release_date: '1999-03-30',
        overview: 'A simulated world.',
        tagline: 'Free your mind.',
        runtime: 136,
        vote_average: 8.2,
        vote_count: 25000,
        genres: [{ name: 'Science Fiction' }],
        production_companies: [{ name: 'Warner Bros.' }],
        production_countries: [{ iso_3166_1: 'US' }],
        poster_path: '/matrix-poster.jpg',
        backdrop_path: '/matrix-backdrop.jpg',
        credits: { cast: [{ name: 'Keanu Reeves', character: 'Neo', order: 0 }], crew: [{ name: 'Lana Wachowski', job: 'Director' }] },
      }));
      return;
    }
    if (url.pathname === '/3/search/tv') {
      response.end(JSON.stringify({ results: [
        { id: 111, name: 'Foundation Story', original_name: 'Foundation Story', first_air_date: '2021-01-01', popularity: 20, vote_average: 7, poster_path: '/candidate-1.jpg' },
        { id: 112, name: 'Foundation Story', original_name: 'Foundation Story', first_air_date: '2021-01-01', popularity: 20, vote_average: 7, poster_path: '/candidate-2.jpg' },
      ] }));
      return;
    }
    if (url.pathname === '/3/tv/93740') {
      response.end(JSON.stringify({
        id: 93740,
        name: 'Foundation',
        original_name: 'Foundation',
        first_air_date: '2021-09-24',
        overview: 'A galactic epic.',
        vote_average: 7.8,
        vote_count: 1300,
        genres: [{ name: 'Sci-Fi & Fantasy' }],
        production_companies: [{ name: 'Skydance Television' }],
        production_countries: [{ iso_3166_1: 'US' }],
        poster_path: '/foundation-poster.jpg',
        backdrop_path: '/foundation-backdrop.jpg',
        credits: { cast: [{ name: 'Jared Harris', character: 'Hari Seldon', order: 0 }], crew: [] },
        external_ids: { imdb_id: 'tt0804484' },
      }));
      return;
    }
    if (url.pathname === '/3/tv/93740/season/1') {
      response.end(JSON.stringify({
        season_number: 1,
        name: 'Season 1',
        overview: 'The first season.',
        air_date: '2021-09-24',
        poster_path: '/foundation-season-1.jpg',
        episodes: [{ episode_number: 1, season_number: 1, name: "The Emperor's Peace", overview: 'The story begins.', air_date: '2021-09-24', runtime: 69, vote_average: 7.7 }],
      }));
      return;
    }
    response.writeHead(404);
    response.end(JSON.stringify({ status_message: `unknown fake path ${url.pathname}` }));
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const port = server.address().port;
  return {
    server,
    requests,
    apiBase: `http://127.0.0.1:${port}/3`,
    imageBase: `http://127.0.0.1:${port}/image`,
  };
}

test('native parser extracts movie, season, episode range, quality and edition', () => {
  const tv = parseMediaName('[Group] Example.Show.S02E03-E04.2160p.WEB-DL.x265.mkv', { media_type: 'tv' });
  assert.equal(tv.title, 'Example Show');
  assert.equal(tv.season, 2);
  assert.equal(tv.episode, 3);
  assert.equal(tv.episode_end, 4);
  assert.match(tv.quality, /2160p/i);
  const chinese = parseMediaName('庆余年.第2季.第12集.1080p.mkv', { media_type: 'tv' });
  assert.equal(chinese.title, '庆余年');
  assert.equal(chinese.season, 2);
  assert.equal(chinese.episode, 12);
  const movie = parseMediaName('Blade.Runner.1982.Directors.Cut.1080p.BluRay.mkv');
  assert.equal(movie.title, 'Blade Runner');
  assert.equal(movie.year, 1982);
  assert.equal(movie.edition, 'Director’s Cut');
});

test('分词状态机拆分中英文标题并识别字幕组/动漫命名', () => {
  // 中英混合：中文名与英文名分别累计
  const mixed = parseMediaName('凡人修仙传.The.Immortal.Ascension.2020.S01E05.2160p.WEB-DL.mkv');
  assert.equal(mixed.cn_name, '凡人修仙传');
  assert.equal(mixed.en_name, 'The Immortal Ascension');
  assert.equal(mixed.title, '凡人修仙传');
  assert.equal(mixed.year, 2020);
  assert.equal(mixed.season, 1);
  assert.equal(mixed.episode, 5);
  // 字幕组多括号 + 动漫集号
  const anime = parseMediaName('【幻月字幕组】【4月新番】【天国大魔境 Tengoku Daimakyou】【01】【1080P】【简日双语】.mp4');
  assert.equal(anime.cn_name, '天国大魔境');
  assert.equal(anime.en_name, 'Tengoku Daimakyou');
  assert.equal(anime.episode, 1);
  assert.equal(anime.media_type, 'tv');
  // 英文动漫 "- 05" 集号
  const subs = parseMediaName('[SubsPlease] Kaiju No. 8 - 05 (1080p) [E02DE726].mkv');
  assert.equal(subs.title, 'Kaiju No 8');
  assert.equal(subs.episode, 5);
  // 纯数字标题不误判为集号（300 是标题，2006 是年份）
  const numeric = parseMediaName('300.2006.1080p.BluRay.x264.mkv');
  assert.equal(numeric.title, '300');
  assert.equal(numeric.year, 2006);
  assert.equal(numeric.media_type, 'movie');
  // HDHive/Emby 风格内嵌 TMDB ID（{tmdb-x}/{tmdbid-x}/[tmdb=x]/-Tmdbx）
  assert.equal(parseMediaName('遥远的桥 (1977) {tmdb-5902}').tmdb_id, 5902);
  assert.equal(parseMediaName('苦尽柑来遇见你 (2025) {tmdbid-219246}').tmdb_id, 219246);
  assert.equal(parseMediaName('热血青春 (2014) {tmdb=252067}').tmdb_id, 252067);
  assert.equal(parseMediaName('哦我的鬼神大人 (2015)-Tmdb63119').tmdb_id, 63119);
  assert.equal(parseMediaName('赌金.Gold Land (2026) [tmdbid-278113]').tmdb_id, 278113);
  // 中文数字季 + 完结区间
  const chineseSeason = parseMediaName('披荆斩棘的哥哥.第三季.EP01.2023.1080p.WEB-DL.mp4', { media_type: 'tv' });
  assert.equal(chineseSeason.season, 3);
  assert.equal(chineseSeason.episode, 1);
  const finRange = parseMediaName('[01-26Fin] 某动画 1080p');
  assert.equal(finRange.episode, 1);
  assert.equal(finRange.episode_end, 26);
});

test('TMDB 匹配：中英文名逐个搜索、精确命中优先、电影→剧集→multi 回退', async (t) => {
  const calls = [];
  const makeClient = (handlers) => ({
    async search(query) { calls.push(['search', query.media_type, query.title, query.year ?? null]); return handlers.search?.(query) || []; },
    async searchMulti(query) { calls.push(['multi', query.title]); return handlers.searchMulti?.(query) || []; },
    async alternativeNames(mediaType, tmdbId) { calls.push(['names', mediaType, tmdbId]); return handlers.alternativeNames?.(mediaType, tmdbId) || []; },
    async details(mediaType, tmdbId) { return { tmdb_id: tmdbId, media_type: mediaType, title: `T${tmdbId}`, original_title: `T${tmdbId}`, year: 2020, release_date: '2020-01-01', overview: '', vote_average: 8, poster_path: '', poster_url: '', seasons: {} }; },
    async season() { return { season_number: 1, episodes: [] }; },
  });
  const analysisBase = { title: '凡人修仙传', title_candidates: ['凡人修仙传', 'The Immortal Ascension'], year: 2020, media_type: 'tv', tmdb_id: null, videos: [], query: {} };

  await t.test('中文名精确命中直接采用（TMDB 级别优先）', async () => {
    const client = makeClient({
      search: (query) => query.title === '凡人修仙传'
        ? [{ tmdb_id: 91557, media_type: 'tv', title: '凡人修仙传', original_title: 'Fan Ren Xiu Xian Zhuan', year: 2020, release_date: '2020-07-25', popularity: 50, score: 0.5 }]
        : [],
    });
    const match = await resolveTmdbMatch({ analysis: analysisBase, client, settings: DEFAULT_ORGANIZER_SETTINGS });
    assert.equal(match.ready, true);
    assert.equal(match.selected.tmdb_id, 91557);
  });

  await t.test('中文名无结果时用英文名命中', async () => {
    const client = makeClient({
      search: (query) => query.title === 'The Immortal Ascension'
        ? [{ tmdb_id: 91557, media_type: 'tv', title: 'The Immortal Ascension', original_title: '凡人修仙传', year: 2020, release_date: '2020-07-25', popularity: 50, score: 0.5 }]
        : [],
    });
    const match = await resolveTmdbMatch({ analysis: analysisBase, client, settings: DEFAULT_ORGANIZER_SETTINGS });
    assert.equal(match.ready, true);
    assert.equal(match.selected.tmdb_id, 91557);
  });

  await t.test('电影查不到时回退剧集，multi 兜底', async () => {
    const analysis = { ...analysisBase, media_type: 'movie', title: 'Some Movie', title_candidates: ['Some Movie'], year: null };
    const client = makeClient({
      searchMulti: () => [{ tmdb_id: 777, media_type: 'tv', title: 'Some Movie', original_title: 'Some Movie', year: 2021, release_date: '2021-01-01', popularity: 10, score: 0.4 }],
    });
    const match = await resolveTmdbMatch({ analysis, client, settings: DEFAULT_ORGANIZER_SETTINGS });
    assert.equal(match.ready, true);
    assert.equal(match.selected.tmdb_id, 777);
    // 详情按选中项的实际类型（tv）拉取
    assert.equal(match.query.media_type, 'tv');
  });

  await t.test('别名/译名第二轮精确匹配', async () => {
    const analysis = { ...analysisBase, title: '沙丘2', title_candidates: ['沙丘2'], year: null, media_type: 'movie' };
    const client = makeClient({
      search: (query) => query.media_type === 'movie' && query.title === '沙丘2'
        ? [{ tmdb_id: 693134, media_type: 'movie', title: 'Dune: Part Two', original_title: 'Dune: Part Two', year: 2024, release_date: '2024-02-27', popularity: 500, score: 0.3 }]
        : [],
      alternativeNames: () => ['Dune: Part Two', '沙丘2', '沙丘：第二部'],
    });
    const match = await resolveTmdbMatch({ analysis, client, settings: DEFAULT_ORGANIZER_SETTINGS });
    assert.equal(match.ready, true);
    assert.equal(match.selected.tmdb_id, 693134);
  });

  await t.test('有年份时信任 TMDB 排序取 ±1 年首个（HDHive 语义）', async () => {
    const analysis = { ...analysisBase, title: '某剧完全不同名', title_candidates: ['某剧完全不同名'], year: 2020, media_type: 'tv' };
    const client = makeClient({
      search: (query) => query.title === '某剧完全不同名' && query.media_type === 'tv'
        ? [
          { tmdb_id: 1, media_type: 'tv', title: '别的名字A', original_title: 'Other A', year: 2015, release_date: '2015-01-01', popularity: 5, score: 0.2 },
          { tmdb_id: 2, media_type: 'tv', title: '别的名字B', original_title: 'Other B', year: 2021, release_date: '2021-01-01', popularity: 5, score: 0.2 },
        ]
        : [],
      alternativeNames: () => [],
    });
    const match = await resolveTmdbMatch({ analysis, client, settings: DEFAULT_ORGANIZER_SETTINGS });
    assert.equal(match.ready, true);
    assert.equal(match.selected.tmdb_id, 2);
  });
});

test('auxiliary recognition applies capture math, forced TMDB and rich naming metadata', () => {
  const parsed = parseMediaName('Alias.24.2160p.WEB-DL.H.265.DDP5.1-WiKi.mkv', {
    media_type: 'tv',
    recognition_words: String.raw`(?i)^Alias\.(\d+) => Example.Show.S01E\1@-12{[tmdbid=93740;type=tv]}`,
    release_groups: 'WiKi',
    render_words: String.raw`(?i)H[ .]?265 => HEVC`,
  });
  assert.equal(parsed.title, 'Example Show');
  assert.equal(parsed.season, 1);
  assert.equal(parsed.episode, 12);
  assert.equal(parsed.tmdb_id, 93740);
  assert.equal(parsed.year, null);
  assert.equal(parsed.video_format, '2160p');
  assert.equal(parsed.resource_type, 'WEB-DL');
  assert.equal(parsed.release_type, 'WEB-DL');
  assert.equal(parsed.video_codec, 'HEVC');
  assert.equal(parsed.audio_codec, 'DDP');
  assert.equal(parsed.audio_info, '5.1');
  assert.equal(parsed.release_group, 'WiKi');
});

test('secondary categories combine genre language and country and preserve nested paths', () => {
  const settings = {
    movie_category: '电影',
    tv_category: '电视剧',
    category_rules: [
      { name: '电视剧/动漫/国漫', media_type: 'tv', genres: ['16'], origin_countries: ['CN', 'TW', 'HK'], enabled: true },
      { name: '电视剧/亚洲剧/国产剧', media_type: 'tv', origin_countries: ['CN'], enabled: true },
    ],
  };
  assert.equal(resolveMediaCategory({ media_type: 'tv', genre_ids: [16], origin_countries: ['CN'], original_language: 'zh' }, settings), '电视剧/动漫/国漫');
  assert.equal(resolveMediaCategory({ media_type: 'tv', genre_ids: [18], origin_countries: ['CN'], original_language: 'zh' }, settings), '电视剧/亚洲剧/国产剧');
  assert.equal(resolveMediaCategory({ media_type: 'tv', genres: ['动画与冒险'], origin_countries: ['CN'] }, {
    ...settings,
    category_rules: [{ name: '电视剧/动漫', media_type: 'tv', genres: ['动画'], enabled: true }],
  }), '电视剧/动漫');
});

test('ambiguous cloud sidecars stay unbound instead of following an unrelated prefix', () => {
  const analysis = analyzeCloudMediaCandidate({
    candidate: { id: 'root', name: 'Shows', path: 'Shows', is_directory: true },
    entries: [
      { id: 'v1', parent_id: 'root', name: 'Alpha.S01E01.mkv', path: 'Shows/Alpha.S01E01.mkv' },
      { id: 'v2', parent_id: 'root', name: 'Beta.S01E02.mkv', path: 'Shows/Beta.S01E02.mkv' },
      { id: 's1', parent_id: 'root', name: 'Unrelated.srt', path: 'Shows/Unrelated.srt' },
    ],
  }, { media_type: 'tv' });
  assert.equal(analysis.sidecars[0].video_source, null);
});

test('large mixed cloud folders are split into stable movie and series boundaries', () => {
  const planned = planCloudScrapeCandidates({
    candidate: { id: 'root', name: 'Downloads', path: 'Downloads', is_directory: true },
    entries: [
      { id: 'show', parent_id: 'root', name: 'Foundation', path: 'Downloads/Foundation', is_directory: true },
      { id: 'season', parent_id: 'show', name: 'Season 1', path: 'Downloads/Foundation/Season 1', is_directory: true },
      { id: 'show-e1', parent_id: 'season', name: 'E01.mkv', path: 'Downloads/Foundation/Season 1/E01.mkv' },
      { id: 'show-e2', parent_id: 'season', name: 'E02.mkv', path: 'Downloads/Foundation/Season 1/E02.mkv' },
      { id: 'movie-a', parent_id: 'root', name: 'The.Matrix.1999', path: 'Downloads/The.Matrix.1999', is_directory: true },
      { id: 'movie-a-file', parent_id: 'movie-a', name: 'The.Matrix.1999.mkv', path: 'Downloads/The.Matrix.1999/The.Matrix.1999.mkv' },
      { id: 'movie-b', parent_id: 'root', name: 'Arrival.2016', path: 'Downloads/Arrival.2016', is_directory: true },
      { id: 'movie-b-file', parent_id: 'movie-b', name: 'Arrival.2016.mkv', path: 'Downloads/Arrival.2016/Arrival.2016.mkv' },
      { id: 'loose-a', parent_id: 'root', name: 'Alpha.S01E01.mkv', path: 'Downloads/Alpha.S01E01.mkv' },
      { id: 'loose-b', parent_id: 'root', name: 'Beta.S01E02.mkv', path: 'Downloads/Beta.S01E02.mkv' },
    ],
  });
  assert.deepEqual(planned.map((item) => item.id).sort(), ['loose-a', 'loose-b', 'movie-a', 'movie-b', 'show']);
  assert.equal(planned.find((item) => item.id === 'show').suggested_media_type, 'tv');
});

test('a title folder with direct episodes remains one TV candidate', () => {
  const planned = planCloudScrapeCandidates({
    candidate: { id: 'show', name: 'Foundation', path: 'Foundation', is_directory: true },
    entries: [
      { id: 'e1', parent_id: 'show', name: 'Foundation.S01E01.mkv', path: 'Foundation/Foundation.S01E01.mkv' },
      { id: 'e2', parent_id: 'show', name: 'Foundation.S01E02.mkv', path: 'Foundation/Foundation.S01E02.mkv' },
    ],
  });
  assert.deepEqual(planned.map((item) => item.id), ['show']);
  assert.equal(planned[0].reason, 'episode-folder');
});

test('candidate scoring favors exact title and year', () => {
  assert.equal(titleSimilarity('The Matrix', 'The.Matrix'), 1);
  const exact = scoreTmdbCandidate({ title: 'The Matrix', year: 1999 }, { title: 'The Matrix', release_date: '1999-03-30', popularity: 50 });
  const wrong = scoreTmdbCandidate({ title: 'The Matrix', year: 1999 }, { title: 'Matrix Resurrections', release_date: '2021-12-16', popularity: 50 });
  assert.ok(exact > 0.95);
  assert.ok(exact > wrong);
});

test('NFO output escapes metadata and contains stable TMDB identity', () => {
  const nfo = renderNfo({ type: 'movie' }, {
    tmdb_id: 7,
    imdb_id: 'tt0000007',
    title: 'A & B',
    original_title: 'A < B',
    year: 2026,
    release_date: '2026-01-01',
    overview: 'One > zero',
    genres: ['Drama'],
    studios: [],
    directors: [],
    actors: [],
  });
  assert.match(nfo, /A &amp; B/);
  assert.match(nfo, /A &lt; B/);
  assert.match(nfo, /<uniqueid type="tmdb" default="true">7<\/uniqueid>/);
});

test('cloud path templates support aliases, padding and safe relative paths', () => {
  const rendered = renderOrganizerPathTemplate(
    '{catgroy}/{country}/{year}/{title} - {tmdbid}/{Season x}/{title}.{Season x}{Expose n}.{ext}',
    { category: '电视剧', country: 'CN', year: 2026, title: '示例剧', tmdb_id: 42, season: 2, episode: 3, season_tag: 'S02', episode_tag: 'E03', ext: 'mkv' },
  );
  assert.equal(rendered, '电视剧/CN/2026/示例剧 - 42/S02/示例剧.S02E03.mkv');
  assert.throws(() => renderOrganizerPathTemplate('../{title}.{ext}', { title: 'x', ext: 'mkv' }), /至少包含一个目录|相对目录/);
  assert.equal(
    renderOrganizerPathTemplate('{category}/{title}.{Season x}{Expose n}.mkv', {
      category: '电视剧', title: '示例剧', season_tag: 'S01', episode_tag: 'E02',
    }),
    '电视剧/示例剧.S01E02.mkv',
  );
  assert.equal(
    renderOrganizerPathTemplate('{{category}}/{{title}}/{{en_title}}.{{season_episode}}.{{videoCodec}}-{{releaseGroup}}{{fileExt}}', {
      category: '电视剧/动漫/国漫', title: '示例剧', en_title: 'Example', season_episode: 'S01E02', videoCodec: 'HEVC', releaseGroup: 'WiKi', fileExt: '.mkv',
    }),
    '电视剧/动漫/国漫/示例剧/Example.S01E02.HEVC-WiKi.mkv',
  );
  assert.equal(
    renderOrganizerPathTemplate('{category}/{videoformat}.{releasetype}.{releasegroup}{fileext}', {
      category: '电视剧', videoFormat: '2160p', releaseType: 'WEB-DL', releaseGroup: 'WiKi', fileExt: '.mkv',
    }),
    '电视剧/2160p.WEB-DL.WiKi.mkv',
  );
  assert.equal(
    renderOrganizerPathTemplate('{category}/{title}{{@if@}}-{{releaseGroup}}{{@endif@}}{{fileExt}}', {
      category: '电影', title: '示例', releaseGroup: 'WiKi', fileExt: '.mkv',
    }),
    '电影/示例-WiKi.mkv',
  );
  assert.equal(
    renderOrganizerPathTemplate('{category}/{title}{{@if@}}-{{releaseGroup}}{{@endif@}}{{fileExt}}', {
      category: '电影', title: '示例', releaseGroup: '', fileExt: '.mkv',
    }),
    '电影/示例.mkv',
  );
});

test('cloud organizer fingerprints use Guangya utime/ctime fields', () => {
  const candidate = normalizeOrganizerCloudEntry({ fileId: 'dir', fileName: 'A', resType: 2, utime: 10 });
  const first = normalizeOrganizerCloudEntry({ fileId: 'video', fileName: 'A.mkv', resType: 1, fileSize: 1, parentId: 'dir', utime: 10 }, 'A.mkv');
  const second = { ...first, raw: { ...first.raw, utime: 11 }, modified_ms: '11' };
  assert.notEqual(cloudCandidateFingerprint(candidate, [first]).signature, cloudCandidateFingerprint(candidate, [second]).signature);
});

test('move transaction rolls back transferred files when a later item fails', async (context) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-organizer-rollback-'));
  context.after(() => fsp.rm(root, { recursive: true, force: true }));
  const source = path.join(root, 'source.mkv');
  const missing = path.join(root, 'missing.mkv');
  const firstTarget = path.join(root, 'library', 'source.mkv');
  const secondTarget = path.join(root, 'library', 'missing.mkv');
  await fsp.writeFile(source, 'video');
  const preview = {
    success: true,
    engine: NATIVE_ENGINE_VERSION,
    message: 'ready',
    metadata: {},
    data: { items: [
      { success: true, kind: 'video', source, target: firstTarget, operation: 'move', action: 'create' },
      { success: true, kind: 'video', source: missing, target: secondTarget, operation: 'move', action: 'create' },
    ] },
  };
  assert.deepEqual(classifyNativePreview(preview), { ready: true, error_code: null, message: 'ready' });
  await assert.rejects(executeNativePreview(preview), /missing\.mkv/);
  assert.equal(await fsp.readFile(source, 'utf8'), 'video');
  await assert.rejects(fsp.access(firstTarget));
});

test('failed metadata overwrite preserves the existing scraped file', async (context) => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-organizer-metadata-'));
  context.after(() => fsp.rm(root, { recursive: true, force: true }));
  const poster = path.join(root, 'poster.jpg');
  await fsp.writeFile(poster, 'existing-poster');
  const preview = {
    success: true,
    engine: NATIVE_ENGINE_VERSION,
    message: 'ready',
    metadata: {},
    data: { items: [
      { success: true, kind: 'video', source: path.join(root, 'source.mkv'), target: path.join(root, 'movie.mkv'), operation: 'copy', action: 'skip' },
      { success: true, kind: 'image', source: 'https://invalid.example/poster.jpg', target: poster, operation: 'download', action: 'overwrite' },
    ] },
  };
  const result = await executeNativePreview(preview, { fetchImpl: async () => { throw new Error('offline'); } });
  assert.equal(result.warnings.length, 1);
  assert.equal(await fsp.readFile(poster, 'utf8'), 'existing-poster');
});

test('cloud-native organizer moves A to B, scrapes selected types and creates a fresh B share', async (context) => {
  const fake = await startFakeTmdb();
  const database = new DatabaseSync(':memory:');
  const events = [];
  const shares = [];
  const batchMoves = [];
  let activeRenames = 0;
  let maximumConcurrentRenames = 0;
  const nodes = new Map([
    ['a', { id: 'a', name: 'A', parent_id: '', is_directory: true, size: 0, modified_ms: '1' }],
    ['b', { id: 'b', name: 'B', parent_id: '', is_directory: true, size: 0, modified_ms: '1' }],
    ['movie-dir', { id: 'movie-dir', name: 'The.Matrix.1999', parent_id: 'a', is_directory: true, size: 0, modified_ms: '2' }],
    ['movie-file', { id: 'movie-file', name: 'The.Matrix.1999.1080p.mkv', parent_id: 'movie-dir', is_directory: false, size: 32, modified_ms: '3' }],
    ['subtitle-file', { id: 'subtitle-file', name: 'The.Matrix.1999.1080p.zh-CN.srt', parent_id: 'movie-dir', is_directory: false, size: 8, modified_ms: '3' }],
  ]);
  let sequence = 0;
  const children = (parentId) => [...nodes.values()].filter((entry) => entry.parent_id === parentId).map((entry) => ({ ...entry }));
  const cloud = {
    isAuthenticated: () => true,
    listChildren: async (parentId) => children(parentId),
    createDirectory: async (parentId, name) => {
      const id = `dir-${++sequence}`;
      const node = { id, name, parent_id: parentId, is_directory: true, size: 0, modified_ms: String(Date.now()) };
      nodes.set(id, node);
      return { ...node };
    },
    copyEntry: async (id, parentId) => {
      const source = nodes.get(id);
      const copy = { ...source, id: `copy-${++sequence}`, parent_id: parentId, modified_ms: String(Date.now()) };
      nodes.set(copy.id, copy);
      return { ...copy };
    },
    moveEntry: async (id, parentId) => { nodes.get(id).parent_id = parentId; },
    moveEntries: async (ids, parentId) => {
      batchMoves.push([...ids]);
      for (const id of ids) nodes.get(id).parent_id = parentId;
    },
    renameEntry: async (id, name) => {
      activeRenames += 1;
      maximumConcurrentRenames = Math.max(maximumConcurrentRenames, activeRenames);
      await new Promise((resolve) => setTimeout(resolve, 5));
      nodes.get(id).name = name;
      activeRenames -= 1;
    },
    deleteEntry: async (id) => { nodes.delete(id); },
    uploadBuffer: async (parentId, name, bytes) => {
      const id = `upload-${++sequence}`;
      nodes.set(id, { id, name, parent_id: parentId, is_directory: false, size: bytes.length, modified_ms: String(Date.now()) });
      return { id };
    },
    shareAfterOrganize: async (request) => {
      shares.push(request);
      return { share_url: 'https://share.example/fresh-b', reused_existing: false };
    },
  };
  const service = createOrganizerService({
    database,
    cloud,
    publish: (event) => events.push(event),
    env: { TMDB_API_BASE: fake.apiBase, TMDB_IMAGE_BASE: fake.imageBase },
    fetchImpl: async (url, options) => {
      if (String(url).includes('/image/')) return new Response(Buffer.from([0xff, 0xd8, 0xff, 0xd9]), { status: 200, headers: { 'content-type': 'image/jpeg' } });
      return fetch(url, options);
    },
  });
  context.after(async () => {
    await service.close();
    database.close();
    await new Promise((resolve) => fake.server.close(resolve));
  });

  await assert.rejects(service.addMapping({ source_dir_id: 'a', target_dir_id: 'b', source_path: '/A', target_path: '/B' }), /TMDB/);
  const settings = service.updateSettings({
    api_key: 'unit-key',
    minimum_match_score: 0.4,
    word_segment_search: false,
    similarity_match: false,
    recognition_words: '# custom recognition',
    release_groups: 'WiKi',
    render_words: String.raw`(?i)H[ .]?265 => HEVC`,
    capture_groups: String.raw`-([A-Za-z0-9@._-]+)$`,
    scrape_targets: [{ id: 'library-b', name: '主媒体库', dir_id: 'b', path: '/B' }],
  });
  assert.equal(settings.configured, true);
  assert.equal(Object.hasOwn(settings, 'api_key'), false);
  assert.equal(settings.default_scrape_types.includes('episode_nfo'), false);
  assert.equal(settings.word_segment_search, false);
  assert.equal(settings.similarity_match, false);
  assert.equal(settings.recognition_words, '# custom recognition');
  assert.equal(settings.release_groups, 'WiKi');
  assert.throws(() => service.updateSettings({ recognition_words: String.raw`Show(?=\.S\d+) => Series` }), /第 1 行.*不支持|第 1 行.*正则/);
  assert.throws(() => service.updateSettings({ recognition_words: '@?{season=1} => Series' }), /尚未支持的 @\? 条件规则/);
  assert.deepEqual(await service.testConnection(), { success: true, message: 'TMDB 连接成功' });
  nodes.set('show-dir', { id: 'show-dir', name: 'Foundation.2021', parent_id: 'a', is_directory: true, size: 0, modified_ms: '2' });
  nodes.set('season-dir', { id: 'season-dir', name: 'Season 1', parent_id: 'show-dir', is_directory: true, size: 0, modified_ms: '2' });
  nodes.set('season-episode', { id: 'season-episode', name: 'E01.mkv', parent_id: 'season-dir', is_directory: false, size: 12, modified_ms: '3' });
  const seasonSubmission = await service.scrapeSelected({
    target_id: 'library-b',
    files: [{ id: 'season-dir', parent_id: 'show-dir', name: 'Season 1', parent_path: '/A/Foundation.2021' }],
  });
  assert.equal(seasonSubmission.jobs.length, 1);
  assert.equal(seasonSubmission.jobs[0].media_type, 'tv');
  assert.equal(seasonSubmission.jobs[0].query_title, 'Foundation');
  await waitFor(() => {
    const job = service.state().jobs.find((item) => item.id === seasonSubmission.jobs[0].id);
    return job && job.status !== 'recognizing' && job.status !== 'running' ? job : null;
  });
  nodes.delete('season-episode');
  nodes.delete('season-dir');
  nodes.delete('show-dir');
  await assert.rejects(service.addMapping({ source_dir_id: 'a', target_dir_id: 'a', source_path: '/A', target_path: '/A' }), /不能相同/);
  await assert.rejects(service.addMapping({ source_dir_id: 'a', target_dir_id: 'other', source_path: '/A', target_path: '/Other' }), /刮削输出/);
  await assert.rejects(service.addMapping({ source_dir_id: 'a', target_dir_id: 'b', source_path: '/A', target_path: '/B', transfer_type: 'move' }), /分享失效风险/);

  const mapping = await service.addMapping({
    source_dir_id: 'a',
    target_dir_id: 'b',
    source_path: '/A',
    target_path: '/B',
    transfer_type: 'move',
    media_type: 'movie',
    conflict_policy: 'rename',
    scrape: true,
    scrape_types: ['movie_nfo', 'poster'],
    sync_extras: true,
    scan_existing: true,
    auto_execute: false,
    share_after_organize: true,
    share_risk_acknowledged: true,
    settle_seconds: 5,
  });
  const ready = await waitFor(() => service.state().jobs.find((job) => job.mapping_id === mapping.id && job.status === 'ready'));
  assert.equal(ready.preview.engine, NATIVE_ENGINE_VERSION);
  assert.equal(ready.tmdb_id, 603);
  assert.match(ready.preview.data.items.find((item) => item.kind === 'video').target_relative, /^电影\/美国\/1999\/The Matrix/);
  assert.equal(ready.preview.data.items.filter((item) => item.kind === 'nfo').length, 1);
  assert.equal(ready.preview.data.items.some((item) => item.kind === 'image' && item.image_role === 'fanart'), false);
  const rearchiveStartedAt = Date.now();
  const submitted = await service.rearchiveJob(ready.id);
  assert.equal(submitted.id, ready.id);
  assert.equal(Date.now() - rearchiveStartedAt < 500, true);
  assert.equal(service.state().jobs.filter((job) => job.mapping_id === mapping.id && job.source_id === ready.source_id).length, 1);
  // 移动模式的固定提醒只进 message，不再把任务染成 completed_warning。
  const completed = await waitFor(() => service.state().jobs.find((job) => job.id === ready.id && job.status === 'completed'));
  assert.equal(completed.status, 'completed');
  assert.equal(completed.result.transferred, 2);
  assert.deepEqual(batchMoves, [['movie-file', 'subtitle-file']]);
  // 云端 rename 接口有并发风控（业务码 120），改名必须串行执行
  assert.equal(maximumConcurrentRenames, 1);
  assert.equal(completed.result.scraped, 2);
  assert.equal(completed.result.share.share_url, 'https://share.example/fresh-b');
  assert.equal(shares.length, 1);
  assert.equal(shares[0].remoteTargetId === 'a' || shares[0].remoteTargetId === 'movie-dir', false);
  const recreatedShare = await service.shareJob(completed.id);
  assert.equal(recreatedShare.share_url, 'https://share.example/fresh-b');
  assert.equal(shares.length, 2);
  assert.equal(shares[1].remoteTargetId, shares[0].remoteTargetId);
  assert.equal(service.state().jobs.find((job) => job.id === completed.id).result.share.share_url, 'https://share.example/fresh-b');
  assert.throws(() => service.updateSettings({ scrape_targets: [] }), /仍被整理监控使用/);
  service.updateSettings({ scrape_targets: [{ id: 'library-b', name: '主媒体库', dir_id: 'b', path: '/媒体库' }] });
  assert.equal(service.state().mappings.find((item) => item.id === mapping.id).target_path, '/媒体库');
  assert.equal(nodes.get('movie-file').parent_id === 'movie-dir', false);
  assert.equal(completed.result.warnings.some((warning) => warning.includes('已有分享失效')), false);
  assert.equal(completed.message.includes('已有分享失效'), true);
  await assert.rejects(service.runJob(completed.id), /已经整理完成/);
  assert.equal(fake.requests.every((request) => request.path.startsWith('/image/') || request.apiKey === 'unit-key'), true);
  assert.equal(events.some((event) => event.type === 'organizer' && event.event === 'job-updated'), true);
  const organizedTargetIds = [...completed.result.targets];
  const removal = await service.removeJob(completed.id, { delete_source: true, delete_target: true });
  assert.deepEqual(removal, { deleted_source: 1, deleted_target: 2, warnings: [] });
  assert.equal(nodes.has('movie-dir'), false);
  assert.equal(organizedTargetIds.every((id) => !nodes.has(id)), true);
  assert.equal(service.state().jobs.some((job) => job.id === completed.id), false);
});

test('rearchiving a completed job cleans previous outputs and empty directories before re-organizing', async (context) => {
  const fake = await startFakeTmdb();
  const database = new DatabaseSync(':memory:');
  const nodes = new Map([
    ['a', { id: 'a', name: 'A', parent_id: '', is_directory: true, size: 0, modified_ms: '1' }],
    ['b', { id: 'b', name: 'B', parent_id: '', is_directory: true, size: 0, modified_ms: '1' }],
    ['movie-dir', { id: 'movie-dir', name: 'The.Matrix.1999', parent_id: 'a', is_directory: true, size: 0, modified_ms: '2' }],
    ['movie-file', { id: 'movie-file', name: 'The.Matrix.1999.1080p.mkv', parent_id: 'movie-dir', is_directory: false, size: 32, modified_ms: '3' }],
  ]);
  let sequence = 0;
  const children = (parentId) => [...nodes.values()].filter((entry) => entry.parent_id === parentId).map((entry) => ({ ...entry }));
  const cloud = {
    isAuthenticated: () => true,
    listChildren: async (parentId) => children(parentId),
    createDirectory: async (parentId, name) => {
      const id = `dir-${++sequence}`;
      const node = { id, name, parent_id: parentId, is_directory: true, size: 0, modified_ms: String(Date.now()) };
      nodes.set(id, node);
      return { ...node };
    },
    copyEntry: async (id, parentId) => {
      const source = nodes.get(id);
      const copy = { ...source, id: `copy-${++sequence}`, parent_id: parentId, modified_ms: String(Date.now()) };
      nodes.set(copy.id, copy);
      return { ...copy };
    },
    moveEntry: async (id, parentId) => { nodes.get(id).parent_id = parentId; },
    renameEntry: async (id, name) => { nodes.get(id).name = name; },
    deleteEntry: async (id) => {
      if (!nodes.has(id)) throw new Error('文件不存在');
      nodes.delete(id);
    },
    uploadBuffer: async (parentId, name, bytes) => {
      const id = `upload-${++sequence}`;
      nodes.set(id, { id, name, parent_id: parentId, is_directory: false, size: bytes.length, modified_ms: String(Date.now()) });
      return { id };
    },
  };
  const service = createOrganizerService({
    database,
    cloud,
    publish: () => {},
    env: { TMDB_API_BASE: fake.apiBase, TMDB_IMAGE_BASE: fake.imageBase },
    fetchImpl: async (url, options) => {
      if (String(url).includes('/image/')) return new Response(Buffer.from([0xff, 0xd8, 0xff, 0xd9]), { status: 200, headers: { 'content-type': 'image/jpeg' } });
      return fetch(url, options);
    },
  });
  context.after(async () => {
    await service.close();
    database.close();
    await new Promise((resolve) => fake.server.close(resolve));
  });
  service.updateSettings({
    api_key: 'unit-key',
    word_segment_search: false,
    similarity_match: false,
    scrape_targets: [{ id: 'library-b', name: '主媒体库', dir_id: 'b', path: '/B' }],
  });
  const mapping = await service.addMapping({
    source_dir_id: 'a',
    target_dir_id: 'b',
    source_path: '/A',
    target_path: '/B',
    transfer_type: 'copy',
    media_type: 'movie',
    conflict_policy: 'rename',
    scrape: true,
    scrape_types: ['movie_nfo', 'poster'],
    scan_existing: true,
    auto_execute: true,
    settle_seconds: 5,
  });
  const completed = await waitFor(() => service.state().jobs.find((job) => job.mapping_id === mapping.id && job.status === 'completed'));
  // 执行结果必须携带完整产物清单（视频 + NFO + 海报），供重新归档时清理。
  const firstItems = completed.result.created_items;
  assert.equal(Array.isArray(firstItems), true);
  assert.deepEqual([...new Set(firstItems.map((item) => item.kind))].sort(), ['image', 'nfo', 'video']);
  assert.equal(firstItems.every((item) => item.target_relative.length > 0), true);
  const firstVideoId = firstItems.find((item) => item.kind === 'video').id;
  assert.equal(nodes.has(firstVideoId), true);
  const firstMovieDirId = nodes.get(firstVideoId).parent_id;

  await service.rearchiveJob(completed.id);
  const again = await waitFor(() => {
    const job = service.state().jobs.find((item) => item.id === completed.id);
    return job && job.status === 'completed' && job.result?.created_items?.some((item) => item.kind === 'video' && item.id !== firstVideoId) ? job : null;
  });
  // 旧视频与旧媒体目录（清空后）被回收，新一轮落位使用重建的目录。
  assert.equal(nodes.has(firstVideoId), false);
  assert.equal(nodes.has(firstMovieDirId), false);
  const secondVideoId = again.result.created_items.find((item) => item.kind === 'video').id;
  assert.equal(nodes.has(secondVideoId), true);
  const rebuiltDir = nodes.get(nodes.get(secondVideoId).parent_id);
  assert.match(rebuiltDir.name, /The Matrix/);
  // 老产物清理后不应留下重复文件：目标目录只有一份视频。
  const videosInDir = children(rebuiltDir.id).filter((entry) => entry.name.endsWith('.mkv'));
  assert.equal(videosInDir.length, 1);
});
