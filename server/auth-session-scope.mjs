import crypto from 'node:crypto';

const STATE_VERSION = 1;

function digest(value) {
  return crypto.createHash('sha256').update(String(value)).digest('base64url');
}

function idValue(value) {
  if (typeof value === 'string' || typeof value === 'number') return String(value).trim();
  return '';
}

export function accountIdFromAuthPayload(payload) {
  const data = payload?.data ?? payload;
  const profile = data?.user ?? data?.profile ?? data;
  for (const key of ['sub', 'accountId', 'account_id', 'userId', 'user_id', 'uid', 'id']) {
    const value = idValue(profile?.[key]);
    if (value) return value;
  }
  return '';
}

export function jwtAccountIdentity(token, fallbackIssuer = '') {
  const payload = String(token || '').split('.')[1];
  if (!payload) return '';
  try {
    const claims = JSON.parse(Buffer.from(payload, 'base64url').toString('utf8'));
    const accountId = accountIdFromAuthPayload(claims);
    if (!accountId) return '';
    const issuer = String(claims.iss || fallbackIssuer || '').trim();
    return `${issuer}\0${accountId}`;
  } catch {
    return '';
  }
}

function parseRecord(raw) {
  if (!raw) return null;
  try {
    const value = typeof raw === 'string' ? JSON.parse(raw) : raw;
    const scope = String(value?.scope || '').trim();
    const tokenFingerprint = String(value?.tokenFingerprint || '').trim();
    if (value?.version !== STATE_VERSION || !scope || !tokenFingerprint) return null;
    return {
      version: STATE_VERSION,
      scope,
      tokenFingerprint,
      accountIdentity: String(value.accountIdentity || ''),
    };
  } catch {
    return null;
  }
}

export function createAuthSessionScopeStore({
  loadValue,
  saveValue,
  randomUUID = () => crypto.randomUUID(),
  issuer = '',
} = {}) {
  if (typeof loadValue !== 'function' || typeof saveValue !== 'function') {
    throw new TypeError('auth session scope persistence is required');
  }

  let activeScope = 'logged-out';
  let activeRecord = null;
  const tokenFingerprint = (accessToken) => digest(`token\0${String(accessToken || '')}`);
  const accountScope = (identity) => digest(`account\0${identity}`);
  const newSessionScope = () => digest(`session\0${randomUUID()}`);
  const persist = (accessToken, scope, accountIdentity = '') => {
    activeScope = scope;
    activeRecord = {
      version: STATE_VERSION,
      scope,
      tokenFingerprint: tokenFingerprint(accessToken),
      accountIdentity,
    };
    saveValue(JSON.stringify(activeRecord));
    return scope;
  };
  const stableIdentity = (accessToken, profilePayload) => {
    const jwtIdentity = jwtAccountIdentity(accessToken, issuer);
    if (jwtIdentity) return jwtIdentity;
    const profileId = accountIdFromAuthPayload(profilePayload);
    return profileId ? `${issuer}\0${profileId}` : '';
  };

  return {
    initialize(accessToken) {
      if (!String(accessToken || '')) {
        activeScope = 'logged-out';
        activeRecord = null;
        return activeScope;
      }
      const identity = stableIdentity(accessToken);
      if (identity) return persist(accessToken, accountScope(identity), identity);
      const stored = parseRecord(loadValue());
      if (stored?.tokenFingerprint === tokenFingerprint(accessToken)) {
        activeScope = stored.scope;
        activeRecord = stored;
        return activeScope;
      }
      return persist(accessToken, newSessionScope());
    },

    establish(accessToken, profilePayload) {
      if (!String(accessToken || '')) {
        activeScope = 'logged-out';
        activeRecord = null;
        return activeScope;
      }
      const identity = stableIdentity(accessToken, profilePayload);
      if (identity) return persist(accessToken, accountScope(identity), identity);
      const stored = parseRecord(loadValue());
      if (stored?.tokenFingerprint === tokenFingerprint(accessToken)) {
        activeScope = stored.scope;
        activeRecord = stored;
        return activeScope;
      }
      return persist(accessToken, newSessionScope());
    },

    retainAfterRefresh(accessToken) {
      if (!String(accessToken || '')) {
        activeScope = 'logged-out';
        activeRecord = null;
        return activeScope;
      }
      if (activeScope === 'logged-out') return this.initialize(accessToken);
      return persist(accessToken, activeScope, activeRecord?.accountIdentity || '');
    },

    clearCurrent() {
      activeScope = 'logged-out';
      activeRecord = null;
    },

    current() {
      return activeScope;
    },
  };
}
