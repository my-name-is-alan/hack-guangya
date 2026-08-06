import assert from 'node:assert/strict';
import http from 'node:http';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { DatabaseSync } from 'node:sqlite';
import test from 'node:test';
import {
  NATIVE_ENGINE_VERSION,
  classifyNativePreview,
  executeNativePreview,
  parseMediaName,
  renderNfo,
  normalizeOrganizerCloudEntry,
  cloudCandidateFingerprint,
  renderOrganizerPathTemplate,
  scoreTmdbCandidate,
  titleSimilarity,
} from './organizer-core.mjs';
import { createOrganizerService } from './organizer.mjs';

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
    renameEntry: async (id, name) => { nodes.get(id).name = name; },
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
  const settings = service.updateSettings({ api_key: 'unit-key', minimum_match_score: 0.4 });
  assert.equal(settings.configured, true);
  assert.equal(Object.hasOwn(settings, 'api_key'), false);
  assert.equal(settings.default_scrape_types.includes('episode_nfo'), false);
  assert.deepEqual(await service.testConnection(), { success: true, message: 'TMDB 连接成功' });
  await assert.rejects(service.addMapping({ source_dir_id: 'a', target_dir_id: 'a', source_path: '/A', target_path: '/A' }), /不能相同/);
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
  assert.match(ready.preview.data.items.find((item) => item.kind === 'video').target_relative, /^电影\/US\/1999\/The Matrix/);
  assert.equal(ready.preview.data.items.filter((item) => item.kind === 'nfo').length, 1);
  assert.equal(ready.preview.data.items.some((item) => item.kind === 'image' && item.image_role === 'fanart'), false);
  const completed = await service.runJob(ready.id);
  assert.equal(completed.status, 'completed_warning');
  assert.equal(completed.result.transferred, 2);
  assert.equal(completed.result.scraped, 2);
  assert.equal(completed.result.share.share_url, 'https://share.example/fresh-b');
  assert.equal(shares.length, 1);
  assert.equal(shares[0].remoteTargetId === 'a' || shares[0].remoteTargetId === 'movie-dir', false);
  assert.equal(nodes.get('movie-file').parent_id === 'movie-dir', false);
  assert.equal(completed.result.warnings.some((warning) => warning.includes('已有分享失效')), true);
  await assert.rejects(service.runJob(ready.id), /已经整理完成/);
  assert.equal(fake.requests.every((request) => request.path.startsWith('/image/') || request.apiKey === 'unit-key'), true);
  assert.equal(events.some((event) => event.type === 'organizer' && event.event === 'job-updated'), true);
  assert.deepEqual(service.removeJob(completed.id), {});
  assert.equal(service.state().jobs.some((job) => job.id === completed.id), false);
});
