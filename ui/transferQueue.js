export const DEFAULT_TRANSFER_CONCURRENCY = 2;
export const MAX_TRANSFER_CONCURRENCY = 8;

export function normalizeTransferConcurrency(value, fallback = DEFAULT_TRANSFER_CONCURRENCY) {
  const parsed = Math.round(Number(value));
  return Number.isFinite(parsed) && parsed >= 1 && parsed <= MAX_TRANSFER_CONCURRENCY
    ? parsed
    : fallback;
}

export function createConcurrencyQueue(getLimit) {
  const pending = [];
  let active = 0;

  function pump() {
    const limit = normalizeTransferConcurrency(getLimit());
    while (active < limit && pending.length) {
      const run = pending.shift();
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
    enqueue(run) {
      pending.push(run);
      pump();
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
