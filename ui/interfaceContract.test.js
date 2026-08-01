import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const uiRoot = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.dirname(uiRoot);

function sourceFiles(directory, extensions) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const absolute = path.join(directory, entry.name);
    if (entry.isDirectory()) return sourceFiles(absolute, extensions);
    return extensions.includes(path.extname(entry.name)) ? [absolute] : [];
  });
}

function activeUiSources() {
  const router = fs.readFileSync(path.join(uiRoot, 'router', 'index.ts'), 'utf8');
  const views = [...router.matchAll(/import\('\.\.\/views\/([^']+)'\)/g)]
    .map((match) => path.join(uiRoot, 'views', match[1]));
  return [
    path.join(uiRoot, 'RootApp.vue'),
    ...views,
    ...sourceFiles(path.join(uiRoot, 'components'), ['.vue', '.ts', '.js']),
    ...sourceFiles(path.join(uiRoot, 'stores'), ['.ts', '.js']),
  ];
}

function invokedCommands(files) {
  const commands = new Set();
  for (const file of files) {
    for (const line of fs.readFileSync(file, 'utf8').split(/\r?\n/)) {
      if (!line.includes('bridge.invoke')) continue;
      for (const match of line.matchAll(/['"]([a-z][a-z0-9_]+)['"]/g)) commands.add(match[1]);
    }
  }
  return commands;
}

test('活跃 UI 的 bridge 命令在桌面或 Web 端都有明确契约', () => {
  const commands = invokedCommands(activeUiSources());
  const bridgeSource = fs.readFileSync(path.join(uiRoot, 'bridge.js'), 'utf8');
  const rustSource = fs.readFileSync(path.join(repoRoot, 'src-tauri', 'src', 'main.rs'), 'utf8');
  const permissionSource = fs.readFileSync(path.join(repoRoot, 'src-tauri', 'permissions', 'app.toml'), 'utf8');
  const handlerBlock = rustSource.match(/tauri::generate_handler!\[([\s\S]*?)\]\)/)?.[1] || '';
  const webCommands = new Set([...bridgeSource.matchAll(/command === ['"]([a-z][a-z0-9_]+)['"]/g)].map(match => match[1]));
  const rustCommands = new Set([...handlerBlock.matchAll(/\b([a-z][a-z0-9_]+)\b/g)].map(match => match[1]));
  const permittedCommands = new Set([...permissionSource.matchAll(/"([a-z][a-z0-9_]+)"/g)].map(match => match[1]));

  const webOnly = new Set(['get_access_status', 'unlock_access', 'update_access_code']);
  const tauriOnly = new Set([
    'clear_expired_session',
    'get_gcid_import_status',
    'prepare_gcid_import',
    'queue_upload_paths',
    'select_gcid_import_file',
    'stage_gcid_import_text',
    'start_gcid_import',
  ]);

  assert.ok(commands.size > 30, `只发现 ${commands.size} 个活跃命令，入口扫描可能失效`);
  for (const command of commands) {
    assert.ok(webCommands.has(command) || rustCommands.has(command), `${command} 没有任何后端实现`);
    if (!tauriOnly.has(command)) assert.ok(webCommands.has(command), `${command} 缺少 Web bridge 映射`);
    if (!webOnly.has(command)) {
      assert.ok(rustCommands.has(command), `${command} 缺少 Tauri handler`);
      assert.ok(permittedCommands.has(command), `${command} 未加入 Tauri 主窗口权限`);
    }
  }

  assert.match(bridgeSource, /login:\s*\(\)\s*=>\s*webRequest\('\/api\/auth\/device\/start'/);
  assert.ok(rustCommands.has('start_device_login'));
  assert.match(bridgeSource, /tauriInvoke\('refresh_session'\)/);
  assert.ok(rustCommands.has('refresh_session'));
  assert.ok(permittedCommands.has('refresh_session'));
  assert.doesNotMatch(bridgeSource, /command === ['"](?:get_settings|get_cache_stats|clear_cache)['"]/);

  const folderPickerSource = fs.readFileSync(path.join(uiRoot, 'components', 'cloud', 'CloudFolderPicker.vue'), 'utf8');
  assert.match(folderPickerSource, /folders_only:\s*true/);
  assert.match(bridgeSource, /args\.folders_only === true[\s\S]*?params\.set\('resType', '2'\)/);
});
