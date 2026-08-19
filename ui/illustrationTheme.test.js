import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('shadcn theme uses Vue-native static styles and readable default controls', async () => {
  const source = await readFile(new URL('./illustrationTheme.ts', import.meta.url), 'utf8');

  assert.match(source, /import \{ createStaticStyles \} from 'antdv-style'/);
  assert.doesNotMatch(source, /\bcreateStyles\b/);
  assert.doesNotMatch(source, /from 'react'/);
  assert.doesNotMatch(source, /\bclsx\b/);
  // 默认控件尺寸必须是 middle（28px）：全局 small 会让整个桌面端过于局促，
  // 高密度场景（表格等）按需显式 size="small"。
  assert.match(source, /getPopupContainer: \(\) => document\.body/);
  assert.match(source, /componentSize: 'middle'/);
  assert.match(source, /controlHeight: 28/);
  assert.match(source, /controlHeightSM: 22/);
  assert.match(source, /controlHeightLG: 34/);
});

test('shadcn theme maps the supplied neutral palette without illustration borders', async () => {
  const source = await readFile(new URL('./illustrationTheme.ts', import.meta.url), 'utf8');

  assert.match(source, /colorPrimary: '#262626'/);
  assert.match(source, /colorSuccess: '#22c55e'/);
  assert.match(source, /colorWarning: '#f97316'/);
  assert.match(source, /colorError: '#ef4444'/);
  assert.match(source, /colorBgLayout: '#fafafa'/);
  assert.match(source, /colorBorder: '#e5e5e5'/);
  assert.match(source, /borderRadiusSM: 6/);
  assert.match(source, /borderRadiusLG: 14/);
  assert.match(source, /lineWidth: 1/);
  assert.doesNotMatch(source, /4px 4px 0/);
  assert.doesNotMatch(source, /textTransform: 'uppercase'/);
});

test('app shell keeps the avatar square and shows used space over total space', async () => {
  const source = await readFile(new URL('./components/shell/AppShell.vue', import.meta.url), 'utf8');

  assert.match(source, /class="account-avatar"/);
  assert.match(source, /flex: 0 0 32px/);
  assert.match(source, /object-fit: cover/);
  assert.match(source, /formatSize\(usedSpace\.value\).*formatSize\(totalSpace\.value\)/s);
  assert.match(source, /class="quota-bar"/);
  assert.match(source, /role="progressbar"/);
  assert.doesNotMatch(source, /\.account-trigger > span/);
});
