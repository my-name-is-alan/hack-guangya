import assert from 'node:assert/strict';
import test from 'node:test';
import { createAuthSessionScopeStore } from './auth-session-scope.mjs';

function persistedStore() {
  let value = null;
  let sequence = 0;
  const create = () => createAuthSessionScopeStore({
    loadValue: () => value,
    saveValue: (next) => { value = next; },
    randomUUID: () => `session-${++sequence}`,
    issuer: 'https://account.guangyapan.com',
  });
  return { create, value: () => value };
}

test('opaque access token refresh retains one persisted session scope', () => {
  const storage = persistedStore();
  const scopeStore = storage.create();
  const originalScope = scopeStore.initialize('opaque-access-1');
  const refreshedScope = scopeStore.retainAfterRefresh('opaque-access-2');
  assert.equal(refreshedScope, originalScope);

  const restarted = storage.create();
  assert.equal(restarted.initialize('opaque-access-2'), originalScope);
});

test('explicit opaque account switch creates a different scope', () => {
  const storage = persistedStore();
  const scopeStore = storage.create();
  const firstScope = scopeStore.establish('opaque-account-a');
  assert.equal(scopeStore.establish('opaque-account-a'), firstScope, 'same explicit credential may reuse its session');
  assert.notEqual(scopeStore.establish('opaque-account-b'), firstScope);
});

test('JWT or profile identity produces a stable account scope across login and refresh', () => {
  const storage = persistedStore();
  const makeToken = (subject, signature) => {
    const header = Buffer.from(JSON.stringify({ alg: 'RS256' })).toString('base64url');
    const payload = Buffer.from(JSON.stringify({ iss: 'https://account.guangyapan.com', sub: subject })).toString('base64url');
    return `${header}.${payload}.${signature}`;
  };
  const first = storage.create();
  const jwtScope = first.establish(makeToken('account-a', 'old'));
  assert.equal(first.establish(makeToken('account-a', 'new')), jwtScope);
  assert.notEqual(first.establish(makeToken('account-b', 'new')), jwtScope);

  const opaque = storage.create();
  const profileScope = opaque.establish('opaque-a', { data: { user: { account_id: 'account-a' } } });
  assert.equal(profileScope, jwtScope);
});
