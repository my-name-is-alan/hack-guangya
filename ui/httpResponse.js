export async function readJsonResponse(response, fallback = '请求失败') {
  const raw = await response.text();
  if (!raw.trim()) {
    if (response.ok) return {};
    throw new Error(`${fallback}（HTTP ${response.status}，服务端返回空响应；请检查反向代理和服务日志）`);
  }

  let payload;
  try {
    payload = JSON.parse(raw.replace(/^\uFEFF/, ''));
  } catch (error) {
    if (!response.ok) throw new Error(`${fallback}（HTTP ${response.status}）：${raw.slice(0, 240)}`);
    throw new Error(`服务端返回了非 JSON 响应（HTTP ${response.status}）：${raw.slice(0, 240)}（${error.message}）`);
  }

  if (!response.ok) throw new Error(payload.error || payload.msg || `${fallback}（HTTP ${response.status}）`);
  if (payload.code && payload.code !== 0) throw new Error(payload.msg || `光鸭接口失败：${payload.code}`);
  return payload;
}
