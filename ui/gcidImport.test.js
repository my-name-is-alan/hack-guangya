import assert from 'node:assert/strict';
import test from 'node:test';
import {
  GCID_IMPORT_PASTE_FILE_THRESHOLD,
  gcidImportFinished,
  gcidImportPercent,
  shouldConvertPasteToFile,
} from './gcidImport.js';

test('large pasted JSON is converted to a staged file', () => {
  assert.equal(shouldConvertPasteToFile('x'.repeat(GCID_IMPORT_PASTE_FILE_THRESHOLD - 1)), false);
  assert.equal(shouldConvertPasteToFile('你'.repeat(Math.ceil(GCID_IMPORT_PASTE_FILE_THRESHOLD / 3))), true);
});

test('terminal GCID import states contribute to progress', () => {
  const counts = { imported: 80, existing: 5, missed: 2, conflict: 1, failed: 2, processing: 10 };
  assert.equal(gcidImportFinished(counts), 90);
  assert.equal(gcidImportPercent({ total_files: 100, counts }), 90);
});

test('GCID import progress is safe before a job is prepared', () => {
  assert.equal(gcidImportFinished(), 0);
  assert.equal(gcidImportPercent(null), 0);
});
