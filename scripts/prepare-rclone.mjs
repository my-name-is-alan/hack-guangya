import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const version = '1.74.4';
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, '..');
const outputDir = path.join(repoRoot, 'src-tauri', 'resources');
const licensePath = path.join(outputDir, 'rclone-COPYING.txt');

function run(command, args) {
  const result = spawnSync(command, args, { cwd: repoRoot, stdio: 'inherit', windowsHide: true });
  if (result.status !== 0) throw new Error(`${command} 执行失败，退出码 ${result.status}`);
}

function hostTriple() {
  const result = spawnSync('rustc', ['-vV'], { encoding: 'utf8', windowsHide: true });
  const match = String(result.stdout || '').match(/^host:\s*(.+)$/m);
  if (!match) throw new Error('无法通过 rustc -vV 识别构建目标');
  return match[1].trim();
}

function targetDescriptor(triple) {
  const arch = triple.startsWith('aarch64') ? 'arm64'
    : triple.startsWith('i686') ? '386'
      : triple.startsWith('universal') ? 'universal'
        : 'amd64';
  if (triple.includes('windows')) return { osName: 'windows', arch, extension: '.exe' };
  if (triple.includes('apple-darwin')) return { osName: 'osx', arch, extension: '' };
  if (triple.includes('linux')) return { osName: 'linux', arch, extension: '' };
  throw new Error(`暂不支持为 ${triple} 准备 rclone`);
}

async function download(url) {
  console.log(`Downloading ${url}`);
  const response = await fetch(url, { signal: AbortSignal.timeout(300_000) });
  if (!response.ok) throw new Error(`下载失败（HTTP ${response.status}）：${url}`);
  const chunks = [];
  let received = 0;
  let nextProgress = 5 * 1024 * 1024;
  for await (const chunk of response.body) {
    const buffer = Buffer.from(chunk);
    chunks.push(buffer);
    received += buffer.length;
    if (received >= nextProgress) {
      console.log(`Downloaded ${(received / 1024 / 1024).toFixed(1)} MiB`);
      nextProgress += 5 * 1024 * 1024;
    }
  }
  return Buffer.concat(chunks);
}

async function findFile(root, fileName) {
  for (const entry of await fsp.readdir(root, { withFileTypes: true })) {
    const target = path.join(root, entry.name);
    if (entry.isDirectory()) {
      const nested = await findFile(target, fileName);
      if (nested) return nested;
    } else if (entry.name === fileName) {
      return target;
    }
  }
  return null;
}

async function fetchAndExtract(tempRoot, osName, arch) {
  const archiveName = `rclone-v${version}-${osName}-${arch}.zip`;
  const baseUrl = `https://downloads.rclone.org/v${version}`;
  const [archive, checksums] = await Promise.all([
    download(`${baseUrl}/${archiveName}`),
    download(`${baseUrl}/SHA256SUMS`),
  ]);
  const expected = checksums.toString('utf8')
    .split(/\r?\n/)
    .map((line) => line.trim().split(/\s+/))
    .find((parts) => parts.at(-1) === archiveName)?.[0];
  if (!expected) throw new Error(`SHA256SUMS 中没有 ${archiveName}`);
  const actual = crypto.createHash('sha256').update(archive).digest('hex');
  if (actual.toLowerCase() !== expected.toLowerCase()) throw new Error(`${archiveName} SHA256 校验失败`);

  const archivePath = path.join(tempRoot, archiveName);
  const extractDir = path.join(tempRoot, `${osName}-${arch}`);
  await fsp.writeFile(archivePath, archive);
  await fsp.mkdir(extractDir, { recursive: true });
  if (process.platform === 'win32') {
    run('tar.exe', ['-xf', archivePath, '-C', extractDir]);
  } else {
    run('unzip', ['-q', archivePath, '-d', extractDir]);
  }
  const executableName = osName === 'windows' ? 'rclone.exe' : 'rclone';
  const executable = await findFile(extractDir, executableName);
  if (!executable) throw new Error(`${archiveName} 中没有找到 ${executableName}`);
  const copying = await findFile(extractDir, 'COPYING');
  return { executable, copying };
}

const triple = process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
const descriptor = targetDescriptor(triple);
const outputPath = path.join(outputDir, `rclone${descriptor.extension}`);
const markerPath = path.join(outputDir, 'rclone.version.json');
let alreadyPrepared = false;
try {
  const marker = JSON.parse(await fsp.readFile(markerPath, 'utf8'));
  if (marker.version === version && marker.triple === triple && (await fsp.stat(outputPath)).isFile()) {
    try {
      await fsp.access(licensePath);
    } catch {
      await fsp.writeFile(
        licensePath,
        await download(`https://raw.githubusercontent.com/rclone/rclone/v${version}/COPYING`),
      );
    }
    console.log(`rclone v${version} ready: ${outputPath}`);
    alreadyPrepared = true;
  }
} catch {}

if (!alreadyPrepared) {
  const tempRoot = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-rclone-'));
  try {
    await fsp.mkdir(outputDir, { recursive: true });
    let prepared;
    if (descriptor.arch === 'universal') {
      const [amd64, arm64] = await Promise.all([
        fetchAndExtract(tempRoot, descriptor.osName, 'amd64'),
        fetchAndExtract(tempRoot, descriptor.osName, 'arm64'),
      ]);
      run('lipo', ['-create', amd64.executable, arm64.executable, '-output', outputPath]);
      prepared = amd64;
    } else {
      prepared = await fetchAndExtract(tempRoot, descriptor.osName, descriptor.arch);
      await fsp.copyFile(prepared.executable, outputPath);
    }
    if (process.platform !== 'win32') await fsp.chmod(outputPath, 0o755);
    if (prepared.copying) {
      await fsp.copyFile(prepared.copying, licensePath);
    } else {
      await fsp.writeFile(
        licensePath,
        await download(`https://raw.githubusercontent.com/rclone/rclone/v${version}/COPYING`),
      );
    }
    await fsp.writeFile(markerPath, `${JSON.stringify({ version, triple }, null, 2)}\n`);
    console.log(`rclone v${version} prepared: ${outputPath}`);
  } finally {
    await fsp.rm(tempRoot, { recursive: true, force: true });
  }
}
