import { readdirSync, readFileSync } from 'node:fs';

// Rust 后端已从单一 main.rs 拆分为 src-tauri/src 下的多个功能模块。
// 契约测试需要针对"完整后端源码"断言，这里按文件名排序拼接，
// 保证单个模块内部的代码顺序不变（跨模块顺序对断言无意义）。
export function readRustBackendSourceSync() {
  const dir = new URL('../src-tauri/src/', import.meta.url);
  return readdirSync(dir)
    .filter((name) => name.endsWith('.rs'))
    .sort()
    .map((name) => readFileSync(new URL(name, dir), 'utf8'))
    .join('\n');
}

export async function readRustBackendSource() {
  return readRustBackendSourceSync();
}
