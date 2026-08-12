import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createRecycleClearTaskCoordinator,
  RecycleClearTaskFailedError,
} from './recycle-clear-task.mjs';

function memoryStore(initial = null) {
  let value = initial;
  return {
    loadState: () => value,
    saveState: (next) => { value = next; },
    clearState: () => { value = null; },
    value: () => value,
  };
}

test('clear recycle submits once and concurrent callers share the same cloud task', async () => {
  const store = memoryStore();
  const calls = [];
  const terminals = [];
  let releaseSubmission;
  const submissionGate = new Promise((resolve) => { releaseSubmission = resolve; });
  const coordinator = createRecycleClearTaskCoordinator({
    ...store,
    deadlineMs: 100,
    pollMs: 1,
    sleep: async () => {},
    apiPost: async (endpoint, body, options) => {
      calls.push({ endpoint, body, options });
      if (endpoint.endsWith('/clear_recycle_bin')) {
        await submissionGate;
        return { code: 0, data: { taskId: 'clear-1' } };
      }
      return { code: 0, data: { status: 2, detail: { code: 0 } } };
    },
    onTerminal: (entry) => terminals.push(entry),
  });

  const first = coordinator.clearRecycleBin();
  const second = coordinator.clearRecycleBin();
  releaseSubmission();
  const [firstResult, secondResult] = await Promise.all([first, second]);

  assert.deepEqual(firstResult, secondResult);
  assert.equal(firstResult.status, 'completed');
  assert.equal(calls.filter((entry) => entry.endpoint.endsWith('/clear_recycle_bin')).length, 1);
  assert.deepEqual(calls.filter((entry) => entry.endpoint.endsWith('/get_task_status')).map((entry) => entry.body), [{ taskId: 'clear-1' }]);
  assert.deepEqual(terminals, [{ outcome: 'completed', taskId: 'clear-1', scope: 'default' }]);
  assert.equal(store.value(), null);
});

test('deadline returns pending, persists task id, and a restarted coordinator only resumes polling', async () => {
  const store = memoryStore();
  let time = 0;
  const calls = [];
  const options = {
    ...store,
    now: () => time,
    sleep: async (delayMs) => { time += delayMs; },
    deadlineMs: 30,
    pollMs: 10,
    requestTimeoutMs: 100,
  };
  const first = createRecycleClearTaskCoordinator({
    ...options,
    apiPost: async (endpoint, body, request) => {
      calls.push({ endpoint, body, request, time });
      if (endpoint.endsWith('/clear_recycle_bin')) return { code: 0, data: { taskId: 'persisted-clear' } };
      return { code: 0, data: { status: 1 } };
    },
  });

  const pending = await first.clearRecycleBin();
  assert.equal(pending.status, 'pending');
  assert.equal(pending.taskId, 'persisted-clear');
  assert.equal(JSON.parse(store.value()).scopes.default.taskId, 'persisted-clear');
  assert.deepEqual(
    calls.filter((entry) => entry.endpoint.endsWith('/get_task_status')).map((entry) => entry.request.timeoutMs),
    [30, 20, 10],
  );

  const restarted = createRecycleClearTaskCoordinator({
    ...options,
    apiPost: async (endpoint, body, request) => {
      calls.push({ endpoint, body, request, time });
      assert.equal(endpoint.endsWith('/get_task_status'), true);
      assert.deepEqual(body, { taskId: 'persisted-clear' });
      return { code: 0, data: { status: 2, detail: { code: 0 } } };
    },
  });
  const completed = await restarted.clearRecycleBin();
  assert.equal(completed.status, 'completed');
  assert.equal(calls.filter((entry) => entry.endpoint.endsWith('/clear_recycle_bin')).length, 1);
  assert.equal(store.value(), null);
});

test('unknown initial submission permanently blocks automatic duplicate posts until explicit force retry', async () => {
  const store = memoryStore();
  let time = 1_000;
  let submissions = 0;
  const create = (apiPost) => createRecycleClearTaskCoordinator({
    ...store,
    now: () => time,
    deadlineMs: 50,
    unknownGuardMs: 100,
    apiPost,
  });
  const first = create(async () => {
    submissions += 1;
    const error = new Error('connection reset after request write');
    error.retryable = true;
    throw error;
  });
  const unknown = await first.clearRecycleBin();
  assert.equal(unknown.status, 'unknown');
  assert.equal(unknown.force_retry_required, true);
  assert.equal(JSON.parse(store.value()).scopes.default.phase, 'unknown');

  const restarted = create(async () => {
    submissions += 1;
    return { code: 0, data: {} };
  });
  const guarded = await restarted.clearRecycleBin();
  assert.equal(guarded.status, 'unknown');
  assert.equal(submissions, 1, 'unknown protection must survive a process-level coordinator restart');

  time += 100_000_000;
  const stillGuarded = await restarted.clearRecycleBin();
  assert.equal(stillGuarded.status, 'unknown');
  assert.equal(submissions, 1, 'elapsed time must never automatically re-submit a destructive POST');

  const completed = await restarted.clearRecycleBin({ forceRetry: true });
  assert.equal(completed.status, 'completed');
  assert.equal(submissions, 2);
  assert.equal(store.value(), null);
});

test('unknown protection is stored before the destructive POST can complete', async () => {
  const store = memoryStore();
  let release;
  const pendingPost = new Promise((resolve) => { release = resolve; });
  const coordinator = createRecycleClearTaskCoordinator({
    ...store,
    apiPost: async (endpoint) => {
      assert.equal(endpoint.endsWith('/clear_recycle_bin'), true);
      const persisted = JSON.parse(store.value());
      assert.equal(persisted.scopes.default.phase, 'unknown');
      await pendingPost;
      return { code: 0, data: { taskId: 'crash-safe-task' } };
    },
  });
  const operation = coordinator.clearRecycleBin();
  await Promise.resolve();
  assert.equal(JSON.parse(store.value()).scopes.default.phase, 'unknown');
  release();
  await operation;
});

test('opaque token refresh retaining its session scope never submits a second clear task', async () => {
  const store = memoryStore();
  let scope = 'persisted-session';
  let posts = 0;
  const coordinator = createRecycleClearTaskCoordinator({
    ...store,
    scope: () => scope,
    deadlineMs: 2,
    pollMs: 1,
    sleep: async () => {},
    apiPost: async (endpoint) => {
      if (endpoint.endsWith('/clear_recycle_bin')) {
        posts += 1;
        scope = 'persisted-session'; // opaque token changed, session scope did not
        return { code: 0, data: { taskId: 'one-task' } };
      }
      return { code: 0, data: { status: 1 } };
    },
  });
  const first = await coordinator.clearRecycleBin();
  const second = await coordinator.clearRecycleBin();
  assert.equal(first.taskId, 'one-task');
  assert.equal(second.taskId, 'one-task');
  assert.equal(posts, 1);
});

test('explicit failed task is terminal, clears persisted state, and invalidates caches', async () => {
  const store = memoryStore(JSON.stringify({
    version: 1,
    scopes: {
      default: { phase: 'pending', scope: 'default', taskId: 'failed-clear', startedAt: 10, updatedAt: 10 },
    },
  }));
  const terminals = [];
  let submissions = 0;
  const coordinator = createRecycleClearTaskCoordinator({
    ...store,
    apiPost: async (endpoint) => {
      if (endpoint.endsWith('/clear_recycle_bin')) submissions += 1;
      return { code: 0, data: { status: 3, detail: { code: 9001, msg: '云端清空失败' } } };
    },
    onTerminal: (entry) => terminals.push(entry),
  });

  await assert.rejects(
    coordinator.clearRecycleBin(),
    (error) => error instanceof RecycleClearTaskFailedError
      && error.message === '云端清空失败'
      && error.taskId === 'failed-clear',
  );
  assert.equal(submissions, 0);
  assert.equal(store.value(), null);
  assert.deepEqual(terminals, [{ outcome: 'failed', taskId: 'failed-clear', scope: 'default' }]);
});

test('a different account neither polls nor gets blocked by another account task', async () => {
  const store = memoryStore(JSON.stringify({
    version: 1,
    scopes: {
      accountA: { phase: 'pending', scope: 'accountA', taskId: 'task-a', startedAt: 10, updatedAt: 10 },
    },
  }));
  let activeScope = 'accountB';
  const calls = [];
  const coordinator = createRecycleClearTaskCoordinator({
    ...store,
    scope: () => activeScope,
    apiPost: async (endpoint, body) => {
      calls.push({ activeScope, endpoint, body });
      if (endpoint.endsWith('/clear_recycle_bin')) return { code: 0, data: { taskId: 'task-b' } };
      return { code: 0, data: { status: 2, detail: { code: 0 } } };
    },
  });

  const accountB = await coordinator.clearRecycleBin();
  assert.equal(accountB.taskId, 'task-b');
  assert.equal(calls.some((entry) => entry.activeScope === 'accountB' && entry.body.taskId === 'task-a'), false);
  assert.equal(JSON.parse(store.value()).scopes.accountA.taskId, 'task-a');

  activeScope = 'accountA';
  const accountA = await coordinator.clearRecycleBin();
  assert.equal(accountA.taskId, 'task-a');
  assert.equal(calls.filter((entry) => entry.endpoint.endsWith('/clear_recycle_bin')).length, 1);
  assert.equal(store.value(), null);
});

test('a definitive rejected submission is not recorded as an accepted or unknown task', async () => {
  const store = memoryStore();
  const coordinator = createRecycleClearTaskCoordinator({
    ...store,
    apiPost: async () => {
      const error = new Error('请求参数无效');
      error.httpStatus = 400;
      error.retryable = false;
      throw error;
    },
  });
  await assert.rejects(coordinator.clearRecycleBin(), /请求参数无效/);
  assert.equal(store.value(), null);
});
