import assert from 'node:assert/strict';
import test from 'node:test';
import { MAX_FOLDER_NAME_LENGTH, normalizeFolderName, validateFolderName } from './folderName.js';

test('文件夹名称会去除首尾空白并统一 Unicode 形式', () => {
  assert.equal(normalizeFolderName('  Cafe\u0301  '), 'Caf\u00e9');
});

test('拒绝空名称、路径分隔符和特殊目录名', () => {
  assert.match(validateFolderName('   '), /请输入/);
  assert.match(validateFolderName('../照片'), /不能包含/);
  assert.match(validateFolderName('照片\\2026'), /不能包含/);
  assert.match(validateFolderName('照片:2026'), /不能包含/);
  assert.match(validateFolderName('照片?'), /不能包含/);
  assert.match(validateFolderName('..'), /不能是/);
});

test('拒绝控制字符和超长名称', () => {
  assert.match(validateFolderName('照片\n备份'), /控制字符/);
  assert.match(validateFolderName(`照片\u0085备份`), /控制字符/);
  assert.match(validateFolderName('a'.repeat(MAX_FOLDER_NAME_LENGTH + 1)), /不能超过/);
  assert.equal(validateFolderName('a'.repeat(MAX_FOLDER_NAME_LENGTH)), '');
  assert.match(validateFolderName('😀'.repeat(MAX_FOLDER_NAME_LENGTH + 1)), /不能超过/);
  assert.equal(validateFolderName('😀'.repeat(MAX_FOLDER_NAME_LENGTH)), '');
});

test('拒绝当前目录下 Unicode 等价的同名项目', () => {
  assert.match(validateFolderName('Caf\u00e9', ['Cafe\u0301']), /同名/);
  assert.equal(validateFolderName('CAF\u00c9', ['Caf\u00e9']), '');
});
