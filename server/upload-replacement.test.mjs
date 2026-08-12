import assert from 'node:assert/strict';
import test from 'node:test';
import {
  createUploadReplacementContext,
  restorePreviousUploadRecord,
  safelyReplaceUploadedFile,
  uploadRemoteName,
} from './upload-replacement.mjs';

function harness(initial) {
  const entries = initial.map((entry) => ({ ...entry }));
  const calls = [];
  return {
    entries,
    calls,
    listEntries: async () => entries.map((entry) => ({ ...entry })),
    renameEntry: async (id, name) => {
      calls.push(['rename', id, name]);
      const entry = entries.find((candidate) => candidate.id === id);
      if (!entry) throw new Error('missing');
      if (entries.some((candidate) => candidate.id !== id && candidate.name === name)) throw new Error('conflict');
      entry.name = name;
    },
    deleteEntry: async (id) => {
      calls.push(['delete', id]);
      const index = entries.findIndex((candidate) => candidate.id === id);
      if (index >= 0) entries.splice(index, 1);
    },
  };
}

test('uploads to a unique name then promotes and removes the old version', async () => {
  const replacement = createUploadReplacementContext({ oldFileId: 'old', originalName: 'movie.mkv', previousSize: 10, previousModifiedMs: 20, randomUUID: () => 'abc' });
  const io = harness([{ id: 'old', name: 'movie.mkv' }, { id: 'new', name: replacement.temporaryName }]);
  assert.equal(uploadRemoteName({ file_path: 'C:/watch/movie.mkv', replacement }), replacement.temporaryName);
  await safelyReplaceUploadedFile({ replacement, newFileId: 'new', ...io });
  assert.deepEqual(io.entries, [{ id: 'new', name: 'movie.mkv' }]);
  assert.deepEqual(io.calls, [
    ['rename', 'old', replacement.backupName],
    ['rename', 'new', 'movie.mkv'],
    ['delete', 'old'],
  ]);
});

test('resumes after a crash between staging and promotion', async () => {
  const replacement = createUploadReplacementContext({ oldFileId: 'old', originalName: 'movie.mkv', randomUUID: () => 'abc' });
  const io = harness([{ id: 'old', name: replacement.backupName }, { id: 'new', name: replacement.temporaryName }]);
  await safelyReplaceUploadedFile({ replacement, newFileId: 'new', ...io });
  assert.deepEqual(io.entries, [{ id: 'new', name: 'movie.mkv' }]);
});

test('does not overwrite a different cloud file that took the original name', async () => {
  const replacement = createUploadReplacementContext({ oldFileId: 'old', originalName: 'movie.mkv', randomUUID: () => 'abc' });
  const io = harness([{ id: 'old', name: replacement.backupName }, { id: 'new', name: replacement.temporaryName }, { id: 'external', name: 'movie.mkv' }]);
  await assert.rejects(
    safelyReplaceUploadedFile({ replacement, newFileId: 'new', ...io }),
    /其他文件占用/,
  );
  assert.equal(io.entries.find((entry) => entry.id === 'external').name, 'movie.mkv');
  assert.equal(io.entries.find((entry) => entry.id === 'new').name, replacement.temporaryName);
});

test('rolls the old name back when promotion fails', async () => {
  const replacement = createUploadReplacementContext({ oldFileId: 'old', originalName: 'movie.mkv', randomUUID: () => 'abc' });
  const io = harness([{ id: 'old', name: 'movie.mkv' }, { id: 'new', name: replacement.temporaryName }]);
  const rename = io.renameEntry;
  io.renameEntry = async (id, name) => {
    if (id === 'new') throw new Error('promotion failed');
    return rename(id, name);
  };
  await assert.rejects(safelyReplaceUploadedFile({ replacement, newFileId: 'new', ...io }), /promotion failed/);
  assert.equal(io.entries.find((entry) => entry.id === 'old').name, 'movie.mkv');
});

test('restores the previous confirmed record when a pending replacement is cancelled', () => {
  const replacement = createUploadReplacementContext({ oldFileId: 'old', originalName: 'movie.mkv', previousSize: 10, previousModifiedMs: 20, randomUUID: () => 'abc' });
  assert.deepEqual(restorePreviousUploadRecord({ item_json: JSON.stringify({ replacement }) }), {
    remoteFileId: 'old', size: 10, modifiedMs: '20',
  });
});
