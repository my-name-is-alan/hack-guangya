import assert from 'node:assert/strict';
import test from 'node:test';
import { parseGuangyaShareLink } from './shareLink.js';

test('解析带提取码和 hash 的光鸭分享链接', () => {
  assert.deepEqual(
    parseGuangyaShareLink('https://www.guangyapan.com/s/1926585463106830337_al8cmYXLP9l33ld2?code=iv5k#/share'),
    {
      shareId: '1926585463106830337_al8cmYXLP9l33ld2',
      code: 'iv5k',
      url: 'https://www.guangyapan.com/s/1926585463106830337_al8cmYXLP9l33ld2?code=iv5k#/share',
    },
  );
});

test('可从一段分享文案中提取链接', () => {
  assert.equal(
    parseGuangyaShareLink('复制链接 https://www.guangyapan.com/s/share_123?code=Ab12 后打开').shareId,
    'share_123',
  );
});

test('拒绝非光鸭链接和缺少 share_id 的链接', () => {
  assert.throws(() => parseGuangyaShareLink('https://example.com/s/share_123'), /只支持/);
  assert.throws(() => parseGuangyaShareLink('https://www.guangyapan.com/'), /share_id/);
});
