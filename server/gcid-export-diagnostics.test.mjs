import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import {
  createGcidExportDiagnostics,
  readGcidExportDiagnosticLog,
  sanitizeGcidDiagnosticText,
} from './gcid-export-diagnostics.mjs';

test('GCID diagnostics redact signed URLs and secrets', () => {
  const text = sanitizeGcidDiagnosticText(
    'request https://cdn.example/file.bin?Signature=abc&token=def failed; Authorization: Bearer-secret',
  );
  assert.equal(text.includes('Signature=abc'), false);
  assert.equal(text.includes('token=def'), false);
  assert.equal(text.includes('Bearer-secret'), false);
  assert.match(text, /https:\/\/cdn\.example\/file\.bin\?<redacted>/);
});

test('GCID diagnostics write sendable JSONL with run correlation', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'guangya-gcid-log-'));
  try {
    const filePath = path.join(root, 'logs', 'latest.jsonl');
    const diagnostics = createGcidExportDiagnostics(filePath, 'test');
    diagnostics.write('error', 'range_request_failed', {
      path: 'library/video.mkv',
      error: 'fetch https://cdn.example/video.mkv?token=secret failed',
    });
    const exported = readGcidExportDiagnosticLog(filePath);
    const record = JSON.parse(exported.content.trim());
    assert.equal(record.run_id, diagnostics.runId);
    assert.equal(record.runtime, 'test');
    assert.equal(record.event, 'range_request_failed');
    assert.equal(record.path, 'library/video.mkv');
    assert.equal(record.error.includes('secret'), false);
  }
  finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});
