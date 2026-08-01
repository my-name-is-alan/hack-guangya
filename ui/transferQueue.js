export const DEFAULT_TRANSFER_CONCURRENCY = 2;
export const MAX_TRANSFER_CONCURRENCY = 8;

export function normalizeTransferConcurrency(value, fallback = DEFAULT_TRANSFER_CONCURRENCY) {
  const parsed = Math.round(Number(value));
  return Number.isFinite(parsed) && parsed >= 1 && parsed <= MAX_TRANSFER_CONCURRENCY
    ? parsed
    : fallback;
}

export function createConcurrencyQueue(getLimit, getPaused = () => false) {
  const pending = [];
  let active = 0;

  function pump() {
    if (getPaused()) return;
    const limit = normalizeTransferConcurrency(getLimit());
    while (active < limit && pending.length) {
      const { run } = pending.shift();
      active += 1;
      Promise.resolve()
        .then(run)
        .catch(() => {})
        .finally(() => {
          active -= 1;
          pump();
        });
    }
  }

  return {
    enqueue(idOrRun, maybeRun) {
      const hasId = typeof idOrRun !== 'function';
      const id = hasId ? String(idOrRun) : '';
      const run = hasId ? maybeRun : idOrRun;
      if (typeof run !== 'function') throw new TypeError('下载队列任务必须是函数');
      if (id && pending.some((item) => item.id === id)) return false;
      pending.push({ id, run });
      pump();
      return true;
    },
    cancel(id) {
      const key = String(id || '');
      const index = pending.findIndex((item) => item.id === key);
      if (index < 0) return false;
      pending.splice(index, 1);
      return true;
    },
    pump,
    get active() {
      return active;
    },
    get pending() {
      return pending.length;
    },
  };
}
