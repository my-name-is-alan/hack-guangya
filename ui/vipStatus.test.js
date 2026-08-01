import assert from 'node:assert/strict';
import test from 'node:test';
import { classifyVipStatus } from './vipStatus.js';

test('vipStatus follows the Guangya 1 non-VIP, 2 active, 3 expired contract', () => {
  assert.deepEqual(classifyVipStatus(1), { status: 1, active: false, expired: false, label: '非 VIP', color: 'default' });
  assert.deepEqual(classifyVipStatus('2'), { status: 2, active: true, expired: false, label: 'VIP 有效', color: 'gold' });
  assert.deepEqual(classifyVipStatus(3), { status: 3, active: false, expired: true, label: 'VIP 已过期', color: 'warning' });
  assert.equal(classifyVipStatus(undefined).active, false);
});
