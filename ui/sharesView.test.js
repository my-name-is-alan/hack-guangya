import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';
import { shareDisplayName } from './shareRecord.js';

const sharesViewSource = readFile(new URL('./views/SharesView.vue', import.meta.url), 'utf8');

function sourceBetween(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start + startMarker.length);
  assert.notEqual(start, -1, `missing source marker: ${startMarker}`);
  assert.notEqual(end, -1, `missing source marker: ${endMarker}`);
  return source.slice(start, end);
}

test('SharesView does not expose the unreachable legacy share form', async () => {
  const source = await sharesViewSource;

  assert.doesNotMatch(source, /\bshareCreate\b/);
  assert.doesNotMatch(source, /\b(?:openShareCreate|submitShareCreate|shareExpireText)\b/);
  assert.doesNotMatch(source, /bridge\.invoke\(['"]create_share['"]/);
  assert.doesNotMatch(source, /v-model:value=['"]shareCreate\.(?:period|password)['"]/);
});

test('shareDisplayName resolves supported backend name variants before generic resource names', () => {
  assert.equal(shareDisplayName({ title: '项目资料' }), '项目资料');
  assert.equal(shareDisplayName({ shareTitle: '产品原型' }), '产品原型');
  assert.equal(shareDisplayName({ share_name: '接口文档' }), '接口文档');
  assert.equal(shareDisplayName({ shareName: '发布包' }), '发布包');
  assert.equal(shareDisplayName({ name: '设计素材' }), '设计素材');
  assert.equal(shareDisplayName({ file_name: '归档文件.zip' }), '归档文件.zip');
});

test('shareDisplayName reads nested share details and uses a concrete id only when names are absent', () => {
  assert.equal(shareDisplayName({ title: ' ', shareInfo: { title: '嵌套标题' }, id: 7 }), '嵌套标题');
  assert.equal(shareDisplayName({ resource: { fileName: '视频目录' }, shareId: 'share-8' }), '视频目录');
  assert.equal(shareDisplayName({ share_id: 'share-9' }), '分享 share-9');
  assert.equal(shareDisplayName({}), '未命名分享');
});

test('SharesView uses the shared display-name resolver in the list', async () => {
  const source = await sharesViewSource;

  assert.match(source, /import \{ shareDisplayName \} from ['"]\.\.\/shareRecord\.js['"]/);
  assert.match(source, /\{\{\s*shareDisplayName\(record\)\s*\}\}/);
  assert.match(source, /确定取消「\$\{shareDisplayName\(record\)\}」吗？/);
});

test('SharesView copies the URL together with direct or URL-derived extraction codes', async () => {
  const source = await sharesViewSource;
  const copySource = sourceBetween(source, 'async function copyCloudShare(record)', 'async function deleteShare(record)');

  assert.match(source, /import \{ parseGuangyaShareLink \} from ['"]\.\.\/shareLink\.js['"]/);
  assert.match(source, /import \{[^}]*\bcopyText\b[^}]*\} from ['"]\.\.\/formatters\.js['"]/);
  assert.match(copySource, /pick\(record,\s*\[['"]shareUrl['"],\s*['"]share_url['"],\s*['"]url['"]\]/);
  assert.match(copySource, /pick\(record,\s*\[['"]code['"],\s*['"]extractCode['"]\]/);
  assert.match(copySource, /parseGuangyaShareLink\(url\)\.code/);
  assert.match(copySource, /copyText\(code\s*\?\s*`\$\{url\} 提取码：\$\{code\}`\s*:\s*url,\s*message\)/);
  assert.match(source, /@click=['"]copyCloudShare\(record\)['"]/);
});
