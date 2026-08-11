import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import {
  organizerCandidates,
  organizerConflictLabel,
  organizerItemActionLabel,
  organizerMatchedTitle,
  organizerMediaLabel,
  normalizeOrganizerCategoryFormRules,
  organizerPreviewItems,
  organizerPreviewTarget,
  organizerStatus,
  organizerTemplateExamples,
  organizerTransferLabel,
  ORGANIZER_MOVIE_TEMPLATE_TOKENS,
  ORGANIZER_TV_TEMPLATE_TOKENS,
  validateOrganizerRuleBlock,
} from './organizer.js';

const organizerViewSource = readFileSync(new URL('./views/OrganizerView.vue', import.meta.url), 'utf8');
const bridgeSource = readFileSync(new URL('./bridge.js', import.meta.url), 'utf8');
const serverOrganizerSource = readFileSync(new URL('../server/organizer.mjs', import.meta.url), 'utf8');

test('native organizer preview summarizes video targets before metadata files', () => {
  const job = {
    preview: {
      metadata: { title: 'A', year: 2026 },
      candidates: [{ tmdb_id: 1 }],
      data: {
        items: [
          { success: true, kind: 'nfo', target: '/media/A/movie.nfo' },
          { success: true, kind: 'video', target: '/media/A/A.mkv' },
          { success: true, kind: 'subtitle', target: '/media/A/A.zh-CN.srt' },
        ],
      },
    },
  };
  assert.equal(organizerPreviewItems(job).length, 3);
  assert.equal(organizerPreviewTarget(job), '/media/A/A.mkv');
  assert.equal(organizerMatchedTitle(job), 'A (2026)');
  assert.equal(organizerCandidates(job).length, 1);
});

test('organizer naming preview accepts rich double-brace variables', () => {
  const examples = organizerTemplateExamples(
    '{{category}}/{{title}}/{{en_title}}.{{videoFormat}}-{{releaseGroup}}{{fileExt}}',
    '{{category}}/{{title}}/{{en_title}}.{{season_episode}}.{{videoCodec}}{{fileExt}}',
    '电影/华语电影',
    '电视剧/动漫/国漫',
  );
  assert.equal(examples.movie.path, '电影/华语电影/示例电影/Example Movie.1080p-Example.mkv');
  assert.equal(examples.tv.path, '电视剧/动漫/国漫/示例剧集/Example Series.S01E02.HEVC.mkv');
  assert.match(organizerViewSource, /v-model:value="settingsForm\.movie_path_template"[\s\S]*?aria-live="polite"[\s\S]*?templateExamples\.movie\.filename/);
  assert.match(organizerViewSource, /v-model:value="settingsForm\.tv_path_template"[\s\S]*?aria-live="polite"[\s\S]*?templateExamples\.tv\.filename/);
  assert.match(organizerViewSource, /insertTemplateToken\('movie', item\.token\)/);
  assert.match(organizerViewSource, /insertTemplateToken\('tv', item\.token\)/);
});

test('manual scrape submission queues recognition instead of awaiting the whole organizer pipeline', () => {
  const start = serverOrganizerSource.indexOf('async function scrapeSelected(input = {})');
  const background = serverOrganizerSource.indexOf('void (async () => {', start);
  const finish = serverOrganizerSource.indexOf('function clearPendingForMapping', background);
  assert.ok(start >= 0 && background > start && finish > background);
  assert.doesNotMatch(serverOrganizerSource.slice(start, background), /\bawait\b/);
  assert.match(serverOrganizerSource.slice(background, finish), /await previewJob[\s\S]*await executeJob/);
  assert.match(serverOrganizerSource.slice(background, finish), /return \{ jobs, failures, state: state\(\) \}/);
});

test('organizer naming exposes the full movie and TV token sets with Chinese countries', () => {
  const examples = organizerTemplateExamples(
    '{category}/{country}/{title}.{ext}',
    '{category}/{country}/{title}.{season_episode}.{ext}',
  );
  assert.match(examples.movie.path, /^电影\/美国\//);
  assert.match(examples.tv.path, /^电视剧\/中国\//);
  assert.ok(ORGANIZER_MOVIE_TEMPLATE_TOKENS.some((item) => item.key === 'media_info'));
  assert.ok(ORGANIZER_TV_TEMPLATE_TOKENS.some((item) => item.key === 'season_year'));
  assert.ok(ORGANIZER_TV_TEMPLATE_TOKENS.length > ORGANIZER_MOVIE_TEMPLATE_TOKENS.length);
  assert.equal(
    organizerTemplateExamples('{category}/{title}{{@if@}}-{{releaseGroup}}{{@endif@}}{{fileExt}}', '{category}/{title}.{ext}').movie.path,
    '电影/示例电影-Example.mkv',
  );
});

test('organizer settings validation normalizes category tags and rejects unsupported rules', () => {
  const [rule] = normalizeOrganizerCategoryFormRules([{
    id: 'category-1',
    name: ' 电视剧\\动漫/国漫 ',
    media_type: 'TV',
    genres: ['16', '动画', '16'],
    original_languages: ['ZH'],
    origin_countries: ['cn', 'HK'],
  }]);
  assert.equal(rule.name, '电视剧/动漫/国漫');
  assert.equal(rule.media_type, 'tv');
  assert.deepEqual(rule.genres, ['16', '动画']);
  assert.deepEqual(rule.original_languages, ['zh']);
  assert.deepEqual(rule.origin_countries, ['CN', 'HK']);
  assert.throws(() => normalizeOrganizerCategoryFormRules([{ name: '空规则', genres: [] }]), /至少配置一个/);
  assert.throws(() => validateOrganizerRuleBlock(String.raw`Show(?=\.S\d+) => Series`, '自定义识别词'), /统一规则语法不支持/);
  assert.equal(
    validateOrganizerRuleBlock(String.raw`(?i)^Alias\.(\d+) => Show.S01E\1@-12`, '自定义识别词'),
    String.raw`(?i)^Alias\.(\d+) => Show.S01E\1@-12`,
  );
});

test('native organizer labels keep backend enum values stable', () => {
  assert.deepEqual(organizerStatus('needs_review'), { label: '需人工确认', color: 'warning' });
  assert.deepEqual(organizerStatus('completed_warning'), { label: '完成有提示', color: 'warning' });
  assert.equal(organizerMediaLabel('tv'), '电视剧');
  assert.equal(organizerTransferLabel('move'), '云盘内移动');
  assert.equal(organizerConflictLabel('rename'), '保留两份');
  assert.equal(organizerItemActionLabel({ success: true, operation: 'generate', action: 'create' }), '生成');
  assert.equal(organizerItemActionLabel({ success: true, operation: 'copy', action: 'skip' }), '跳过');
});

test('organizer default categories persist immediately and history exposes rearchive plus scoped deletion', () => {
  assert.match(organizerViewSource, /恢复默认分类/);
  assert.match(organizerViewSource, /update_organizer_settings[\s\S]*category_rules: categoryRules/);
  assert.match(organizerViewSource, /\['failed', 'completed', 'completed_warning'\][\s\S]*重新归档/);
  assert.match(organizerViewSource, /rearchive_organizer_job/);
  assert.match(organizerViewSource, /重新归档已提交，正在后台处理/);
  assert.match(bridgeSource, /rearchive_organizer_job[\s\S]*\/rearchive/);
  assert.match(organizerViewSource, /@click="openDeleteActions\(record\)"/);
  assert.match(organizerViewSource, /title="操作选项"/);
  for (const label of ['仅删除历史记录', '删除历史记录和源文件', '删除历史记录和媒体库文件', '删除历史记录、源文件和媒体库文件']) {
    assert.match(organizerViewSource, new RegExp(label));
  }
  assert.match(bridgeSource, /remove_organizer_job[\s\S]*method: 'DELETE'[\s\S]*JSON\.stringify\(args\.input/);
});

test('organizer monitors use global output and rules while completed jobs share the final media folder', () => {
  assert.match(organizerViewSource, /输出媒体库（刮削输出）[\s\S]*:options="scrapeTargetOptions"/);
  assert.match(organizerViewSource, /统一沿用全局整理规则/);
  assert.match(organizerViewSource, /二级分类[\s\S]*辅助识别[\s\S]*搜索策略/);
  assert.match(organizerViewSource, /share_organizer_job/);
  assert.match(organizerViewSource, /最终媒体目录分享/);
  assert.match(bridgeSource, /share_organizer_job[\s\S]*\/share/);
  assert.match(serverOrganizerSource, /bindConfiguredOutputTarget[\s\S]*刮削输出/);
  const shareStart = serverOrganizerSource.indexOf('async function shareJob(id)');
  const shareEnd = serverOrganizerSource.indexOf('async function runJob', shareStart);
  assert.ok(shareStart >= 0 && shareEnd > shareStart);
  assert.match(serverOrganizerSource.slice(shareStart, shareEnd), /resolver\.resolve\(relativePath, true\)/);
  assert.doesNotMatch(serverOrganizerSource.slice(shareStart, shareEnd), /ensureDirectory/);
});
