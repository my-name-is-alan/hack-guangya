import { spawnSync } from 'node:child_process';
import crypto from 'node:crypto';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, '..');
const releaseDir = path.join(repoRoot, 'release');
const metadata = JSON.parse(await fsp.readFile(path.join(repoRoot, 'package.json'), 'utf8'));
const bundleName = `guangya-sync-native-ubuntu-x64-${metadata.version}`;
const archiveName = `${bundleName}.tar.gz`;
const checksumName = `${archiveName}.sha256`;
const imageTag = `hack-guangya-ubuntu-native-package:${metadata.version}`;
const containerName = `guangya-native-export-${crypto.randomUUID()}`;

await fsp.mkdir(releaseDir, { recursive: true });
for (const name of [archiveName, checksumName]) {
  const target = path.resolve(releaseDir, name);
  if (!target.startsWith(`${releaseDir}${path.sep}`)) throw new Error('发布文件超出 release 目录');
  await fsp.rm(target, { force: true });
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: repoRoot, stdio: 'inherit', ...options });
  if (result.status !== 0) throw new Error(`${command} 执行失败，退出码 ${result.status}`);
}

run('docker', ['build', '-f', 'Dockerfile.ubuntu-native', '--build-arg', `VERSION=${metadata.version}`, '-t', imageTag, '.']);
try {
  run('docker', ['create', '--name', containerName, imageTag, '/package-export']);
  run('docker', ['cp', `${containerName}:/${archiveName}`, path.join(releaseDir, archiveName)]);
  run('docker', ['cp', `${containerName}:/${checksumName}`, path.join(releaseDir, checksumName)]);
} finally {
  spawnSync('docker', ['rm', '-f', containerName], { cwd: repoRoot, stdio: 'ignore' });
}

const digest = crypto.createHash('sha256').update(await fsp.readFile(path.join(releaseDir, archiveName))).digest('hex');
const recorded = (await fsp.readFile(path.join(releaseDir, checksumName), 'utf8')).trim().split(/\s+/)[0];
if (digest !== recorded) throw new Error(`发布包哈希不一致：${digest} != ${recorded}`);

console.log(`Ubuntu native bundle: ${path.join(releaseDir, archiveName)}`);
console.log(`SHA256: ${digest}`);
