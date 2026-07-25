export const MAX_FOLDER_NAME_LENGTH = 255;

export function normalizeFolderName(value) {
  return String(value ?? '').trim().normalize('NFC');
}

export function validateFolderName(value, existingNames = []) {
  const name = normalizeFolderName(value);
  if (!name) return '请输入文件夹名称';
  if (name === '.' || name === '..') return '文件夹名称不能是 . 或 ..';
  if (/[\\/:*?"<>|]/.test(name)) return '文件夹名称不能包含 \\/:*?"<>|';
  if (/[\u0000-\u001f\u007f-\u009f]/.test(name)) return '文件夹名称不能包含控制字符';
  if ([...name].length > MAX_FOLDER_NAME_LENGTH) return `文件夹名称不能超过 ${MAX_FOLDER_NAME_LENGTH} 个字符`;

  const normalizedExistingNames = existingNames.map(normalizeFolderName);
  if (normalizedExistingNames.includes(name)) return '当前目录已存在同名项目';
  return '';
}
