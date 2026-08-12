import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readRustBackendSource } from './rustBackendSource.js';
import test from 'node:test';
import {
  developerTransferIsActive,
  developerTransferPercent,
  developerTransferStageLabel,
  normalizeDeveloperTransferJob,
} from './developerTransfer.js';

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
    readRustBackendSource(),
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
    'export_gcid_json',
  ]) {
    assert.match(bridge, new RegExp(`command === '${command}'`));
    assert.match(rust, new RegExp(`\\b${command}\\b`));
  }

  assert.match(server, /\/api\/developer\/transfers/);
  assert.match(server, /\/api\/developer\/mode/);
  assert.match(server, /\/api\/files\/export-gcid/);
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
    readRustBackendSource(),
  ]);

  assert.match(server, /apiCode !== 18011/);
  assert.match(server, /\/developer\/v1\/pre_upload/);
  assert.match(server, /\/developer\/v1\/pre_upload_status/);
  assert.match(server, /\/developer\/v1\/upload_status/);
  assert.match(rust, /error\.code == Some\(18011\)/);
  assert.match(rust, /"\/developer\/v1\/pre_upload"/);
  assert.match(rust, /"\/developer\/v1\/upload_status"/);
});

test('pre-audit submits original file ids without changing source names', async () => {
  const [server, rust, cloud, selectionBar] = await Promise.all([
    read('../server/server.mjs'),
    readRustBackendSource(),
    read('./views/CloudView.vue'),
    read('./components/files/FileSelectionBar.vue'),
  ]);

  const serverBranchStart = server.indexOf('if (!(error instanceof DeveloperApiError) || error.apiCode !== 18011)');
  const rustBranchStart = rust.indexOf('Err(error) if error.code == Some(18011)');
  const serverBranch = server.slice(serverBranchStart, server.indexOf('await finishDeveloperPreAudit', serverBranchStart));
  const rustBranch = rust.slice(rustBranchStart, rust.indexOf('finish_developer_pre_audit(', rustBranchStart));
  assert.ok(serverBranchStart >= 0);
  assert.ok(rustBranchStart >= 0);
  assert.match(serverBranch, /startDeveloperPreAudit/);
  assert.match(rustBranch, /start_developer_pre_audit/);
  assert.doesNotMatch(serverBranch, /acquireDeveloperNameObfuscation|renameDeveloperNameWithRetry/);
  assert.doesNotMatch(rustBranch, /acquire_developer_name_obfuscation|rename_developer_name_with_retry/);
  assert.match(server, /chunkDeveloperPreAuditFileIds/);
  assert.match(server, /collectCloudSelectionEntries\(job\.file_ids, job\.file_names, false\)/);
  assert.match(rust, /collect_cloud_selection_entries\(/);
  assert.match(rust, /DEVELOPER_PRE_AUDIT_BATCH_SIZE/);
  assert.match(server, /return await submitDeveloperUpload\(client, completed, targetToken\)/);
  assert.match(rust, /return match submit_developer_upload\(/);
  assert.match(server, /error\.apiCode === 18014/);
  assert.match(rust, /error\.code == Some\(18014\)/);
  assert.match(server, /预审完成：\$\{completed\.rejected_count\} 个文件均未通过，未开始秒传/);
  assert.match(rust, /预审完成：\{\} 个文件均未通过，未开始秒传/);
  assert.match(cloud, /小号流程不会修改源文件名/);
  assert.match(selectionBar, /exportGcid/);
});

test('selected cloud files and folders export GCID plus sampled CID with visible progress', async () => {
  const [server, retry, rust, cloud, exportStatus, bridge] = await Promise.all([
    read('../server/server.mjs'),
    read('../server/gcid-export-retry.mjs'),
    readRustBackendSource(),
    read('./views/CloudView.vue'),
    read('./components/files/GcidExportStatus.vue'),
    read('./bridge.js'),
  ]);

  assert.match(cloud, /头、中、尾各 20 KB/);
  assert.match(cloud, /最多同时处理 20 个文件/);
  assert.match(cloud, /只会错峰重试该分段，最多 3 次/);
  assert.match(cloud, /绝不会下载整文件做完整验证/);
  assert.match(exportStatus, /单分段失败会独立错峰重试（最多 3 次）/);
  assert.match(cloud, /GcidExportStatus/);
  assert.match(cloud, /DeveloperTransferStatus/);
  assert.match(cloud, /export_gcid_json/);
  assert.match(cloud, /onOk\(\)\s*\{\s*void runGcidExport\(targets\)/);
  assert.match(cloud, /keepFailureVisible/);
  assert.match(cloud, /export_gcid_diagnostic_log/);
  assert.match(cloud, /status: 'warning'/);
  assert.match(exportStatus, /导出诊断日志/);
  assert.match(exportStatus, /签名地址已脱敏/);
  assert.match(server, /calculateGuangyaCidSamples/);
  assert.match(server, /mapConcurrent\(files, GCID_EXPORT_FILE_CONCURRENCY/);
  assert.match(server, /retryGcidExportRange\(async \(attempt\)/);
  assert.doesNotMatch(server, /sample_mode_failed_falling_back/);
  assert.doesNotMatch(server, /fullHash/);
  assert.doesNotMatch(server, /GCID_EXPORT_FULL_ATTEMPTS/);
  assert.match(server, /createGcidExportRangeGate/);
  assert.match(retry, /GCID_EXPORT_FILE_CONCURRENCY = 20/);
  assert.match(retry, /GCID_EXPORT_SCAN_CONCURRENCY = 24/);
  assert.match(retry, /GCID_EXPORT_RANGE_ATTEMPTS = 3/);
  assert.match(retry, /GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY = 24/);
  assert.match(server, /rangeHashWithRetry/);
  assert.match(server, /skippedFiles/);
  assert.match(server, /content-range/);
  assert.match(server, /guangya-gcid-export-2\.0/);
  assert.match(server, /entry\.path\.slice\(rootPrefix\.length\)/);
  assert.match(rust, /FlashHashAccumulator/);
  assert.match(rust, /GCID_EXPORT_FILE_CONCURRENCY: usize = 20/);
  assert.match(rust, /GCID_EXPORT_SCAN_CONCURRENCY: usize = 24/);
  assert.match(rust, /GCID_EXPORT_RANGE_ATTEMPTS: usize = 3/);
  assert.match(rust, /GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY: usize = 24/);
  assert.match(rust, /sample_cloud_selection_cid_with_retry/);
  assert.doesNotMatch(rust, /sample_mode_failed_falling_back/);
  assert.doesNotMatch(rust, /hash_cloud_selection_entry_full/);
  assert.doesNotMatch(rust, /GCID_EXPORT_FULL_ATTEMPTS/);
  assert.match(rust, /Semaphore::new\(GCID_EXPORT_GLOBAL_RANGE_CONCURRENCY\)/);
  assert.match(rust, /skipped_files/);
  assert.match(rust, /read_cloud_cid_range_with_retry/);
  assert.match(rust, /guangya-gcid-export-2\.0/);
  assert.match(server, /gcid_export_snapshots/);
  assert.match(server, /snapshot_cache_hit/);
  assert.match(rust, /gcid_export_snapshots/);
  assert.match(rust, /snapshot_cache_hit/);
  assert.match(bridge, /command === 'export_gcid_json'/);
  assert.match(bridge, /command === 'export_gcid_diagnostic_log'/);
  assert.match(server, /\/api\/files\/export-gcid-log/);
  assert.match(rust, /\bexport_gcid_diagnostic_log\b/);
});

test('developer transfer progress normalizes persisted and live job payloads', () => {
  const job = normalizeDeveloperTransferJob({
    id: 'job-1',
    targetName: '小号 A',
    status: 'auditing',
    phase: 'obfuscating',
    totalCount: 3,
    workTotalCount: 88,
    processedCount: 81,
    currentPath: '目录/当前文件.mkv',
  });
  assert.equal(developerTransferIsActive(job), true);
  assert.equal(developerTransferPercent(job), 92);
  assert.equal(developerTransferStageLabel(job), '处理源文件名');
  assert.equal(job.current_path, '目录/当前文件.mkv');
  assert.equal(developerTransferIsActive({ ...job, status: 'failed', phase: 'restoring' }), true);
});
