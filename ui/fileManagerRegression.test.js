import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const cloudViewSource = readFile(new URL('./views/CloudView.vue', import.meta.url), 'utf8');
const fileSelectionBarSource = readFile(new URL('./components/files/FileSelectionBar.vue', import.meta.url), 'utf8');
const gcidImportStatusSource = readFile(new URL('./components/files/GcidImportStatus.vue', import.meta.url), 'utf8');
const renameRulesSource = readFile(new URL('./renameRules.js', import.meta.url), 'utf8');
const stylesSource = readFile(new URL('./styles.css', import.meta.url), 'utf8');
const transfersSource = readFile(new URL('./stores/transfers.ts', import.meta.url), 'utf8');

function sourceBetween(source, startMarker, endMarker) {
  const start = source.indexOf(startMarker);
  const end = source.indexOf(endMarker, start + startMarker.length);
  assert.notEqual(start, -1, `missing source marker: ${startMarker}`);
  assert.notEqual(end, -1, `missing source marker: ${endMarker}`);
  return source.slice(start, end);
}

function cssRuleBody(source, selector) {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = source.match(new RegExp(`${escapedSelector}\\s*\\{([^}]*)\\}`));
  assert.ok(match, `missing CSS rule: ${selector}`);
  return match[1];
}

test('CloudView composes the file selection action bar', async () => {
  const [source, selectionSource, styles] = await Promise.all([cloudViewSource, fileSelectionBarSource, stylesSource]);

  assert.match(source, /import FileSelectionBar from ['"]\.\.\/components\/files\/FileSelectionBar\.vue['"]/);
  assert.equal(source.match(/<FileSelectionBar\b/g)?.length, 1);
  assert.match(source, /<div class="file-toolbar"[^>]*>[\s\S]*?<FileSelectionBar[\s\S]*?<div class="file-list-region"/);
  assert.match(source, /v-if="fileActionBarVisible"/);
  assert.match(selectionSource, /<slot name="status"\s*\/>/);
  assert.match(cssRuleBody(styles, '.file-toolbar'), /min-height:\s*42px\s*;/);
});

test('CloudView creates shares with the backend contract and opens an explicit result dialog', async () => {
  const source = await cloudViewSource;
  const createShareSource = sourceBetween(
    source,
    'async function createCloudShare(records)',
    'function shareUrlForSave',
  );

  assert.match(source, /import ShareResultDialog from ['"]\.\.\/components\/shares\/ShareResultDialog\.vue['"]/);
  assert.match(source, /<ShareResultDialog\b/);
  assert.match(createShareSource, /file_ids:\s*targets\.map\(fileId\)\.filter\(Boolean\)/);
  assert.match(createShareSource, /\btitle\s*,/);
  assert.match(createShareSource, /target_type:\s*targetType/);
  assert.match(createShareSource, /share_type:\s*shareType/);
  assert.match(createShareSource, /code:\s*shareType === 2 \? code : ['"]['"]/);
  assert.match(createShareSource, /auto_fill_code:\s*false/);
  assert.match(source, /value="none">不设置/);
  assert.match(source, /value="random">随机/);
  assert.match(source, /value="fixed">固定/);
  assert.doesNotMatch(createShareSource, /\b(?:period|password)\s*:/);
  assert.doesNotMatch(createShareSource, /navigator\.clipboard\.writeText/);
});

test('row keyboard navigation preserves Alt+Up and keyboard context-menu focus', async () => {
  const source = await cloudViewSource;

  assert.match(source, /event\.currentTarget\?\.focus\?\.\(\{ preventScroll: true \}\)/);
  assert.match(source, /if \(fileContextMenu\.open\) \{[\s\S]*?handleFileContextMenuKeydown\(event\)/);
  assert.match(source, /event\.key === ['"]ArrowUp['"] && !event\.altKey/);
  assert.match(source, /openFileContextMenu\([\s\S]*?record,\s*true\)/);
  assert.match(source, /:auto-focus="fileContextMenu\.keyboard"/);
  assert.match(source, /focusKeyboardContextMenu\(\)/);
  assert.match(source, /function handleFileContextMenuKeydown\(event\)/);
  assert.match(source, /\['ArrowDown', 'ArrowUp', 'Home', 'End'\]\.includes\(event\.key\)/);
  assert.match(source, /addEventListener\(['"]keydown['"],\s*handleFileContextMenuKeydown,\s*true\)/);
  assert.match(source, /blocked:\s*fileContextMenu\.open \|\|/);
  assert.match(source, /\[role="menuitem"\]:not\(\[aria-disabled="true"\]\)/);
  assert.match(source, /closeFileContextMenu\(true\)/);
});

test('cancelled desktop folder selection does not report a queued download', async () => {
  const [cloudSource, transferStoreSource] = await Promise.all([cloudViewSource, transfersSource]);
  const downloadSource = sourceBetween(
    cloudSource,
    'async function downloadCloudFiles(records)',
    'function handleUploadMenuClick',
  );

  assert.match(transferStoreSource, /typeof selected !== ['"]string['"] \|\| !selected\) return false/);
  assert.match(downloadSource, /const queued = await transfers\.downloadRecords\(targets\)/);
  assert.match(downloadSource, /if \(isTauri && queued\) message\.success\(['"]已加入下载队列['"]\)/);
});

test('rename request keeps file identifiers and names in camelCase', async () => {
  const [cloudSource, rulesSource] = await Promise.all([cloudViewSource, renameRulesSource]);
  const submitRenameSource = sourceBetween(
    cloudSource,
    'async function submitRename()',
    'async function openFolderPicker',
  );

  assert.match(submitRenameSource, /item\.fileId && item\.newName !== item\.currentName/);
  assert.match(submitRenameSource, /bridge\.invoke\(['"]batch_rename_files['"],\s*\{\s*renames\s*\}\)/);
  assert.doesNotMatch(submitRenameSource, /\b(?:file_id|current_name|new_name)\b/);
  assert.match(
    rulesSource,
    /fileId:\s*String\([\s\S]*?currentName:\s*String\([\s\S]*?newName:\s*applyRenameRules\(/,
  );
});

test('GCID import uses the staged command chain instead of the removed shortcut command', async () => {
  const source = await cloudViewSource;
  const commands = [
    'select_gcid_import_file',
    'stage_gcid_import_text',
    'prepare_gcid_import',
    'get_gcid_import_status',
    'start_gcid_import',
  ];

  assert.doesNotMatch(source, /\bimport_gcid_json\b/);
  for (const command of commands) {
    assert.match(source, new RegExp(`bridge\\.invoke\\(['"]${command}['"]`));
  }
});

test('GCID import keeps creation compact and moves running progress into a detail surface', async () => {
  const [source, statusSource] = await Promise.all([cloudViewSource, gcidImportStatusSource]);
  const importModal = sourceBetween(source, 'title="导入 GCID JSON"', 'title="选择服务器文件或文件夹"');
  const submitSource = sourceBetween(source, 'async function submitGcidImport()', 'async function readDirectoryEntry');

  assert.match(source, /import GcidImportStatus from ['"]\.\.\/components\/files\/GcidImportStatus\.vue['"]/);
  assert.match(source, /void resumeGcidImport\(\)/);
  assert.match(submitSource, /gcidImport\.open = false/);
  assert.match(importModal, /:width="520"/);
  assert.match(importModal, /:rows="4"/);
  assert.doesNotMatch(importModal, /<a-progress|gcid-import-counts|gcid-import-status/);
  assert.match(statusSource, /class="gcid-task-trigger"/);
  assert.match(statusSource, /<a-drawer[^>]*title="GCID 导入详情"/);
});

test('CloudView applies compact breadcrumbs and the saved folder-open preference', async () => {
  const source = await cloudViewSource;

  assert.match(source, /import CompactFileBreadcrumb from ['"]\.\.\/components\/files\/CompactFileBreadcrumb\.vue['"]/);
  assert.match(source, /<CompactFileBreadcrumb :segments="currentPath" @navigate="jumpToPath\(\$event\.index\)"\s*\/>/);
  assert.match(source, /useFolderOpenPreference\(\)/);
  assert.match(source, /folderOpenMode\.value === FOLDER_OPEN_MODE\.SINGLE_CLICK/);
  assert.match(source, /folderOpenMode\.value === FOLDER_OPEN_MODE\.DOUBLE_CLICK/);
});

test('web upload menu remains connected to the server file picker endpoints', async () => {
  const source = await cloudViewSource;

  assert.match(
    source,
    /!isTauri\s*\?\s*\[[\s\S]*?key:\s*['"]server['"][\s\S]*?label:\s*['"]选择服务器文件['"]/,
  );
  assert.match(source, /fetch\(`\/api\/server-files\?\$\{query\}`\)/);
  assert.match(source, /fetch\(['"]\/api\/server-upload['"],\s*\{[\s\S]*?method:\s*['"]POST['"]/);
  assert.match(source, /v-if="!isTauri"[\s\S]*?title="选择服务器文件或文件夹"/);
});

test('file table stays content-sized and uses a six-pixel scrollbar', async () => {
  const source = await stylesSource;
  const fileCardRule = cssRuleBody(source, '.cloud-view .file-card');
  const tableBodyRule = cssRuleBody(source, '.file-card .ant-table-body');
  const scrollbarRule = source.match(
    /\.file-card \.ant-table-body::-webkit-scrollbar\s*,\s*\.route-content::-webkit-scrollbar\s*\{([^}]*)\}/,
  );

  assert.match(fileCardRule, /min-height:\s*0\s*;/);
  assert.match(fileCardRule, /flex:\s*0\s+0\s+auto\s*;/);
  assert.doesNotMatch(fileCardRule, /min-height:\s*420px\s*;/);
  assert.doesNotMatch(fileCardRule, /flex:\s*1(?:\s|;)/);
  assert.match(tableBodyRule, /overflow-y:\s*auto\s*!important\s*;/);
  assert.ok(scrollbarRule, 'missing file table scrollbar rule');
  assert.match(scrollbarRule[1], /width:\s*6px\s*;/);
  assert.match(scrollbarRule[1], /height:\s*6px\s*;/);
});
