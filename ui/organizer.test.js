import assert from 'node:assert/strict';
import test from 'node:test';
import {
  organizerCandidates,
  organizerConflictLabel,
  organizerItemActionLabel,
  organizerMatchedTitle,
  organizerMediaLabel,
  organizerPreviewItems,
  organizerPreviewTarget,
  organizerStatus,
  organizerTransferLabel,
} from './organizer.js';

test('native organizer preview summarizes video targets before metadata files', () => {
  const job = {
    preview: {
      metadata: { title: 'A', year: 2026 },
      candidates: [{ tmdb_id: 1 }],
      data: {
        items: [
          { success: true, kind: 'nfo', target: '/media/A/movie.nfo' },
          { success: true, kind: 'video', target: '/media/A/A.mkv' },
          { success: true, kind: 'subtitle', target: '/media/A/A.zh-CN.srt' },
        ],
      },
    },
  };
  assert.equal(organizerPreviewItems(job).length, 3);
  assert.equal(organizerPreviewTarget(job), '/media/A/A.mkv');
  assert.equal(organizerMatchedTitle(job), 'A (2026)');
  assert.equal(organizerCandidates(job).length, 1);
});

test('native organizer labels keep backend enum values stable', () => {
  assert.deepEqual(organizerStatus('needs_review'), { label: '需人工确认', color: 'warning' });
  assert.deepEqual(organizerStatus('completed_warning'), { label: '完成有提示', color: 'warning' });
  assert.equal(organizerMediaLabel('tv'), '电视剧');
  assert.equal(organizerTransferLabel('move'), '云盘内移动');
  assert.equal(organizerConflictLabel('rename'), '保留两份');
  assert.equal(organizerItemActionLabel({ success: true, operation: 'generate', action: 'create' }), '生成');
  assert.equal(organizerItemActionLabel({ success: true, operation: 'copy', action: 'skip' }), '跳过');
});
