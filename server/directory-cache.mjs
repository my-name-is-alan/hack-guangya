// WebDAV clients (especially rclone mounts) keep their own directory cache.
// Keep this server-side cache deliberately short so a change made by another
// mount/process becomes observable on the next VFS refresh instead of being
// hidden behind a second minute-long cache.
const DEFAULT_FRESH_MS = 2_000;
const DEFAULT_STALE_MS = 15_000;
const DEFAULT_MAX_ENTRIES = 2_048;
const DEFAULT_ACTIVE_MS = 60_000;
const DEFAULT_REFRESH_LIMIT = 4;

class InvalidatedDirectoryLoadError extends Error {
  constructor() {
    super('directory cache load was invalidated');
    this.name = 'InvalidatedDirectoryLoadError';
  }
}

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
    String(valueOf(entry, 'updatedAt', 'updateTime', 'modifiedAt', 'modifyTime', 'utime', 'createdAt', 'createTime', 'ctime') || ''),
    String(valueOf(entry, 'fileSize', 'size') || 0),
  ].join('\0');
}

function directoryName(entry) {
  return String(valueOf(entry, 'fileName', 'name') || '');
}

function directoryMap(records) {
  return new Map((Array.isArray(records) ? records : [])
    .filter(isDirectory)
    .map((entry) => [String(valueOf(entry, 'fileId', 'id') || ''), directoryFingerprint(entry)])
    .filter(([id]) => id));
}

function directoryNameMap(records) {
  return new Map((Array.isArray(records) ? records : [])
    .filter(isDirectory)
    .map((entry) => [String(valueOf(entry, 'fileId', 'id') || ''), directoryName(entry)])
    .filter(([id]) => id));
}

export function createDirectoryCache({
  freshMs = DEFAULT_FRESH_MS,
  staleMs = DEFAULT_STALE_MS,
  maxEntries = DEFAULT_MAX_ENTRIES,
  activeMs = DEFAULT_ACTIVE_MS,
  refreshLimit = DEFAULT_REFRESH_LIMIT,
  onDirectoryInvalidated = () => {},
  now = Date.now,
} = {}) {
  const entries = new Map();
  const generations = new Map();
  const inflight = new Map();
  // `inflight` only points at the newest request for a key. Older requests can
  // still be running after an invalidation replaces that pointer, so keep a
  // separate count until every generation has settled. Otherwise generation
  // cleanup can reset to zero and let an old response overwrite newer data.
  const activeLoads = new Map();
  let scope;

  function generation(key) {
    return generations.get(key) || 0;
  }

  function cleanupGeneration(key) {
    if (!entries.has(key) && !inflight.has(key) && !activeLoads.has(key)) {
      generations.delete(key);
    }
  }

  function invalidate(key) {
    const normalized = String(key || '');
    entries.delete(normalized);
    generations.set(normalized, generation(normalized) + 1);
    cleanupGeneration(normalized);
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

  // 反查条目所在的父目录并精确失效（对齐 Rust 端 invalidate_entries）：
  // 返回定位到的父目录列表，以及是否所有条目都被定位。
  function invalidateEntries(entryIds) {
    const wanted = new Set((Array.isArray(entryIds) ? entryIds : [])
      .map((id) => String(id || ''))
      .filter(Boolean));
    if (!wanted.size) return { parents: [], allLocated: true };
    const parents = new Set();
    const located = new Set();
    for (const [parentId, cached] of entries) {
      for (const record of Array.isArray(cached.value) ? cached.value : []) {
        const id = String(valueOf(record, 'fileId', 'id') || '');
        if (wanted.has(id)) {
          parents.add(parentId);
          located.add(id);
        }
      }
    }
    for (const parentId of parents) invalidate(parentId);
    for (const id of wanted) invalidateSubtree(id);
    return { parents: [...parents], allLocated: located.size === wanted.size };
  }

  // 只把快照标脏（不递增 generation）：下一次读取走 stale-while-revalidate。
  // 用于"UI 刚读过这个目录"这类读路径，避免打断在途的挂载端加载。
  function markStale(parentId) {
    const cached = entries.get(String(parentId || ''));
    if (cached) cached.fetchedAt = now() - freshMs - 1;
  }

  // 用一份已知的完整目录快照覆写缓存（generation 不变时才写入）。
  function overwriteSnapshot(parentId, records) {
    const key = String(parentId || '');
    const cached = entries.get(key);
    store(key, generation(key), records, cached?.loader);
  }

  function store(key, expectedGeneration, value, loader) {
    if (generation(key) !== expectedGeneration) return false;
    const previous = entries.get(key);
    const oldDirectories = directoryMap(previous?.value);
    const nextDirectories = directoryMap(value);
    const oldNames = directoryNameMap(previous?.value);
    const nextNames = directoryNameMap(value);
    for (const [id, fingerprint] of oldDirectories) {
      if (nextDirectories.get(id) !== fingerprint) {
        // 内容指纹（含大小/时间）变化 → 该子目录的列表快照需要重新拉取。
        invalidate(id);
        // 但"路径→ID"映射只有在子目录被改名或消失时才会失效；上传过程中
        // 目录大小持续变化，若也通知观察者会把上传路径缓存反复清掉，
        // 造成 ensureRemote 的 generation 重试风暴。
        const renamedOrGone = !nextNames.has(id) || nextNames.get(id) !== oldNames.get(id);
        if (renamedOrGone) {
          try { onDirectoryInvalidated(id); } catch { /* cache observers must not break reads */ }
        }
      }
    }
    const timestamp = now();
    entries.set(key, {
      value,
      loader,
      fetchedAt: timestamp,
      accessedAt: previous?.accessedAt ?? timestamp,
    });
    while (entries.size > maxEntries) {
      const oldest = [...entries.entries()]
        .sort((left, right) => left[1].accessedAt - right[1].accessedAt)[0]?.[0];
      if (oldest === undefined) break;
      entries.delete(oldest);
      cleanupGeneration(oldest);
    }
    return true;
  }

  function load(key, loader) {
    const expectedGeneration = generation(key);
    const current = inflight.get(key);
    if (current?.generation === expectedGeneration) return current.promise;
    const record = { generation: expectedGeneration, promise: null };
    activeLoads.set(key, (activeLoads.get(key) || 0) + 1);
    record.promise = Promise.resolve()
      .then(loader)
      .then((value) => {
        if (!store(key, expectedGeneration, value, loader)) {
          throw new InvalidatedDirectoryLoadError();
        }
        return value;
      })
      .finally(() => {
        if (inflight.get(key) === record) {
          inflight.delete(key);
        }
        const remaining = (activeLoads.get(key) || 1) - 1;
        if (remaining > 0) activeLoads.set(key, remaining);
        else activeLoads.delete(key);
        cleanupGeneration(key);
      });
    inflight.set(key, record);
    return record.promise;
  }

  async function get(parentId, loader, options = {}) {
    const { force = false, foreground = false } = options;
    const staleIfError = options.staleIfError ?? !force;
    const key = String(parentId || '');
    let invalidationRetries = 0;
    for (;;) {
      const expectedGeneration = generation(key);
      const cached = entries.get(key);
      if (cached) {
        cached.accessedAt = now();
        cached.loader = loader;
      }
      const age = cached ? now() - cached.fetchedAt : Number.POSITIVE_INFINITY;
      if (!force && cached && age <= freshMs) return cached.value;
      if (!force && !foreground && cached && age <= staleMs) {
        void load(key, loader).catch(() => {});
        return cached.value;
      }
      try {
        return await load(key, loader);
      } catch (error) {
        // A cloud mutation may finish while an older directory read is still in
        // flight. Retry with the new generation so that the old response is not
        // returned to the caller or written back into the cache.
        if (error instanceof InvalidatedDirectoryLoadError) {
          invalidationRetries += 1;
          if (invalidationRetries <= 8) continue;
          throw new Error('directory cache changed too frequently; retry the operation');
        }
        const fallback = entries.get(key);
        if (staleIfError && fallback && generation(key) === expectedGeneration) return fallback.value;
        throw error;
      }
    }
  }

  async function refreshActive({ force = false, limit = refreshLimit } = {}) {
    const timestamp = now();
    const candidates = [...entries.entries()]
      .filter(([, entry]) => typeof entry.loader === 'function'
        && timestamp - entry.accessedAt <= activeMs
        && (force || timestamp - entry.fetchedAt > freshMs))
      .sort((left, right) => left[1].fetchedAt - right[1].fetchedAt)
      .slice(0, Math.max(0, Number(limit) || 0));
    const results = await Promise.allSettled(
      candidates.map(([key, entry]) => load(key, entry.loader)),
    );
    return {
      attempted: candidates.length,
      refreshed: results.filter((result) => result.status === 'fulfilled').length,
    };
  }

  function activeKeys() {
    const timestamp = now();
    return [...entries.entries()]
      .filter(([, entry]) => timestamp - entry.accessedAt <= activeMs)
      .map(([key]) => key);
  }

  function dispose() {
    clear();
    scope = undefined;
  }

  function stats() {
    return {
      entries: entries.size,
      inflight: inflight.size,
      active: activeKeys().length,
    };
  }

  function clear() {
    const keys = new Set([
      ...entries.keys(),
      ...generations.keys(),
      ...inflight.keys(),
      ...activeLoads.keys(),
    ]);
    entries.clear();
    for (const key of keys) {
      generations.set(key, generation(key) + 1);
      cleanupGeneration(key);
    }
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

  return {
    clear,
    dispose,
    get,
    invalidate,
    invalidateEntries,
    invalidateSubtree,
    markStale,
    overwriteSnapshot,
    refreshActive,
    setScope,
    stats,
  };
}
