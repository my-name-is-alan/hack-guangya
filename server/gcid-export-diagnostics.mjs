import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

const URL_PATTERN = /https?:\/\/[^\s"'<>]+/gi;
const SECRET_PATTERN = /\b(authorization|cookie|access[_-]?token|refresh[_-]?token|client[_-]?secret|signature|security[_-]?token|x-oss-security-token)\b\s*[:=]\s*([^\s,;]+)/gi;
const MAX_INFO_LOG_BYTES = 8 * 1024 * 1024;
const MAX_DETAIL_LOG_BYTES = 24 * 1024 * 1024;
const TERMINAL_EVENTS = new Set(['run_completed', 'run_failed']);

export function sanitizeGcidDiagnosticText(value) {
  return String(value ?? '')
    .replace(URL_PATTERN, (raw) => {
      try {
        const parsed = new URL(raw);
        return `${parsed.origin}${parsed.pathname}${parsed.search ? '?<redacted>' : ''}`;
      }
      catch {
        return '<redacted-url>';
      }
    })
    .replace(SECRET_PATTERN, '$1=<redacted>')
    .slice(0, 2_000);
}

function sanitizeValue(value) {
  if (typeof value === 'string') return sanitizeGcidDiagnosticText(value);
  if (typeof value === 'bigint') return value.toString();
  if (Array.isArray(value)) return value.map(sanitizeValue);
  if (value && typeof value === 'object') {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, sanitizeValue(item)]));
  }
  return value;
}

export function gcidDiagnosticLogPath(dataDir) {
  return path.join(dataDir, 'logs', 'gcid-export-latest.jsonl');
}

export function createGcidExportDiagnostics(filePath, runtime = 'docker-web') {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, '', { mode: 0o600 });
  const runId = crypto.randomUUID();
  const startedAt = Date.now();
  let writtenBytes = 0;
  let infoSuppressed = false;
  let detailsSuppressed = false;
  const append = (record) => {
    const line = `${JSON.stringify(sanitizeValue(record))}\n`;
    fs.appendFileSync(filePath, line, { encoding: 'utf8', mode: 0o600 });
    writtenBytes += Buffer.byteLength(line);
  };
  const write = (level, event, fields = {}) => {
    const base = {
      timestamp: new Date().toISOString(),
      elapsed_ms: Math.max(0, Date.now() - startedAt),
      run_id: runId,
      runtime,
    };
    if (!TERMINAL_EVENTS.has(event) && writtenBytes >= MAX_DETAIL_LOG_BYTES) {
      if (!detailsSuppressed) {
        detailsSuppressed = true;
        append({
          ...base,
          level: 'warn',
          event: 'detail_log_limit_reached',
          limit_bytes: MAX_DETAIL_LOG_BYTES,
          message: '诊断明细已达到大小上限，最终任务结果仍会写入',
        });
      }
      return;
    }
    if (level === 'info' && writtenBytes >= MAX_INFO_LOG_BYTES) {
      if (!infoSuppressed) {
        infoSuppressed = true;
        append({
          ...base,
          level: 'warn',
          event: 'info_log_limit_reached',
          limit_bytes: MAX_INFO_LOG_BYTES,
          message: '普通成功明细已停止记录，后续警告和错误仍会继续写入',
        });
      }
      return;
    }
    append({
      ...base,
      level,
      event,
      ...fields,
    });
  };
  return { filePath, runId, write };
}

export function readGcidExportDiagnosticLog(filePath) {
  if (!fs.existsSync(filePath)) throw new Error('还没有秒传 JSON 诊断日志，请先运行一次生成任务');
  const content = fs.readFileSync(filePath, 'utf8');
  if (!content.trim()) throw new Error('秒传 JSON 诊断日志为空，请重新运行一次生成任务');
  return {
    file_name: '光鸭秒传诊断日志.jsonl',
    content,
  };
}
