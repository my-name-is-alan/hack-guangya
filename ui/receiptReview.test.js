import test from 'node:test';
import assert from 'node:assert/strict';
import { needsTmdbReview } from './receiptReview.js';

test('HDHive 要求人工匹配媒体时显示 TMDB 输入入口', () => {
  assert.equal(needsTmdbReview({
    status: 'needs_review',
    error_code: 'tmdb_required',
    message: '无法从分享根目录识别影视标题',
  }), true);
});

test('非 TMDB 类型的人工复核不显示 TMDB 输入入口', () => {
  assert.equal(needsTmdbReview({
    status: 'needs_review',
    error_code: 'owner_not_configured',
    message: '投稿账号未配置',
  }), false);
});

test('旧回执没有错误码时兼容 TMDB 和标题识别失败文案', () => {
  assert.equal(needsTmdbReview({
    status: 'needs_review',
    message: '无法可靠匹配 TMDB，请人工补充',
  }), true);
  assert.equal(needsTmdbReview({
    status: 'needs_review',
    message: '无法从分享根目录识别影视标题',
  }), true);
});

test('非人工复核状态不显示 TMDB 输入入口', () => {
  assert.equal(needsTmdbReview({
    status: 'failed',
    error_code: 'tmdb_required',
  }), false);
});
