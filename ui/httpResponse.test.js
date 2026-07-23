import assert from 'node:assert/strict';
import test from 'node:test';
import { readJsonResponse } from './httpResponse.js';

test('空的 502 响应保留 HTTP 状态和排查提示', async () => {
  await assert.rejects(
    readJsonResponse(new Response('', { status: 502 }), '上传接口失败'),
    /上传接口失败（HTTP 502，服务端返回空响应；请检查反向代理和服务日志）/,
  );
});

test('JSON 错误响应显示服务端错误信息', async () => {
  await assert.rejects(
    readJsonResponse(new Response(JSON.stringify({ error: '队列不可用' }), { status: 503 }), '上传接口失败'),
    /队列不可用/,
  );
});
