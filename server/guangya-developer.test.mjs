import assert from 'node:assert/strict';
import test from 'node:test';
import {
  buildDeveloperHeaders,
  buildDeveloperSignature,
  createGuangyaDeveloperClient,
  DeveloperApiError,
} from './guangya-developer.mjs';

const vector = {
  clientId: 'developer-client',
  clientSecret: 'developer-secret',
  nonce: '0123456789abcdef',
  timestamp: 1_700_000_000,
};

test('developer signature hashes the binary MD5 digest with SHA-512', () => {
  assert.equal(
    buildDeveloperSignature(vector),
    '217fb5d9f8a9b7c9c65e307cda0dea4f893b5e553e231f148b9b710a609d3aa643a78574605c1f9bdff14e267811ed04bec5f4e5674a67f81493c5c818d885ac',
  );
});

test('developer headers contain no client secret', () => {
  const headers = buildDeveloperHeaders(vector);
  assert.equal(headers.client_id, vector.clientId);
  assert.equal(headers.nonce, vector.nonce);
  assert.equal(headers.timestamp, String(vector.timestamp));
  assert.equal(headers.sign, buildDeveloperSignature(vector));
  assert.doesNotMatch(JSON.stringify(headers), /developer-secret/);
});

test('developer client signs each JSON request and preserves the API error code', async () => {
  let request;
  const client = createGuangyaDeveloperClient({
    clientId: vector.clientId,
    clientSecret: vector.clientSecret,
    fetchImpl: async (url, options) => {
      request = { url, options };
      return new Response(JSON.stringify({ code: 18011, msg: 'not approved' }), { status: 200 });
    },
  });
  await assert.rejects(
    client.post('/developer/v1/upload_by_fileid', { token_id: 'target', file_ids: ['file-1'] }),
    (error) => error instanceof DeveloperApiError && error.apiCode === 18011,
  );
  assert.equal(request.url, 'https://dapi.guangyapan.com/developer/v1/upload_by_fileid');
  assert.deepEqual(JSON.parse(request.options.body), { token_id: 'target', file_ids: ['file-1'] });
  assert.equal(request.options.headers.client_id, vector.clientId);
  assert.ok(request.options.headers.sign);
});
