import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';
import { validateExport } from '../scripts/import-guangya-gcid.mjs';

const serverRoot = path.dirname(fileURLToPath(import.meta.url));
const source = fs.readFileSync(path.join(serverRoot, '..', 'scripts', 'import-guangya-gcid.mjs'), 'utf8');

test('standalone GCID importer shares the current Windows API protocol', () => {
  assert.match(source, /from '\.\.\/server\/guangya-protocol\.mjs'/);
  assert.match(source, /buildBusinessHeaders\(/);
  assert.match(source, /buildAccountHeaders\(/);
  assert.match(source, /client_secret:\s*guangyaProfile\.clientSecret/);
  assert.match(source, /isAuthExpiredBusinessCode\(code\)/);
  assert.match(source, /\.toUpperCase\(\)/);
  assert.doesNotMatch(source, /aMe-8VSlkrbQXpUR/);
  assert.doesNotMatch(source, /dt:\s*['"]4['"]/);
  assert.match(source, /check_can_flash_upload[\s\S]{0,180}taskId,[\s\S]{0,180}gcid:\s*row\.gcid,[\s\S]{0,180}cid:\s*row\.cid/);
});

test('standalone GCID importer only treats code 147 as cloud indexing pending', () => {
  assert.match(source, /CLOUD_TASK_PENDING_CODES\s*=\s*new Set\(\[147\]\)/);
  assert.match(source, /CLOUD_TASK_INVALID_CODES\s*=\s*new Set\(\[145, 146, 152, 155, 163\]\)/);
  assert.doesNotMatch(source, /TRANSIENT_TASK_CODES/);
});

test('standalone GCID importer normalizes exported hashes to upstream uppercase form', () => {
  const [file] = validateExport({
    source: 'guangya',
    hashType: 'gcid',
    usesGcidInExport: true,
    usesCidInExport: true,
    totalFilesCount: 1,
    files: [{
      path: 'Movies/Film.mkv',
      size: 10,
      gcid: '0123456789abcdef0123456789abcdef01234567',
      cid: '89abcdef0123456789abcdef0123456789abcdef',
    }],
  });
  assert.equal(file.gcid, '0123456789ABCDEF0123456789ABCDEF01234567');
  assert.equal(file.cid, '89ABCDEF0123456789ABCDEF0123456789ABCDEF');
});

test('standalone GCID importer rejects legacy exports without CID', () => {
  assert.throws(() => validateExport({
    source: 'guangya',
    hashType: 'gcid',
    usesGcidInExport: true,
    files: [],
  }), /同时包含 GCID 与 CID/);
});
