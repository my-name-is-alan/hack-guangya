import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const uiRoot = path.dirname(fileURLToPath(import.meta.url));
const read = (...segments) => fs.readFileSync(path.join(uiRoot, ...segments), 'utf8');

test('文件工作区接入新建、详情、最近记录和回收站闭环', () => {
  const router = read('router', 'index.ts');
  const workspace = read('views', 'FilesWorkspaceView.vue');
  const cloud = read('views', 'CloudView.vue');
  const recycle = read('components', 'files', 'RecycleBinPanel.vue');
  const recent = read('components', 'files', 'RecentFilesPanel.vue');

  assert.match(router, /FilesWorkspaceView\.vue/);
  assert.match(workspace, /key="recent"/);
  assert.match(workspace, /key="recycle"/);
  assert.match(cloud, /bridge\.invoke\('create_folder'/);
  assert.match(cloud, /bridge\.invoke\('delete_files'/);
  assert.match(cloud, /已移入回收站/);
  assert.match(recycle, /bridge\.invoke\(command/);
  assert.match(recycle, /'restore_files'/);
  assert.match(recycle, /'permanently_delete_files'/);
  assert.match(recycle, /'clear_recycle_bin'/);
  assert.match(recycle, /无法恢复/);
  assert.match(recent, /'list_recent_actions'/);
  assert.match(recent, /nextCursor/);
  assert.match(recent, /FileDetailsDrawer/);
});

test('云添加使用官方 cursor、资源预解析、子文件序号和任务生命周期命令', () => {
  const offline = read('views', 'OfflineView.vue');
  const bridge = read('bridge.js');
  const settings = read('views', 'SystemSettingsView.vue');
  const offlineSettings = read('components', 'settings', 'OfflineSettingsPanel.vue');

  assert.match(offline, /'resolve_offline_resource'/);
  assert.match(offline, /resolvedUrl/);
  assert.match(offline, /file_indexes:\s*fileIndexes/);
  assert.match(offline, /'get_offline_statistics'/);
  assert.match(offline, /'cancel_offline_tasks'/);
  assert.match(offline, /'delete_offline_tasks'/);
  assert.match(offline, /'retry_offline_tasks'/);
  assert.match(offline, /hasMore/);
  assert.match(offline, /nextCursor/);
  assert.match(offline, /restore_name:\s*restoreName/);
  assert.match(offline, /protectedSubmission \? \{\} : unwrapData\(await resolveOfflineResource/);
  assert.match(offline, /保护模式跳过云端预解析，默认保存全部文件/);
  assert.match(offline, /nameRestoreStatus/);
  assert.match(settings, /OfflineSettingsPanel/);
  assert.match(offlineSettings, /filename_obfuscation_enabled/);
  assert.match(bridge, /command === 'get_offline_settings'/);
  assert.match(bridge, /command === 'update_offline_settings'/);
  assert.doesNotMatch(offline, /list_offline_tasks',\s*\{\s*page:/);
  assert.match(bridge, /command === 'list_offline_tasks'[\s\S]*?cursor:/);
  assert.doesNotMatch(bridge.match(/command === 'list_offline_tasks'[\s\S]*?return webRequest\(`\/api\/offline\?\$\{params\}`\);/)?.[0] || '', /page:/);
});

test('账号开发者模式、分享编辑、失效清理和文件直链均有活跃 UI 消费者', () => {
  const account = read('components', 'settings', 'AccountSettingsPanel.vue');
  const settings = read('views', 'SystemSettingsView.vue');
  const shares = read('views', 'SharesView.vue');
  const details = read('components', 'files', 'FileDetailsDrawer.vue');

  assert.match(settings, /DeveloperSettingsPanel/);
  assert.match(settings, /key="developerTransfer"/);
  assert.match(account, /session\.loadOverview\(\)/);
  assert.doesNotMatch(account, /get_global_config|vipRights|当前权益规则/);
  assert.match(shares, /bridge\.invoke\('update_share'/);
  assert.match(shares, /bridge\.invoke\('delete_invalid_shares'/);
  assert.match(shares, /trafficLimit = editForm\.downloadType === 0/);
  assert.match(shares, /批量取消/);
  assert.match(details, /bridge\.invoke\('set_direct_link'/);
  assert.match(details, /bridge\.invoke\('unset_direct_link'/);
  assert.match(details, /bridge\.invoke\('get_direct_link'/);
  assert.match(details, /short_link: shortLink/);
  assert.match(details, /const canToggleDirectLink = computed/);
  assert.match(details, /Number\(pick\(details\.value, \['depth'\], 0\)\) === 1/);
  assert.match(details, /const canGetDirectLink = computed/);
  assert.match(details, /直链文件夹已开启/);
  const enableDirectLink = details.match(/async function enableDirectLink\(\)[\s\S]*?\n}\n\nfunction disableDirectLink/)?.[0] || '';
  assert.doesNotMatch(enableDirectLink, /getDirectLink\(/);
});
