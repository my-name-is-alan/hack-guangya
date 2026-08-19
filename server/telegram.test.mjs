import assert from 'node:assert/strict';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import {
  chunkLines,
  createTelegramService,
  describeEmbyEvent,
  describeJobTitle,
  escapeHtml,
  findJobByRef,
  formatOrganizeDone,
  formatReviewNeeded,
  normalizeTelegramApiBaseUrl,
  parseChatIds,
  parseEmbyWebhookBody,
  parseOverrideTokens,
  parseReCommand,
  shortJobId,
} from './telegram.mjs';
import { createLogBuffer } from './log-buffer.mjs';

function createServiceForTest() {
  const database = new DatabaseSync(':memory:');
  database.exec('CREATE TABLE app_state (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_at INTEGER NOT NULL)');
  const logBuffer = createLogBuffer(100);
  const service = createTelegramService({
    database,
    env: {},
    logBuffer,
    version: '0.0.1',
    platform: 'Test',
    runtime: {},
  });
  return { service, database, logBuffer };
}

function fakeWebhookRequest(method, body, contentType) {
  return {
    method,
    headers: { 'content-type': contentType },
    async *[Symbol.asyncIterator]() {
      if (body) yield Buffer.from(body);
    },
  };
}

function fakeWebhookResponse() {
  const output = { code: 0, payload: null };
  output.writeHead = (code) => { output.code = code; };
  output.end = (raw) => { output.payload = raw ? JSON.parse(raw) : null; };
  return output;
}

test('parseReCommand 解析 tmdbid 与类型、季集号', () => {
  const parsed = parseReCommand('re ab12cd34 tmdbid=94605 tv s=1 e=3');
  assert.equal(parsed.jobRef, 'ab12cd34');
  assert.deepEqual(parsed.input, { tmdb_id: 94605, media_type: 'tv', season: 1, episode: 3 });
});

test('parseReCommand 支持裸数字与 sNN 形式', () => {
  const parsed = parseReCommand('/re ab12 94605 movie s02');
  assert.equal(parsed.jobRef, 'ab12');
  assert.deepEqual(parsed.input, { tmdb_id: 94605, media_type: 'movie', season: 2 });
});

test('parseReCommand 非 re 前缀返回 null，缺参数返回错误', () => {
  assert.equal(parseReCommand('/status'), null);
  assert.equal(parseReCommand('rearchive abc'), null);
  assert.match(parseReCommand('re').error, /用法/);
  assert.match(parseReCommand('re ab12 tmdbid=abc').error, /必须是数字/);
  assert.match(parseReCommand('re ab12 foo=bar').error, /无法识别参数/);
});

test('parseOverrideTokens 支持中文类型与 title=', () => {
  const parsed = parseOverrideTokens(['12345', '电影', 'title=沙丘', 'year=2021']);
  assert.deepEqual(parsed.input, { tmdb_id: 12345, media_type: 'movie', title: '沙丘', year: 2021 });
});

test('findJobByRef 支持短前缀且检测歧义', () => {
  const jobs = [
    { id: 'ab12cd34-0000-4000-8000-000000000001' },
    { id: 'ab12ff00-0000-4000-8000-000000000002' },
    { id: 'ff340000-0000-4000-8000-000000000003' },
  ];
  assert.equal(findJobByRef(jobs, 'ff34').job.id, jobs[2].id);
  assert.equal(findJobByRef(jobs, 'ab12cd34').job.id, jobs[0].id);
  assert.match(findJobByRef(jobs, 'ab12').error, /匹配到 2 个/);
  assert.match(findJobByRef(jobs, '9999').error, /没有找到/);
  assert.match(findJobByRef(jobs, '').error, /请提供任务 ID/);
});

test('shortJobId 去掉连字符取前 8 位', () => {
  assert.equal(shortJobId('ab12cd34-0000-4000-8000-000000000001'), 'ab12cd34');
});

test('describeJobTitle 组合标题、年份、类型与季号', () => {
  assert.equal(
    describeJobTitle({ query_title: '凡人修仙传', query_year: 2020, media_type: 'tv', season: 1 }),
    '凡人修仙传 (2020) · 剧集 S01',
  );
  assert.equal(describeJobTitle({ source_path: '/整理/来源/某电影.2021.mkv' }), '某电影.2021.mkv');
});

test('formatOrganizeDone 包含目标路径与短 ID', () => {
  const text = formatOrganizeDone({
    id: 'ab12cd34-0000-4000-8000-000000000001',
    query_title: '沙丘',
    query_year: 2021,
    media_type: 'movie',
    source_path: '/watch/dune.mkv',
    message: '云盘整理完成：转移 1 项，刮削 3 项',
    preview: { share_relative_path: '电影/沙丘 (2021)' },
  }, { target_path: '/媒体库' });
  assert.match(text, /入库完成/);
  assert.match(text, /沙丘 \(2021\)/);
  assert.match(text, /\/媒体库\/电影\/沙丘 \(2021\)/);
  assert.match(text, /ab12cd34/);
});

test('formatReviewNeeded 返回文本与操作 keyboard', () => {
  const { text, keyboard } = formatReviewNeeded({
    id: 'ab12cd34-0000-4000-8000-000000000001',
    status: 'needs_review',
    error_code: 'tmdb_not_found',
    source_path: '/watch/未知剧集.mkv',
    message: '没有找到匹配',
  });
  assert.match(text, /识别待处理/);
  assert.match(text, /TMDB 没有找到匹配条目/);
  assert.match(text, /re ab12cd34 tmdbid=12345/);
  assert.equal(keyboard.length, 2);
  assert.equal(keyboard[0][0].data, 'retry:ab12cd34-0000-4000-8000-000000000001');
  assert.equal(keyboard[1][0].data, 'ask:ab12cd34-0000-4000-8000-000000000001');
  const failed = formatReviewNeeded({ id: 'x', status: 'failed', error_code: 'transfer_failed', source_path: '/a' });
  assert.match(failed.text, /整理失败/);
});

test('parseEmbyWebhookBody 解析 JSON 与 multipart data 字段', () => {
  const payload = { Event: 'library.new', Item: { Name: '沙丘', Type: 'Movie', ProductionYear: 2021 } };
  assert.deepEqual(parseEmbyWebhookBody('application/json', Buffer.from(JSON.stringify(payload))), payload);
  const boundary = '----EmbyBoundaryX';
  const multipart = Buffer.from([
    `--${boundary}`,
    'Content-Disposition: form-data; name="data"',
    '',
    JSON.stringify(payload),
    `--${boundary}--`,
    '',
  ].join('\r\n'));
  assert.deepEqual(parseEmbyWebhookBody(`multipart/form-data; boundary=${boundary}`, multipart), payload);
  const encoded = Buffer.from(`data=${encodeURIComponent(JSON.stringify(payload))}`);
  assert.deepEqual(parseEmbyWebhookBody('application/x-www-form-urlencoded', encoded), payload);
  assert.equal(parseEmbyWebhookBody('application/json', Buffer.from('not json')), null);
});

test('describeEmbyEvent 映射入库、播放与登录事件', () => {
  const created = describeEmbyEvent({ Event: 'library.new', Item: { Name: '沙丘', Type: 'Movie', ProductionYear: 2021, Path: '/media/dune.mkv' } });
  assert.equal(created.category, 'emby_new');
  assert.match(created.text, /Emby 入库/);
  assert.match(created.text, /沙丘 \(2021\)/);

  const playback = describeEmbyEvent({
    Event: 'playback.start',
    User: { Name: 'alice' },
    Item: { Name: '第一集', Type: 'Episode', SeriesName: '凡人修仙传', ParentIndexNumber: 1, IndexNumber: 2, RunTimeTicks: 12_000_000_000 },
    Session: { DeviceName: '客厅电视', Client: 'Emby for Android' },
    PlaybackInfo: { PositionTicks: 6_000_000_000 },
  });
  assert.equal(playback.category, 'emby_play');
  assert.match(playback.text, /开始播放/);
  assert.match(playback.text, /凡人修仙传 S01E02 第一集/);
  assert.match(playback.text, /50%/);

  const login = describeEmbyEvent({ Event: 'user.authenticated', User: { Name: 'bob' }, Session: { RemoteEndPoint: '192.168.1.2' } });
  assert.equal(login.category, 'emby_login');
  assert.match(login.text, /用户登录/);
  const loginFailed = describeEmbyEvent({ Event: 'user.authenticationfailed', User: { Name: 'mallory' } });
  assert.equal(loginFailed.category, 'emby_login');
  assert.match(loginFailed.text, /登录失败/);

  assert.equal(describeEmbyEvent({ Event: 'system.serverrestartrequired' }), null);
  assert.equal(describeEmbyEvent(null), null);
});

test('chunkLines 按行拆分且单行超长截断', () => {
  const chunks = chunkLines(['a'.repeat(30), 'b'.repeat(30), 'c'.repeat(120)], 64);
  assert.equal(chunks.length, 2);
  assert.equal(chunks[0], `${'a'.repeat(30)}\n${'b'.repeat(30)}`);
  assert.equal(chunks[1].length, 64);
  assert.ok(chunks[1].endsWith('…'));
  assert.deepEqual(chunkLines(['x', 'y'], 64), ['x\ny']);
});

test('parseChatIds 过滤非法值', () => {
  assert.deepEqual(parseChatIds('123456, -1001234567890 abc; 42'), ['123456', '-1001234567890', '42']);
  assert.deepEqual(parseChatIds(''), []);
});

test('normalizeTelegramApiBaseUrl 校验并去掉末尾斜杠', () => {
  assert.equal(normalizeTelegramApiBaseUrl('https://tg.example.com/'), 'https://tg.example.com');
  assert.equal(normalizeTelegramApiBaseUrl(''), '');
  assert.throws(() => normalizeTelegramApiBaseUrl('ftp://x'), /HTTP 或 HTTPS/);
  assert.throws(() => normalizeTelegramApiBaseUrl('https://user:pass@tg.example.com'), /不能包含/);
});

test('escapeHtml 转义特殊字符', () => {
  assert.equal(escapeHtml('<b>&"x"'), '&lt;b&gt;&amp;"x"');
});

test('createLogBuffer 环形淘汰并按数量读取', () => {
  const buffer = createLogBuffer(50);
  for (let index = 0; index < 120; index += 1) buffer.push('info', `line-${index}`);
  assert.equal(buffer.size(), 50);
  const last = buffer.list(10);
  assert.equal(last.length, 10);
  assert.equal(last[9].message, 'line-119');
  assert.equal(last[0].message, 'line-110');
});

test('telegram 服务配置持久化、校验与回滚', async () => {
  const { service } = createServiceForTest();
  const initial = service.publicSettings();
  assert.equal(initial.enabled, false);
  assert.equal(initial.configured, false);
  assert.ok(initial.webhook.secret.length >= 16);

  // 启用但缺 Token：报错且 enabled 回滚，不留下“已启用未配置”状态。
  assert.throws(() => service.updateSettings({ enabled: true }), /Bot Token/);
  assert.equal(service.publicSettings().enabled, false);

  const updated = service.updateSettings({
    mode: 'bot_api',
    bot_token: '123456:ABCDEFGHIJKLMNOPQRSTUV',
    api_base_url: 'https://tg.example.com/',
    chat_id: '123, 456',
    notify: { emby_play: false },
  });
  assert.equal(updated.bot_token_configured, true);
  assert.equal(updated.api_base_url, 'https://tg.example.com');
  assert.equal(updated.chat_id, '123,456');
  assert.equal(updated.notify.emby_play, false);
  assert.equal(updated.notify.organize, true);
  assert.equal(updated.configured, true);

  assert.throws(() => service.updateSettings({ mode: 'xxx' }), /接入模式/);
  assert.throws(() => service.updateSettings({ chat_id: 'abc' }), /Chat ID/);
  assert.throws(() => service.updateSettings({ api_hash: 'zz' }), /API Hash/);
  assert.throws(() => service.updateSettings({ bot_token: 'bad token' }), /Bot Token 格式/);

  // off 清除 Token；mtproto 需要完整凭据。
  const cleared = service.updateSettings({ bot_token: 'off' });
  assert.equal(cleared.bot_token_configured, false);
  service.updateSettings({ mode: 'mtproto' });
  assert.throws(() => service.updateSettings({ enabled: true }), /MTProto/);
  assert.equal(service.publicSettings().enabled, false);

  const secretBefore = service.publicSettings().webhook.secret;
  const regenerated = service.updateSettings({ regenerate_webhook_secret: true });
  assert.notEqual(regenerated.webhook.secret, secretBefore);
  await service.close();
});

test('handleEmbyWebhook 校验密钥、方法并解析事件', async () => {
  const { service, logBuffer } = createServiceForTest();
  const secret = service.publicSettings().webhook.secret;

  const wrongToken = fakeWebhookResponse();
  await service.handleEmbyWebhook(
    fakeWebhookRequest('POST', '{}', 'application/json'),
    wrongToken,
    new URL('http://127.0.0.1/webhooks/emby?token=wrong'),
  );
  assert.equal(wrongToken.code, 403);

  const wrongMethod = fakeWebhookResponse();
  await service.handleEmbyWebhook(
    fakeWebhookRequest('GET', '', ''),
    wrongMethod,
    new URL(`http://127.0.0.1/webhooks/emby?token=${secret}`),
  );
  assert.equal(wrongMethod.code, 405);

  const accepted = fakeWebhookResponse();
  const payload = JSON.stringify({ Event: 'library.new', Item: { Name: '沙丘', Type: 'Movie', ProductionYear: 2021 } });
  await service.handleEmbyWebhook(
    fakeWebhookRequest('POST', payload, 'application/json'),
    accepted,
    new URL(`http://127.0.0.1/webhooks/emby?token=${secret}`),
  );
  assert.equal(accepted.code, 200);
  assert.equal(accepted.payload.handled, true);
  assert.ok(logBuffer.list(10).some((entry) => entry.message.includes('library.new')));

  const unknownEvent = fakeWebhookResponse();
  await service.handleEmbyWebhook(
    fakeWebhookRequest('POST', JSON.stringify({ Event: 'system.serverrestartrequired' }), 'application/json'),
    unknownEvent,
    new URL(`http://127.0.0.1/webhooks/emby?token=${secret}`),
  );
  assert.equal(unknownEvent.code, 200);
  assert.equal(unknownEvent.payload.handled, false);
  await service.close();
});
