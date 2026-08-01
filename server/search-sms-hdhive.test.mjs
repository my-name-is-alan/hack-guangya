import assert from 'node:assert/strict';
import { once } from 'node:events';
import fsp from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { DatabaseSync } from 'node:sqlite';
import { startTestServer, stopTestServer, waitUntil } from './test-helpers.mjs';

const WINDOWS_CLIENT_ID = 'aMe_SVSlkrbQXpUT';

async function listen(server) {
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  return `http://127.0.0.1:${server.address().port}`;
}

async function close(server) {
  if (!server?.listening) return;
  await new Promise((resolve) => server.close(resolve));
}

async function jsonBody(request) {
  const chunks = [];
  for await (const chunk of request) chunks.push(chunk);
  return chunks.length ? JSON.parse(Buffer.concat(chunks).toString('utf8')) : {};
}

function sendJson(response, payload, status = 200) {
  response.writeHead(status, { 'content-type': 'application/json' });
  response.end(JSON.stringify(payload));
}

test('全盘搜索、上传并发、短信登录和 HDHive 总开关按真实上游契约工作', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-search-sms-test-'));
  const apiRequests = [];
  const accountRequests = [];
  const hdhiveRequests = [];
  let activeTaskChecks = 0;
  let maxActiveTaskChecks = 0;

  const searchItems = [
    { fileId: 'image-1', fileName: 'holiday.jpg', resType: 1, fileType: 1 },
    { fileId: 'video-1', fileName: 'holiday.mkv', resType: 1, fileType: 2 },
    { fileId: 'audio-1', fileName: 'holiday.mp3', resType: 1, fileType: 3 },
    { fileId: 'document-1', fileName: 'holiday.PDF', resType: 1, fileType: 4 },
    { fileId: 'archive-1', fileName: 'holiday.zip', resType: 1, fileType: 5 },
    { fileId: 'folder-1', fileName: 'holiday', resType: 2, fileType: 0 },
  ];

  const apiServer = http.createServer(async (request, response) => {
    const body = await jsonBody(request);
    apiRequests.push({ url: request.url, body, headers: request.headers });
    if (request.url === '/userres/v1/file/search_files') return sendJson(response, { code: 0, data: { list: searchItems, total: searchItems.length } });
    if (request.url === '/cloudcollection/v1/create_task') return sendJson(response, { code: 0, data: { taskId: 'offline-task-1' } });
    if (request.url === '/cloudcollection/v1/list_task') return sendJson(response, { code: 0, data: { list: [], total: 250 } });
    if (request.url === '/userres/v1/file/get_file_list') {
      const list = searchItems.filter((item) => item.resType === body.resType
        && (!Array.isArray(body.fileTypes) || body.fileTypes.includes(item.fileType)));
      return sendJson(response, { code: 0, data: { list, total: list.length } });
    }
    if (request.url === '/userres/v1/get_share_list') return sendJson(response, { code: 0, data: { list: [], total: 0 } });
    if (request.url === '/userres/v1/share_file') return sendJson(response, { code: 0, data: { shareId: 'share-1', shareUrl: 'https://www.guangyapan.com/s/share-1' } });
    if (request.url === '/userres/v1/get_res_center_token') return sendJson(response, { code: 156, data: { taskId: `task-${body.name}` } });
    if (request.url === '/userres/v1/file/get_info_by_task_id') {
      activeTaskChecks += 1;
      maxActiveTaskChecks = Math.max(maxActiveTaskChecks, activeTaskChecks);
      await new Promise((resolve) => setTimeout(resolve, 450));
      activeTaskChecks -= 1;
      return sendJson(response, { code: 0, data: { fileId: `remote-${body.taskId}` } });
    }
    return sendJson(response, { code: 404, msg: 'not found' }, 404);
  });

  const accountServer = http.createServer(async (request, response) => {
    const body = await jsonBody(request);
    accountRequests.push({ url: request.url, body, headers: request.headers });
    if (request.url === '/v1/shield/captcha/init') {
      if (String(body.meta?.phone_number || '').endsWith('0001')) return sendJson(response, { data: { url: 'https://captcha.example.test/frame' } });
      return sendJson(response, { data: { captcha_token: 'automatic-captcha-token', expires_in: 300 } });
    }
    if (request.url === '/v1/auth/verification') {
      const suffix = String(body.phone_number).slice(-4);
      return sendJson(response, { verification_id: `verification-${suffix}`, is_user: suffix !== '0002' });
    }
    if (request.url === '/v1/auth/verification/verify') return sendJson(response, { verification_token: `verified-${body.verification_id}` });
    if (request.url === '/v1/auth/signin') return sendJson(response, { access_token: 'sms-user-access', refresh_token: 'sms-user-refresh' });
    if (request.url === '/v1/auth/signup') return sendJson(response, { access_token: 'sms-new-access', refresh_token: 'sms-new-refresh' });
    return sendJson(response, { error: 'not_found', error_description: 'not found' }, 404);
  });

  const hdhiveServer = http.createServer(async (request, response) => {
    const body = await jsonBody(request);
    hdhiveRequests.push({ method: request.method, url: request.url, body });
    if (request.method === 'POST') return sendJson(response, { data: { status: 'accepted' } }, 202);
    return sendJson(response, { data: { status: 'completed', action: 'created' } });
  });

  let instance;
  try {
    const [apiBase, accountBase, hdhiveBase] = await Promise.all([listen(apiServer), listen(accountServer), listen(hdhiveServer)]);
    instance = await startTestServer(root, {
      GUANGYA_API_BASE: apiBase,
      GUANGYA_ACCOUNT_BASE: accountBase,
      GUANGYA_TOKEN: 'initial-cloud-token',
      GUANGYA_FILE_STABILITY_MS: '200',
    });
    const base = `http://127.0.0.1:${instance.port}`;

    const videoSearch = await fetch(`${base}/api/search?query=holiday&type=video&page=0`).then((response) => response.json());
    assert.deepEqual(videoSearch.data.list.map((item) => item.fileId), ['video-1']);
    assert.equal(videoSearch.data.total, 1);
    assert.equal(videoSearch.data.remote_total, 6);
    const searchRequest = apiRequests.find((entry) => entry.url === '/userres/v1/file/search_files');
    assert.deepEqual(searchRequest.body, { name: 'holiday', pageSize: 100, page: 0 });
    assert.equal(searchRequest.headers.authorization, 'Bearer initial-cloud-token');
    assert.equal(searchRequest.headers.dt, '5');
    assert.equal(searchRequest.headers.av, '1.0.2');
    assert.equal(searchRequest.headers.vc, '1002');
    assert.equal(searchRequest.headers['x-client-id'], WINDOWS_CLIENT_ID);
    assert.match(searchRequest.headers['x-device-id'], /^[a-f0-9]{32}$/);
    assert.equal(searchRequest.headers['user-agent'], 'GuangyapanPC/1.0.2');

    const offline = await fetch(`${base}/api/offline`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ url: 'magnet:?xt=urn:btih:example', parent_id: 'folder-1', new_name: '示例' }),
    });
    assert.equal(offline.status, 200, await offline.clone().text());
    const offlineRequest = apiRequests.find((entry) => entry.url === '/cloudcollection/v1/create_task');
    assert.deepEqual(offlineRequest.body, {
      url: 'magnet:?xt=urn:btih:example',
      parentId: 'folder-1',
      newName: '示例',
    });
    await fetch(`${base}/api/offline`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ url: 'ed2k://|file|example.iso|1|ABC|/', parent_id: '' }),
    });
    assert.deepEqual(apiRequests.filter((entry) => entry.url === '/cloudcollection/v1/create_task').at(-1).body, {
      url: 'ed2k://|file|example.iso|1|ABC|/',
      parentId: '',
    });
    const offlineList = await fetch(`${base}/api/offline?cursor=cursor-2&pageSize=100`).then((response) => response.json());
    assert.equal(offlineList.data.total, 250);
    const offlineListRequest = apiRequests.find((entry) => entry.url === '/cloudcollection/v1/list_task');
    assert.deepEqual(offlineListRequest.body, { cursor: 'cursor-2', pageSize: 100 });

    const extensionSearch = await fetch(`${base}/api/search?query=holiday&extension=.pdf`).then((response) => response.json());
    assert.deepEqual(extensionSearch.data.list.map((item) => item.fileId), ['document-1']);
    const folderSearch = await fetch(`${base}/api/search?query=holiday&type=folder`).then((response) => response.json());
    assert.deepEqual(folderSearch.data.list.map((item) => item.fileId), ['folder-1']);
    assert.equal((await fetch(`${base}/api/search?query=holiday&type=executable`)).status, 400);

    const listRequestStart = apiRequests.length;
    const emptyVideoSearch = await fetch(`${base}/api/search?type=video&page=3`).then((response) => response.json());
    assert.deepEqual(emptyVideoSearch.data.list.map((item) => item.fileId), ['video-1']);
    assert.equal(emptyVideoSearch.data.remote_total, 1);
    assert.equal(emptyVideoSearch.data.remote_count, 1);
    const emptyExtensionSearch = await fetch(`${base}/api/search?extension=.pdf`).then((response) => response.json());
    assert.deepEqual(emptyExtensionSearch.data.list.map((item) => item.fileId), ['document-1']);
    const emptyFolderSearch = await fetch(`${base}/api/search?type=folder`).then((response) => response.json());
    assert.deepEqual(emptyFolderSearch.data.list.map((item) => item.fileId), ['folder-1']);
    const listRequests = apiRequests.slice(listRequestStart);
    assert.deepEqual(listRequests.map((entry) => entry.url), Array(3).fill('/userres/v1/file/get_file_list'));
    assert.deepEqual(listRequests.map((entry) => entry.body), [
      { parentId: '*', pageSize: 100, page: 3, resType: 1, orderBy: 3, sortType: 1, fileTypes: [2] },
      { parentId: '*', pageSize: 100, page: 0, resType: 1, orderBy: 3, sortType: 1, fileTypes: [4] },
      { parentId: '*', pageSize: 100, page: 0, resType: 2, orderBy: 3, sortType: 1 },
    ]);

    const settings = await fetch(`${base}/api/settings/transfer`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ upload_concurrency: 3 }),
    });
    assert.equal(settings.status, 200, await settings.clone().text());
    const uploads = await Promise.all(Array.from({ length: 4 }, (_, index) => fetch(`${base}/api/upload?fileName=file-${index}.txt&relativePath=file-${index}.txt&lastModified=${1000 + index}`, {
      method: 'POST', headers: { 'content-type': 'text/plain' }, body: Buffer.from(`upload-${index}`),
    })));
    assert.deepEqual(uploads.map((response) => response.status), [202, 202, 202, 202]);
    await waitUntil(async () => {
      const state = await fetch(`${base}/api/state`).then((response) => response.json());
      return state.pending === 0 && state.active_uploads === 0;
    }, 12_000);
    // 三个普通上传槽之外，允许一条独立的后台秒传预检通道。
    assert.equal(maxActiveTaskChecks, 4);

    const captchaRequired = await fetch(`${base}/api/auth/sms/send`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ phone_number: '13800000001' }),
    }).then((response) => response.json());
    assert.equal(captchaRequired.captcha_required, true);
    assert.equal(captchaRequired.url, 'https://captcha.example.test/frame');
    assert.equal(accountRequests.filter((entry) => entry.url === '/v1/auth/verification').length, 0);

    const sent = await fetch(`${base}/api/auth/sms/send`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ phone: '138 0000 0000' }),
    }).then((response) => response.json());
    assert.deepEqual(sent, { verification_id: 'verification-0000', request_id: 'verification-0000', is_user: true, phone_number: '+86 13800000000', captcha_required: false });
    const verificationRequest = accountRequests.find((entry) => entry.url === '/v1/auth/verification' && entry.body.phone_number.endsWith('0000'));
    assert.equal(verificationRequest.headers['x-captcha-token'], 'automatic-captcha-token');
    assert.deepEqual(verificationRequest.body, {
      phone_number: '+86 13800000000',
      target: 'ANY',
      client_id: WINDOWS_CLIENT_ID,
      usage: 'SIGN_IN',
      selected_channel: 'VERIFICATION_PHONE',
    });

    const loggedIn = await fetch(`${base}/api/auth/sms/login`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ verification_id: sent.verification_id, verification_code: '123456' }),
    }).then((response) => response.json());
    assert.deepEqual(loggedIn, { authenticated: true, is_user: true });
    const signin = accountRequests.find((entry) => entry.url === '/v1/auth/signin');
    assert.deepEqual(signin.body, {
      username: '+86 13800000000',
      verification_code: '123456',
      verification_token: 'verified-verification-0000',
      client_id: WINDOWS_CLIENT_ID,
    });

    const newUserSent = await fetch(`${base}/api/auth/sms/send`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ phone_number: '+86 13800000002', captcha_token: 'solved-captcha-token' }),
    }).then((response) => response.json());
    assert.equal(newUserSent.is_user, false);
    const newUserVerification = accountRequests.find((entry) => entry.url === '/v1/auth/verification' && entry.body.phone_number.endsWith('0002'));
    assert.equal(newUserVerification.headers['x-captcha-token'], 'solved-captcha-token');
    await fetch(`${base}/api/auth/sms/login`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ verification_id: newUserSent.verification_id, code: '654321' }),
    }).then(async (response) => assert.equal(response.status, 200, await response.text()));
    const signup = accountRequests.find((entry) => entry.url === '/v1/auth/signup');
    assert.deepEqual(signup.body, {
      phone_number: '+86 13800000002',
      verification_code: '654321',
      verification_token: 'verified-verification-0002',
      client_id: WINDOWS_CLIENT_ID,
      name: '光鸭用户0002',
    });
    const database = new DatabaseSync(path.join(instance.dataDir, 'state.sqlite3'));
    const auth = database.prepare('SELECT access_token, refresh_token FROM auth_session WHERE id = 1').get();
    database.close();
    assert.equal(auth.access_token, 'sms-new-access');
    assert.equal(auth.refresh_token, 'sms-new-refresh');

    const disabled = await fetch(`${base}/api/hdhive/config`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ base_url: hdhiveBase, secret: 'integration-secret', enabled: false }),
    });
    assert.equal(disabled.status, 200, await disabled.clone().text());
    const disabledShare = await fetch(`${base}/api/share`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ file_ids: ['folder-1'], title: '关闭投稿测试', target_type: 'folder' }),
    }).then((response) => response.json());
    assert.equal(disabledShare.hdhive_status, 'disabled');
    assert.equal(hdhiveRequests.length, 0);

    await fetch(`${base}/api/hdhive/config`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ enabled: true }),
    });
    const enabledShare = await fetch(`${base}/api/share`, {
      method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ file_ids: ['folder-2'], title: '开启投稿测试', target_type: 'folder' }),
    }).then((response) => response.json());
    assert.equal(enabledShare.hdhive_status, 'accepted');
    assert.equal(hdhiveRequests.filter((entry) => entry.method === 'POST').length, 1);
  } finally {
    await stopTestServer(instance);
    await Promise.all([close(apiServer), close(accountServer), close(hdhiveServer)]);
    await fsp.rm(root, { recursive: true, force: true });
  }
});

test('带本地文件类型过滤的搜索会跨远端页收集并返回可继续分页的总数', async () => {
  const root = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-search-page-test-'));
  const requestedPages = [];
  const remotePages = [
    [
      ...Array.from({ length: 60 }, (_, index) => ({ fileId: `p0-pdf-${index}`, fileName: `needle-${index}.pdf`, resType: 1 })),
      ...Array.from({ length: 40 }, (_, index) => ({ fileId: `p0-jpg-${index}`, fileName: `needle-${index}.jpg`, resType: 1 })),
    ],
    [
      ...Array.from({ length: 60 }, (_, index) => ({ fileId: `p1-pdf-${index}`, fileName: `needle-${index}.pdf`, resType: 1 })),
      ...Array.from({ length: 40 }, (_, index) => ({ fileId: `p1-jpg-${index}`, fileName: `needle-${index}.jpg`, resType: 1 })),
    ],
    [{ fileId: 'p2-jpg-0', fileName: 'needle-tail.jpg', resType: 1 }],
  ];
  const apiServer = http.createServer(async (request, response) => {
    const body = await jsonBody(request);
    if (request.url !== '/userres/v1/file/search_files') return sendJson(response, { code: 404, msg: 'not found' }, 404);
    requestedPages.push(body.page);
    return sendJson(response, { code: 0, msg: 'success', data: { list: remotePages[body.page] || [], total: 201 } });
  });
  let instance;
  try {
    const apiBase = await listen(apiServer);
    instance = await startTestServer(root, {
      GUANGYA_API_BASE: apiBase,
      GUANGYA_TOKEN: 'search-page-token',
    });
    const base = `http://127.0.0.1:${instance.port}`;
    const response = await fetch(`${base}/api/search?query=needle&extension=pdf&page=1`).then((value) => value.json());
    assert.equal(response.data.list.length, 20);
    assert.deepEqual(response.data.list.map((item) => item.fileId), Array.from({ length: 20 }, (_, index) => `p1-pdf-${index + 40}`));
    assert.equal(response.data.total, 120);
    assert.equal(response.data.remote_total, 201);
    assert.equal(response.data.remote_count, 201);
    assert.deepEqual(requestedPages, [0, 1, 2]);
  } finally {
    await stopTestServer(instance);
    await close(apiServer);
    await fsp.rm(root, { recursive: true, force: true });
  }
});
