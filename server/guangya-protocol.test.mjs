import assert from 'node:assert/strict';
import test from 'node:test';
import {
  WINDOWS_CLIENT_ID,
  buildAccountHeaders,
  buildBusinessHeaders,
  businessResponseCode,
  cloudCollectionResourceType,
  isAuthExpiredBusinessCode,
  isUploadSecurityTokenExpired,
  resolveGuangyaProfile,
  uploadCredentialsExpired,
} from './guangya-protocol.mjs';

test('Windows API profile follows the live Guangya request contract', () => {
  const profile = resolveGuangyaProfile({});
  const headers = buildBusinessHeaders({
    token: 'access-token',
    deviceId: '0123456789abcdef0123456789abcdef',
    profile,
    traceparent: '00-trace-span-01',
  });

  assert.equal(profile.clientId, WINDOWS_CLIENT_ID);
  assert.ok(profile.clientSecret);
  assert.equal(headers.dt, '5');
  assert.equal(headers.av, '1.0.2');
  assert.equal(headers.vc, '1002');
  assert.equal(headers['x-client-id'], WINDOWS_CLIENT_ID);
  assert.equal(headers['x-device-id'], '0123456789abcdef0123456789abcdef');
  assert.equal(headers['user-agent'], 'GuangyapanPC/1.0.2');
  assert.equal(headers.authorization, 'Bearer access-token');

  const accountHeaders = buildAccountHeaders({
    deviceId: '0123456789abcdef0123456789abcdef',
    profile,
    token: 'account-access-token',
  });
  assert.equal(accountHeaders['x-client-id'], WINDOWS_CLIENT_ID);
  assert.equal(accountHeaders['x-device-id'], '0123456789abcdef0123456789abcdef');
  assert.equal(accountHeaders['x-client-version'], '1.0.2');
  assert.equal(accountHeaders['x-sdk-version'], '9.0.2');
  assert.equal(accountHeaders['x-protocol-version'], '301');
  assert.equal(accountHeaders.authorization, 'Bearer account-access-token');
  assert.equal(accountHeaders.dt, undefined);
});

test('business responses require either a zero code or an explicit success message', () => {
  assert.equal(businessResponseCode({ code: 0, msg: 'success' }), 0);
  assert.equal(businessResponseCode({ code: '147', msg: '文件上传中' }), 147);
  assert.equal(businessResponseCode({ msg: 'success', data: {} }), 0);
  assert.equal(businessResponseCode({ msg: 'ok', data: {} }), 0);
  assert.equal(businessResponseCode({ data: {} }), 0);
  assert.equal(businessResponseCode({ code: 0, data: {} }), 0);
  assert.equal(businessResponseCode({ msg: '参数错误', data: {} }), null);
  assert.equal(businessResponseCode({ code: 0, msg: '参数错误', data: {} }), null);
  assert.equal(businessResponseCode({ msg: '参数错误' }), null);
  assert.equal(businessResponseCode({ code: 'not-a-code' }), null);
});

test('all documented authentication expiry codes share one decision', () => {
  assert.equal(isAuthExpiredBusinessCode(110), true);
  assert.equal(isAuthExpiredBusinessCode('117'), true);
  assert.equal(isAuthExpiredBusinessCode(118), true);
  assert.equal(isAuthExpiredBusinessCode(147), false);
});

test('upload credentials follow the official Web expiration recovery contract', () => {
  const now = Date.parse('2026-08-07T05:00:00.000Z');
  assert.equal(uploadCredentialsExpired({ creds: { expiration: '2026-08-07T04:59:59.000Z' } }, now), true);
  assert.equal(uploadCredentialsExpired({ creds: { expiration: '2026-08-07T05:00:01.000Z' } }, now), false);
  assert.equal(uploadCredentialsExpired({ creds: {} }, now), true);
  assert.equal(isUploadSecurityTokenExpired({ code: 'SecurityTokenExpired' }), true);
  assert.equal(isUploadSecurityTokenExpired({ response: { data: { Code: 'SecurityTokenExpired' } } }), true);
  assert.equal(isUploadSecurityTokenExpired(new Error('error sending request for url')), false);
});

test('cloud collection resource types distinguish magnets and ED2K from ordinary links', () => {
  assert.equal(cloudCollectionResourceType('https://example.test/file.iso'), 0);
  assert.equal(cloudCollectionResourceType('magnet:?xt=urn:btih:abc'), 1);
  assert.equal(cloudCollectionResourceType('ed2k://|file|example|1|ABC|/'), 3);
});
