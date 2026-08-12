export const RECYCLE_CLEAR_STATE_KEY = 'recycle_clear_task_v1';
export const RECYCLE_CLEAR_DEADLINE_MS = 120_000;
export const RECYCLE_CLEAR_POLL_MS = 1_000;
export const RECYCLE_CLEAR_UNKNOWN_GUARD_MS = 120_000;

const CLEAR_ENDPOINT = '/userres/v1/file/clear_recycle_bin';
const STATUS_ENDPOINT = '/userres/v1/get_task_status';

function defaultSleep(delayMs) {
  return new Promise((resolve) => setTimeout(resolve, delayMs));
}

function taskIdFrom(value) {
  const taskId = String(value || '').trim();
  return taskId || null;
}

function normalizedState(value, fallbackScope = '') {
  const scope = String(value?.scope || fallbackScope || '').trim();
  if (!scope) return null;
  if (value?.phase === 'pending' && taskIdFrom(value.taskId)) {
    return {
      phase: 'pending',
      scope,
      taskId: taskIdFrom(value.taskId),
      startedAt: Number(value.startedAt) || 0,
      updatedAt: Number(value.updatedAt) || 0,
    };
  }
  if (value?.phase === 'unknown' && Number.isFinite(Number(value.retryNotBefore))) {
    return {
      phase: 'unknown',
      scope,
      startedAt: Number(value.startedAt) || 0,
      updatedAt: Number(value.updatedAt) || 0,
      retryNotBefore: Number(value.retryNotBefore),
      reason: String(value.reason || ''),
    };
  }
  return null;
}

function parseStoredEnvelope(raw) {
  if (!raw) return { version: 1, scopes: {} };
  try {
    const parsed = typeof raw === 'string' ? JSON.parse(raw) : raw;
    const scopes = {};
    if (parsed?.version === 1 && parsed.scopes && typeof parsed.scopes === 'object') {
      for (const [key, value] of Object.entries(parsed.scopes)) {
        const state = normalizedState(value, key);
        if (state) scopes[state.scope] = state;
      }
      return { version: 1, scopes };
    }
    // Accept an early scoped draft of this state shape during development.
    const state = normalizedState(parsed);
    if (state) scopes[state.scope] = state;
    return { version: 1, scopes };
  } catch {
    // A corrupt value cannot identify an accepted cloud task, so discard it.
  }
  return { version: 1, scopes: {} };
}

function initialSubmissionMayHaveReachedCloud(error) {
  const status = Number(error?.httpStatus ?? error?.statusCode);
  return error?.retryable === true
    || ['AbortError', 'TimeoutError'].includes(error?.name)
    || status === 408
    || status === 429
    || status >= 500;
}

function pendingResult(taskId, message, lastError = '') {
  return {
    status: 'pending',
    pending: true,
    taskId,
    task_id: taskId,
    message,
    ...(lastError ? { last_error: lastError } : {}),
  };
}

function unknownResult() {
  return {
    status: 'unknown',
    pending: true,
    taskId: null,
    task_id: null,
    force_retry_required: true,
    message: '云端是否已接收清空任务暂时无法确认。为避免重复删除，程序不会自动重新提交；仅在人工确认后才能强制重新发起。',
  };
}

export class RecycleClearTaskFailedError extends Error {
  constructor(message, taskId, detail = {}) {
    super(message || '清空回收站失败');
    this.name = 'RecycleClearTaskFailedError';
    this.statusCode = 400;
    this.taskId = taskId;
    this.detail = detail;
    this.terminal = true;
  }
}

export function createRecycleClearTaskCoordinator({
  loadState,
  saveState,
  clearState,
  apiPost,
  onTerminal = () => {},
  now = () => Date.now(),
  sleep = defaultSleep,
  deadlineMs = RECYCLE_CLEAR_DEADLINE_MS,
  pollMs = RECYCLE_CLEAR_POLL_MS,
  unknownGuardMs = RECYCLE_CLEAR_UNKNOWN_GUARD_MS,
  requestTimeoutMs = RECYCLE_CLEAR_DEADLINE_MS,
  scope = () => 'default',
} = {}) {
  if (typeof loadState !== 'function' || typeof saveState !== 'function' || typeof clearState !== 'function') {
    throw new TypeError('recycle clear state store is required');
  }
  if (typeof apiPost !== 'function') throw new TypeError('recycle clear apiPost is required');

  const volatileStates = new Map();
  const inFlight = new Map();
  const currentScope = () => String(scope() || 'logged-out');

  const load = (operationScope) => {
    if (volatileStates.has(operationScope)) return volatileStates.get(operationScope);
    const state = parseStoredEnvelope(loadState()).scopes[operationScope] || null;
    if (state) volatileStates.set(operationScope, state);
    return state;
  };
  const save = (operationScope, state) => {
    const scopedState = { ...state, scope: operationScope };
    volatileStates.set(operationScope, scopedState);
    const envelope = parseStoredEnvelope(loadState());
    envelope.scopes[operationScope] = scopedState;
    saveState(JSON.stringify(envelope));
  };
  const clear = (operationScope) => {
    volatileStates.delete(operationScope);
    const envelope = parseStoredEnvelope(loadState());
    delete envelope.scopes[operationScope];
    if (Object.keys(envelope.scopes).length) saveState(JSON.stringify(envelope));
    else clearState();
  };
  const remainingMs = (deadline) => Math.max(0, deadline - now());
  const callApi = (endpoint, body, deadline) => {
    const remaining = remainingMs(deadline);
    if (remaining <= 0) return null;
    return apiPost(endpoint, body, {
      timeoutMs: Math.max(1, Math.min(requestTimeoutMs, remaining)),
    });
  };
  const finishTerminal = async (outcome, taskId, operationScope) => {
    clear(operationScope);
    await onTerminal({ outcome, taskId, scope: operationScope });
  };

  async function pollPending(state, deadline, operationScope) {
    let lastError = '';
    while (remainingMs(deadline) > 0) {
      if (currentScope() !== operationScope) {
        return pendingResult(
          state.taskId,
          '登录账号已切换，原账号的清空任务仍被保留；切回原账号并再次点击清空时会继续查询，不会使用当前账号重复提交。',
        );
      }
      let response;
      try {
        response = await callApi(STATUS_ENDPOINT, { taskId: state.taskId }, deadline);
      } catch (error) {
        lastError = String(error?.message || error || '查询清空任务失败');
        if (error?.retryable !== true) {
          return pendingResult(
            state.taskId,
            '清空任务仍保留在云端，当前无法确认状态；再次点击清空时会继续查询，不会重复提交。',
            lastError,
          );
        }
      }

      if (response) {
        const status = Number(response.data?.status);
        const detail = response.data?.detail || {};
        const detailCode = Number(detail.code || 0);
        if (status === 2 && detailCode === 0) {
          await finishTerminal('completed', state.taskId, operationScope);
          return {
            status: 'completed',
            pending: false,
            taskId: state.taskId,
            task_id: state.taskId,
            message: '回收站已清空',
          };
        }
        if (status === 3 || ([2, 3].includes(status) && detailCode !== 0)) {
          await finishTerminal('failed', state.taskId, operationScope);
          throw new RecycleClearTaskFailedError(detail.msg || '清空回收站失败', state.taskId, detail);
        }
      }

      const delayMs = Math.min(pollMs, remainingMs(deadline));
      if (delayMs > 0) await sleep(delayMs);
    }
    return pendingResult(
      state.taskId,
      '清空任务仍在云端执行；再次点击清空时会继续查询，不会重复提交。',
      lastError,
    );
  }

  async function run(operationScope, forceRetry) {
    const deadline = now() + deadlineMs;
    let state = load(operationScope);
    if (state?.phase === 'pending') return pollPending(state, deadline, operationScope);
    if (state?.phase === 'unknown') {
      if (!forceRetry) return unknownResult();
      clear(operationScope);
      state = null;
    }

    let result;
    const submissionStartedAt = now();
    const submissionMarker = {
      phase: 'unknown',
      startedAt: submissionStartedAt,
      updatedAt: submissionStartedAt,
      retryNotBefore: submissionStartedAt + unknownGuardMs,
      reason: '正在提交清空任务',
    };
    // Persist before the destructive request. If the process exits after the
    // cloud accepts it but before the response is stored, restart protection
    // must already be in place.
    save(operationScope, submissionMarker);
    try {
      result = await callApi(CLEAR_ENDPOINT, {}, deadline);
      if (!result) {
        const error = new Error('提交清空任务前已超过等待时限');
        error.retryable = true;
        throw error;
      }
    } catch (error) {
      if (!initialSubmissionMayHaveReachedCloud(error)) {
        clear(operationScope);
        throw error;
      }
      const timestamp = now();
      const unknownState = {
        phase: 'unknown',
        startedAt: timestamp,
        updatedAt: timestamp,
        retryNotBefore: timestamp + unknownGuardMs,
        reason: String(error?.message || error || '提交结果未知').slice(0, 500),
      };
      save(operationScope, unknownState);
      return unknownResult();
    }

    const taskId = taskIdFrom(result.data?.taskId);
    if (!taskId) {
      await finishTerminal('completed', null, operationScope);
      return {
        ...(result.data || {}),
        status: 'completed',
        pending: false,
        taskId: null,
        task_id: null,
        message: '回收站已清空',
      };
    }

    const timestamp = now();
    state = { phase: 'pending', taskId, startedAt: timestamp, updatedAt: timestamp };
    save(operationScope, state);
    return pollPending(state, deadline, operationScope);
  }

  return {
    clearRecycleBin({ forceRetry = false } = {}) {
      const operationScope = currentScope();
      if (inFlight.has(operationScope)) return inFlight.get(operationScope);
      const operation = run(operationScope, forceRetry === true);
      inFlight.set(operationScope, operation);
      operation.finally(() => {
        if (inFlight.get(operationScope) === operation) inFlight.delete(operationScope);
      }).catch(() => {});
      return operation;
    },
    state() {
      return load(currentScope());
    },
  };
}
