import test from 'node:test';
import assert from 'node:assert/strict';
import { applyRenameRules, buildRenamePreview } from './renameRules.js';

test('链式规则按顺序执行且保留扩展名', () => {
  const result = applyRenameRules(
    { fileName: 'IMG_2026.JPG', resType: 1 },
    [
      { type: 'regex', search: '^IMG_', replacement: 'photo-', ignoreCase: false },
      { type: 'lower' },
      { type: 'sequence', value: '-{n}', start: 7, padding: 3 },
    ],
    0,
    true,
  );
  assert.equal(result, 'photo-2026-007.JPG');
});

test('批量预览会拒绝重复目标名称', () => {
  const preview = buildRenamePreview(
    [{ fileId: '1', fileName: 'a.txt' }, { fileId: '2', fileName: 'b.txt' }],
    [{ type: 'set', value: 'same' }],
    true,
  );
  assert.match(preview.error, /重复/);
});
