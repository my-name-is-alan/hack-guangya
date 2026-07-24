export const GCID_IMPORT_PASTE_FILE_THRESHOLD = 512 * 1024;

export function shouldConvertPasteToFile(value) {
  return typeof value === 'string' && new Blob([value]).size >= GCID_IMPORT_PASTE_FILE_THRESHOLD;
}

export function gcidImportFinished(counts = {}) {
  return ['imported', 'existing', 'missed', 'conflict', 'failed']
    .reduce((total, key) => total + Number(counts[key] || 0), 0);
}

export function gcidImportPercent(status) {
  const total = Number(status?.total_files || 0);
  if (!total) return 0;
  return Math.min(100, Math.round(gcidImportFinished(status.counts) / total * 100));
}
