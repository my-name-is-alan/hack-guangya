import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const read = (relative) => readFile(new URL(relative, import.meta.url), 'utf8');

test('developer transfer is wired through the active UI and both runtime bridges', async () => {
  const [settingsView, accountPanel, panel, selectionBar, cloud, bridge, server, rust] = await Promise.all([
    read('./views/SystemSettingsView.vue'),
    read('./components/settings/AccountSettingsPanel.vue'),
    read('./components/settings/DeveloperSettingsPanel.vue'),
    read('./components/files/FileSelectionBar.vue'),
    read('./views/CloudView.vue'),
    read('./bridge.js'),
    read('../server/server.mjs'),
    read('../src-tauri/src/main.rs'),
  ]);

  assert.match(settingsView, /AccountSettingsPanel/);
  assert.match(settingsView, /key="developerTransfer"/);
  assert.match(settingsView, /<DeveloperSettingsPanel\s*\/>/);
  assert.doesNotMatch(accountPanel, /DeveloperSettingsPanel/);
  assert.match(panel, /key="tokens"/);
  assert.match(panel, /key="jobs"/);
  assert.match(panel, /Token 配置/);
  assert.match(panel, /任务记录/);
  assert.match(settingsView, /\.settings-tabs > :deep\(\.ant-tabs-nav\)/);
  assert.doesNotMatch(settingsView, /\.settings-tabs :deep\(\.ant-tabs-nav\)/);
  assert.doesNotMatch(accountPanel, /vipRights|get_global_config|当前权益规则/);
  assert.match(panel, /update_developer_credentials/);
  assert.match(panel, /update_developer_mode/);
  assert.match(panel, /验证当前账号/);
  assert.match(panel, /probe_file_id/);
  assert.match(panel, /token_masked/);
  assert.match(panel, /一个 TOKEN 只对应/);
  assert.match(panel, /targetListFromPayload/);
  assert.match(panel, /jobListFromPayload/);
  assert.match(selectionBar, /transferAccount/);
  assert.match(cloud, /start_developer_transfer/);
  assert.match(cloud, /normalizeDeveloperSettings/);
  assert.match(cloud, /一次最多互传 20 项/);

  for (const command of [
    'get_developer_settings',
    'update_developer_credentials',
    'test_developer_credentials',
    'update_developer_mode',
    'upsert_developer_target',
    'delete_developer_target',
    'list_developer_transfers',
    'start_developer_transfer',
  ]) {
    assert.match(bridge, new RegExp(`command === '${command}'`));
    assert.match(rust, new RegExp(`\\b${command}\\b`));
  }

  assert.match(server, /\/api\/developer\/transfers/);
  assert.match(server, /\/api\/developer\/mode/);
  assert.match(server, /apiFileReadWithDeveloperFallback/);
  assert.match(server, /verifyDeveloperAccountOwnership/);
  assert.match(server, /developer_verified_client_id/);
  assert.match(server, /resumeDeveloperTransfers\(\)/);
  assert.match(rust, /resume_developer_transfer_jobs/);
  assert.match(server, /token_masked/);
  assert.match(rust, /developer_file_read_fallback/);
  assert.match(rust, /verify_developer_account_ownership/);
  assert.match(rust, /developer_verified_client_id/);
  assert.doesNotMatch(panel, /settings\.client_secret\b/);
});

test('direct upload falls back to pre-audit only for business code 18011', async () => {
  const [server, rust] = await Promise.all([
    read('../server/server.mjs'),
    read('../src-tauri/src/main.rs'),
  ]);

  assert.match(server, /apiCode !== 18011/);
  assert.match(server, /\/developer\/v1\/pre_upload/);
  assert.match(server, /\/developer\/v1\/pre_upload_status/);
  assert.match(server, /\/developer\/v1\/upload_status/);
  assert.match(rust, /error\.code == Some\(18011\)/);
  assert.match(rust, /"\/developer\/v1\/pre_upload"/);
  assert.match(rust, /"\/developer\/v1\/upload_status"/);
});
