import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(here, 'server.mjs'), 'utf8');

test('Web cloud confirmation uses structured codes instead of localized message guesses', () => {
  assert.match(source, /CLOUD_TASK_PENDING_CODES\s*=\s*new Set\(\[147\]\)/);
  assert.match(source, /if \(error\?\.retryable !== true\) throw error/);
  assert.match(source, /云端入库成功响应缺少有效的 fileId，已停止轮询/);
  assert.doesNotMatch(source, /isCloudIndexPendingMessage/);
});

test('every Web flash-upload check carries both Guangya hashes', () => {
  const calls = [...source.matchAll(/apiPost\('\/userres\/v1\/check_can_flash_upload',\s*\{([^}]+)\}/g)];
  assert.equal(calls.length, 2);
  for (const [, body] of calls) {
    assert.match(body, /\btaskId\b/);
    assert.match(body, /\bgcid\b/);
    assert.match(body, /\bcid\b/);
  }
});
