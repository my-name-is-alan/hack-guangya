import assert from 'node:assert/strict';
import test from 'node:test';
import {
  autoShareTargetFor,
  DEFAULT_SHARE_TEMPLATE,
  normalizeShareAccess,
  shareFilePayload,
  signHdhiveRequest,
} from './auto-share.mjs';

test('自动分享目标固定为同步根目录下第一层内容', () => {
  assert.deepEqual(autoShareTargetFor('movie.mkv'), {
    key: 'movie.mkv', type: 'file', title: 'movie.mkv', relativePath: 'movie.mkv',
  });
  assert.deepEqual(autoShareTargetFor('tvname/season 1/s01.mkv'), {
    key: 'tvname', type: 'folder', title: 'tvname', relativePath: 'tvname/season 1/s01.mkv',
  });
  assert.equal(autoShareTargetFor('tvname/season 2/s02.mkv').key, 'tvname');
  assert.equal(autoShareTargetFor('tvname/subtitles/season 2/zh/s02.ass').key, 'tvname');
  assert.equal(autoShareTargetFor('movie.mkv', '__manual__'), null);
});

test('Hdhive HMAC 使用固定 canonical request', () => {
  assert.equal(
    signHdhiveRequest('secret', 'post', '/api/integrations/guangya-sync/events', '{"a":1}', '1700000000'),
    'v1=83db0943a113d8cdd5786f9447ebf125c764a64fb935b577f43aae6a2a8c5c5d',
  );
});

test('光鸭分享请求与网页版普通分享参数一致', () => {
  const payload = shareFilePayload(['file-1'], '测试分享');

  assert.deepEqual(payload, {
    fileIds: ['file-1'],
    title: '测试分享',
    validateDuration: 0,
    shareType: 0,
    code: '',
    autoFillCode: false,
    trafficLimit: '0',
    maxRestoreCount: 0,
    downloadType: 1,
    shareTemplate: DEFAULT_SHARE_TEMPLATE,
  });
});

test('光鸭分享请求不会提交空标题', () => {
  assert.equal(shareFilePayload(['file-1'], '   ').title, '云盘分享');
});

test('光鸭分享访问码支持不设置、随机和固定四位码', () => {
  assert.deepEqual(normalizeShareAccess({ share_type: 0 }), {
    shareType: 0, code: '', autoFillCode: false,
  });
  assert.deepEqual(normalizeShareAccess({ share_type: 1 }), {
    shareType: 1, code: '', autoFillCode: false,
  });
  assert.deepEqual(normalizeShareAccess({ share_type: 2, code: 'a1B2' }), {
    shareType: 2, code: 'a1B2', autoFillCode: false,
  });
  assert.throws(
    () => normalizeShareAccess({ share_type: 2, code: 'xyz' }),
    /4 位.*数字/,
  );
});
