import { spawn } from 'node:child_process';
import { once } from 'node:events';
import fsp from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));

export async function freePort() {
  const server = http.createServer();
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const { port } = server.address();
  await new Promise((resolve) => server.close(resolve));
  return port;
}

export async function waitUntil(check, timeout = 10_000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const value = await check();
    if (value) return value;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error('等待测试条件超时');
}

export async function startTestServer(root, extraEnv = {}) {
  const watchRoot = path.join(root, 'watch');
  const archiveRoot = path.join(root, 'archive');
  const dataDir = path.join(root, 'data');
  await Promise.all([
    fsp.mkdir(watchRoot, { recursive: true }),
    fsp.mkdir(archiveRoot, { recursive: true }),
    fsp.mkdir(dataDir, { recursive: true }),
  ]);
  const port = await freePort();
  const child = spawn(process.execPath, [path.join(here, 'server.mjs')], {
    cwd: path.resolve(here, '..'),
    env: {
      ...process.env,
      PORT: String(port),
      DATA_DIR: dataDir,
      GUANGYA_WATCH_ROOT: watchRoot,
      GUANGYA_ARCHIVE_ROOT: archiveRoot,
      GUANGYA_FILE_ROOTS: watchRoot,
      GUANGYA_ADMIN_PASSWORD: '',
      LISTEN_HOST: '127.0.0.1',
      ...extraEnv,
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  let output = '';
  child.stdout.on('data', (chunk) => { output += chunk; });
  child.stderr.on('data', (chunk) => { output += chunk; });
  await waitUntil(() => {
    if (output.includes('Guangya Web listening')) return true;
    if (child.exitCode != null) throw new Error(`测试服务器提前退出：\n${output}`);
    return false;
  });
  return { child, port, dataDir, watchRoot, archiveRoot, output: () => output };
}

export async function stopTestServer(instance) {
  if (!instance?.child || instance.child.exitCode != null) return;
  instance.child.kill();
  await Promise.race([
    once(instance.child, 'exit'),
    new Promise((resolve) => setTimeout(resolve, 2_000)),
  ]);
}
