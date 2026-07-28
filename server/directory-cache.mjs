const DEFAULT_FRESH_MS = 60_000;
const DEFAULT_STALE_MS = 10 * 60_000;
const DEFAULT_MAX_ENTRIES = 2_048;

function valueOf(entry, ...keys) {
  for (const key of keys) {
    if (entry?.[key] !== undefined && entry?.[key] !== null) return entry[key];
  }
  return undefined;
}

function isDirectory(entry) {
  return valueOf(entry, 'resType', 'type') === 2
    || valueOf(entry, 'resType', 'type') === '2'
    || entry?.isDirectory === true;
}

function directoryFingerprint(entry) {
  return [
    String(valueOf(entry, 'fileName', 'name') || ''),
    String(valueOf(entry, 'updatedAt', 'updateTime', 'modifiedAt', 'modifyTime', 'createdAt', 'createTime') || ''),
    String(valueOf(entry, 'fileSize', 'size') || 0),
  ].join('\0');
}

function directoryMap(records) {
  return new Map((Array.isArray(records) ? records : [])
    .filter(isDirectory)
    .map((entry) => [String(valueOf(entry, 'fileId', 'id') || ''), directoryFingerprint(entry)])
    .filter(([id]) => id));
}

export function createDirectoryCache({
  freshMs = DEFAULT_FRESH_MS,
  staleMs = DEFAULT_STALE_MS,
  maxEntries = DEFAULT_MAX_ENTRIES,
  now = Date.now,
} = {}) {
  const entries = new Map();
  const generations = new Map();
  const inflight = new Map();
  let scope;

  function generation(key) {
    return generations.get(key) || 0;
  }

  function invalidate(key) {
    const normalized = String(key || '');
    entries.delete(normalized);
    generations.set(normalized, generation(normalized) + 1);
  }

  function invalidateSubtree(rootId) {
    const pending = [String(rootId || '')];
    while (pending.length) {
      const key = pending.shift();
      const cached = entries.get(key);
      if (cached) pending.push(...directoryMap(cached.value).keys());
      invalidate(key);
    }
  }

  function store(key, expectedGeneration, value) {
    if (generation(key) !== expectedGeneration) return false;
    const oldDirectories = directoryMap(entries.get(key)?.value);
    const nextDirectories = directoryMap(value);
    for (const [id, fingerprint] of oldDirectories) {
      if (nextDirectories.get(id) !== fingerprint) invalidate(id);
    }
    const timestamp = now();
    entries.set(key, { value, fetchedAt: timestamp, accessedAt: timestamp });
    while (entries.size > maxEntries) {
      const oldest = [...entries.entries()]
        .sort((left, right) => left[1].accessedAt - right[1].accessedAt)[0]?.[0];
      if (oldest === undefined) break;
      entries.delete(oldest);
    }
    return true;
  }

  function load(key, loader) {
    const expectedGeneration = generation(key);
    const current = inflight.get(key);
    if (current?.generation === expectedGeneration) return current.promise;
    const record = { generation: expectedGeneration, promise: null };
    record.promise = Promise.resolve()
      .then(loader)
      .then((value) => {
        store(key, expectedGeneration, value);
        return value;
      })
      .finally(() => {
        if (inflight.get(key) === record) inflight.delete(key);
      });
    inflight.set(key, record);
    return record.promise;
  }

  async function get(parentId, loader, { force = false } = {}) {
    const key = String(parentId || '');
    const cached = entries.get(key);
    if (cached) cached.accessedAt = now();
    const age = cached ? now() - cached.fetchedAt : Number.POSITIVE_INFINITY;
    if (!force && cached && age <= freshMs) return cached.value;
    if (!force && cached && age <= staleMs) {
      void load(key, loader).catch(() => {});
      return cached.value;
    }
    try {
      return await load(key, loader);
    } catch (error) {
      if (cached) return cached.value;
      throw error;
    }
  }

  function clear() {
    const keys = new Set([...entries.keys(), ...generations.keys(), ...inflight.keys()]);
    entries.clear();
    for (const key of keys) generations.set(key, generation(key) + 1);
  }

  function setScope(nextScope) {
    const normalized = String(nextScope || '');
    if (scope === undefined) {
      scope = normalized;
      return;
    }
    if (scope !== normalized) {
      clear();
      scope = normalized;
    }
  }

  function stats() {
    return { entries: entries.size, inflight: inflight.size };
  }

  return { clear, get, invalidate, invalidateSubtree, setScope, stats };
}
