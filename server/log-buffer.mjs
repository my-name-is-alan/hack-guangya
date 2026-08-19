/** 运行日志环形缓冲：供 /api/logs 与 Telegram /logs 命令读取最近 N 条。 */
export function createLogBuffer(capacity = 500) {
  const normalizedCapacity = Math.max(50, Math.min(5000, Number(capacity) || 500));
  const entries = [];
  let sequence = 0;
  function push(level, message) {
    const entry = {
      seq: ++sequence,
      time: Date.now(),
      level: String(level || 'info'),
      message: String(message ?? '').slice(0, 2000),
    };
    entries.push(entry);
    if (entries.length > normalizedCapacity) entries.splice(0, entries.length - normalizedCapacity);
    return entry;
  }
  function list(limit = 50) {
    const count = Math.max(1, Math.min(normalizedCapacity, Number(limit) || 50));
    return entries.slice(-count);
  }
  function size() {
    return entries.length;
  }
  return { push, list, size, capacity: normalizedCapacity };
}
