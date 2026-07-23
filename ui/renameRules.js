function splitFileName(name, preserveExtension, isFolder) {
  if (!preserveExtension || isFolder) return { stem: name, extension: '' };
  const index = name.lastIndexOf('.');
  if (index <= 0) return { stem: name, extension: '' };
  return { stem: name.slice(0, index), extension: name.slice(index) };
}

function replaceLiteral(value, search, replacement, ignoreCase) {
  if (!search) return value;
  if (!ignoreCase) return value.split(search).join(replacement);
  const escaped = search.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  return value.replace(new RegExp(escaped, 'gi'), replacement);
}

export function applyRenameRules(item, rules, index, preserveExtension = true) {
  const original = String(item.fileName || item.name || '');
  const { stem, extension } = splitFileName(original, preserveExtension, Number(item.resType) === 2);
  let value = stem;
  for (const rule of rules) {
    if (rule.type === 'set') value = String(rule.value || '');
    else if (rule.type === 'replace') value = replaceLiteral(value, String(rule.search || ''), String(rule.replacement || ''), Boolean(rule.ignoreCase));
    else if (rule.type === 'regex') {
      if (!rule.search) continue;
      value = value.replace(new RegExp(rule.search, rule.ignoreCase ? 'gi' : 'g'), String(rule.replacement || ''));
    } else if (rule.type === 'prefix') value = `${rule.value || ''}${value}`;
    else if (rule.type === 'suffix') value = `${value}${rule.value || ''}`;
    else if (rule.type === 'sequence') {
      const start = Number.isFinite(Number(rule.start)) ? Number(rule.start) : 1;
      const padding = Math.max(1, Math.min(12, Number(rule.padding) || 1));
      const number = String(start + index).padStart(padding, '0');
      const template = String(rule.value || '_{n}');
      value = template.includes('{n}') ? `${value}${template.replaceAll('{n}', number)}` : `${value}${template}${number}`;
    } else if (rule.type === 'upper') value = value.toUpperCase();
    else if (rule.type === 'lower') value = value.toLowerCase();
  }
  return `${value}${extension}`;
}

export function buildRenamePreview(items, rules, preserveExtension = true) {
  try {
    const rows = items.map((item, index) => ({
      fileId: String(item.fileId || item.id || ''),
      currentName: String(item.fileName || item.name || ''),
      newName: applyRenameRules(item, rules, index, preserveExtension),
    }));
    const seen = new Set();
    for (const row of rows) {
      if (!row.newName.trim()) throw new Error('重命名结果不能为空');
      if (/[\\/:*?"<>|]/.test(row.newName)) throw new Error(`文件名包含非法字符：${row.newName}`);
      const key = row.newName.toLocaleLowerCase();
      if (seen.has(key)) throw new Error(`存在重复目标名称：${row.newName}`);
      seen.add(key);
    }
    return { rows, error: '' };
  } catch (error) {
    return { rows: [], error: error instanceof Error ? error.message : String(error) };
  }
}
