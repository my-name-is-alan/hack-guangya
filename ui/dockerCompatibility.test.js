import assert from 'node:assert/strict';
import fsp from 'node:fs/promises';
import test from 'node:test';

const [backupSource, bridgeSource, sessionSource, authGateSource, dockerfile, composeSource] = await Promise.all([
  fsp.readFile(new URL('./views/BackupView.vue', import.meta.url), 'utf8'),
  fsp.readFile(new URL('./bridge.js', import.meta.url), 'utf8'),
  fsp.readFile(new URL('./stores/session.ts', import.meta.url), 'utf8'),
  fsp.readFile(new URL('./components/auth/AuthGate.vue', import.meta.url), 'utf8'),
  fsp.readFile(new URL('../Dockerfile', import.meta.url), 'utf8'),
  fsp.readFile(new URL('../docker-compose.yml', import.meta.url), 'utf8'),
]);

test('备份任务通过云端目录 ID 创建，不再要求手填路径', () => {
  assert.match(backupSource, /remote_parent_id:\s*backupForm\.remoteParentId/);
  assert.match(backupSource, /readonly placeholder="选择光鸭云盘目录"/);
  assert.match(backupSource, /title="选择云端目录"/);
  assert.doesNotMatch(backupSource, /v-model:value="backupForm\.remote"\s+placeholder="例如/);
});

test('Docker 备份默认使用更适合挂载卷的轮询监听', () => {
  assert.match(backupSource, /monitor_mode:\s*isTauri\s*\?\s*'native'\s*:\s*'polling'/);
  assert.match(dockerfile, /GUANGYA_DEFAULT_MONITOR_MODE=polling/);
  assert.match(composeSource, /GUANGYA_DEFAULT_MONITOR_MODE:\s*polling/);
  assert.match(dockerfile, /VOLUME\s*\["\/data",\s*"\/watch",\s*"\/archive",\s*"\/media",\s*"\/virtual-library"\]/);
});

test('Docker 不支持的桥接命令明确报错，登录失效有统一通知', () => {
  assert.match(bridgeSource, /Docker Web 端暂不支持命令/);
  assert.match(bridgeSource, /subscribeAuthExpired/);
  assert.match(sessionSource, /handleAuthExpired/);
  assert.match(sessionSource, /text\.includes\('登录态已失效'\)/);
});

test('重新登录会先清理账号级目录缓存再读取新账号', () => {
  assert.match(sessionSource, /function resetAccountScope\(\)[\s\S]*?useFilesStore\(\)\.reset\(\)/);
  assert.match(authGateSource, /session\.resetAccountScope\(\)[\s\S]*?await session\.connect\(\)/);
});
