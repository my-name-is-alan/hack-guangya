import crypto from 'node:crypto';
import { spawnSync } from 'node:child_process';
import fsp from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const version = '1.74.4';
const archiveSha256 = Object.freeze({
  'rclone-v1.74.4-linux-386.zip': '7feee086d7ff72652c5a91ef4b4a576941ccd33b2929772a2d70471904e516f0',
  'rclone-v1.74.4-linux-amd64.zip': 'fe435e0c36228e7c2f116a8701f01127bb1f694005fc11d1f27186c8bca4115d',
  'rclone-v1.74.4-linux-arm64.zip': '97685285c9ad6a0cf17d5844115d2a67245af6444db672187074bd9c358de419',
  'rclone-v1.74.4-osx-amd64.zip': '4188aa84043d7a6240912923f47639a9d2da21f3b40a521c065c8d92e66563f6',
  'rclone-v1.74.4-osx-arm64.zip': 'c2100e2d4a4b3be04c55cd45380cafe7647e1ad772bb055f52f00876ed701167',
  'rclone-v1.74.4-windows-386.zip': '006c5d3e9fe992ed47a2c34806d7a2262e392e2b64e009ccaf965350a07b8109',
  'rclone-v1.74.4-windows-amd64.zip': 'ef097ef9de37a57feb7d9f9c7afb34148ad3c65be8025f1d8f7f521554a701ea',
  'rclone-v1.74.4-windows-arm64.zip': '72194ad0aaf210d7a55808801191fecc7e175444dab7be7491b7a63074521f3a',
});
const licenseFileName = `rclone-v${version}-COPYING.txt`;
const licenseSha256 = '8cd2e9e750b90a04b7d82dbbca3930c696ae0309d7c10464f90a44f45754cd04';
const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(scriptDir, '..');
const outputDir = path.join(repoRoot, 'src-tauri', 'resources');
const licensePath = path.join(outputDir, 'rclone-COPYING.txt');
const offline = /^(?:1|true|yes|on)$/i.test(String(process.env.GUANGYA_RCLONE_OFFLINE || ''));
const archiveDirectory = process.env.GUANGYA_RCLONE_ARCHIVE_DIR
  ? path.resolve(process.env.GUANGYA_RCLONE_ARCHIVE_DIR)
  : '';
const signingIdentity = process.platform === 'darwin'
  ? String(process.env.APPLE_SIGNING_IDENTITY || '').trim()
  : '';
const signingIdentitySha256 = signingIdentity ? sha256Buffer(Buffer.from(signingIdentity)) : '';

function run(command, args) {
  const result = spawnSync(command, args, { cwd: repoRoot, stdio: 'inherit', windowsHide: true });
  if (result.status !== 0) throw new Error(`${command} 执行失败，退出码 ${result.status}`);
}

function signPreparedBinary(binaryPath) {
  if (!signingIdentity) return;
  const args = ['--force', '--options', 'runtime'];
  args.push(signingIdentity === '-' ? '--timestamp=none' : '--timestamp');
  args.push('--sign', signingIdentity, binaryPath);
  run('codesign', args);
  run('codesign', ['--verify', '--strict', binaryPath]);
}

function hostTriple() {
  const result = spawnSync('rustc', ['-vV'], { encoding: 'utf8', windowsHide: true });
  const match = String(result.stdout || '').match(/^host:\s*(.+)$/m);
  if (!match) throw new Error('无法通过 rustc -vV 识别构建目标');
  return match[1].trim();
}

function targetDescriptor(triple) {
  const arch = triple.startsWith('aarch64') ? 'arm64'
    : triple.startsWith('x86_64') ? 'amd64'
      : triple.startsWith('i686') ? '386'
        : triple.startsWith('universal') ? 'universal'
          : null;
  if (!arch) throw new Error(`暂不支持的 rclone 架构：${triple}`);
  if (triple.includes('windows')) return { osName: 'windows', arch, extension: '.exe' };
  if (triple.includes('apple-darwin')) return { osName: 'osx', arch, extension: '' };
  if (triple.includes('linux')) return { osName: 'linux', arch, extension: '' };
  throw new Error(`暂不支持为 ${triple} 准备 rclone`);
}

function archiveMetadata(osName, arch) {
  const name = `rclone-v${version}-${osName}-${arch}.zip`;
  const sha256 = archiveSha256[name];
  if (!sha256) throw new Error(`没有为 ${name} 固定 SHA-256，已拒绝下载`);
  return { name, sha256 };
}

function expectedArchives(descriptor) {
  const architectures = descriptor.arch === 'universal' ? ['amd64', 'arm64'] : [descriptor.arch];
  return Object.fromEntries(architectures.map((arch) => {
    const metadata = archiveMetadata(descriptor.osName, arch);
    return [metadata.name, metadata.sha256];
  }));
}

function sha256Buffer(value) {
  return crypto.createHash('sha256').update(value).digest('hex');
}

async function sha256File(filePath) {
  return sha256Buffer(await fsp.readFile(filePath));
}

async function download(url) {
  console.log(`Downloading ${url}`);
  const response = await fetch(url, { signal: AbortSignal.timeout(300_000), redirect: 'error' });
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

async function loadArchive(metadata) {
  let archive;
  if (archiveDirectory) {
    const localPath = path.join(archiveDirectory, metadata.name);
    try {
      archive = await fsp.readFile(localPath);
      console.log(`Using local rclone archive: ${localPath}`);
    } catch (error) {
      if (offline) throw new Error(`离线模式找不到 ${localPath}：${error.message}`);
    }
  }
  if (!archive) {
    if (offline) {
      throw new Error(`离线模式没有可用的 ${metadata.name}；请保留已校验资源，或通过 GUANGYA_RCLONE_ARCHIVE_DIR 提供归档`);
    }
    archive = await download(`https://downloads.rclone.org/v${version}/${metadata.name}`);
  }
  const actual = sha256Buffer(archive);
  if (actual !== metadata.sha256) {
    throw new Error(`${metadata.name} SHA256 校验失败：${actual} != ${metadata.sha256}`);
  }
  return archive;
}

async function loadLicense() {
  let license;
  if (archiveDirectory) {
    const localPath = path.join(archiveDirectory, licenseFileName);
    try {
      license = await fsp.readFile(localPath);
      console.log(`Using local rclone license: ${localPath}`);
    } catch (error) {
      if (offline) throw new Error(`离线模式找不到 ${localPath}：${error.message}`);
    }
  }
  if (!license) {
    if (offline) {
      throw new Error(`离线模式没有可用的 ${licenseFileName}；请通过 GUANGYA_RCLONE_ARCHIVE_DIR 同时提供归档和许可证`);
    }
    license = await download(`https://raw.githubusercontent.com/rclone/rclone/v${version}/COPYING`);
  }
  const actual = sha256Buffer(license);
  if (actual !== licenseSha256) {
    throw new Error(`${licenseFileName} SHA256 校验失败：${actual} != ${licenseSha256}`);
  }
  return license;
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
  const metadata = archiveMetadata(osName, arch);
  const archive = await loadArchive(metadata);
  const archivePath = path.join(tempRoot, metadata.name);
  const extractDir = path.join(tempRoot, `${osName}-${arch}`);
  await fsp.writeFile(archivePath, archive, { mode: 0o600 });
  await fsp.mkdir(extractDir, { recursive: true });
  if (process.platform === 'win32') run('tar.exe', ['-xf', archivePath, '-C', extractDir]);
  else run('unzip', ['-q', archivePath, '-d', extractDir]);
  const executableName = osName === 'windows' ? 'rclone.exe' : 'rclone';
  const executable = await findFile(extractDir, executableName);
  if (!executable) throw new Error(`${metadata.name} 中没有找到 ${executableName}`);
  return { executable };
}

async function replaceAtomic(source, target, mode) {
  await fsp.mkdir(path.dirname(target), { recursive: true });
  const temporary = path.join(path.dirname(target), `.${path.basename(target)}.${process.pid}.${crypto.randomUUID()}.tmp`);
  try {
    await fsp.copyFile(source, temporary);
    if (mode !== undefined) await fsp.chmod(temporary, mode);
    await fsp.rename(temporary, target);
  } finally {
    await fsp.rm(temporary, { force: true });
  }
}

async function writeAtomic(target, content, mode = 0o600) {
  await fsp.mkdir(path.dirname(target), { recursive: true });
  const temporary = path.join(path.dirname(target), `.${path.basename(target)}.${process.pid}.${crypto.randomUUID()}.tmp`);
  try {
    await fsp.writeFile(temporary, content, { mode });
    await fsp.rename(temporary, target);
  } finally {
    await fsp.rm(temporary, { force: true });
  }
}

async function preparedResourcesAreValid(markerPath, outputPath, triple, archives) {
  try {
    const marker = JSON.parse(await fsp.readFile(markerPath, 'utf8'));
    if (marker.version !== version || marker.triple !== triple) return false;
    if (JSON.stringify(marker.archive_sha256) !== JSON.stringify(archives)) return false;
    if (marker.license_source_sha256 !== licenseSha256) return false;
    if (marker.signing_identity_sha256 !== signingIdentitySha256) return false;
    const [outputStat, licenseStat, outputHash, licenseHash] = await Promise.all([
      fsp.stat(outputPath),
      fsp.stat(licensePath),
      sha256File(outputPath),
      sha256File(licensePath),
    ]);
    return outputStat.isFile()
      && licenseStat.isFile()
      && outputHash === marker.output_sha256
      && licenseHash === marker.license_sha256;
  } catch {
    return false;
  }
}

const triple = process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
const descriptor = targetDescriptor(triple);
if (descriptor.arch === 'universal' && descriptor.osName !== 'osx') {
  throw new Error(`universal 资源只支持 macOS：${triple}`);
}
const outputPath = path.join(outputDir, `rclone${descriptor.extension}`);
const markerPath = path.join(outputDir, 'rclone.version.json');
const archives = expectedArchives(descriptor);

if (await preparedResourcesAreValid(markerPath, outputPath, triple, archives)) {
  console.log(`rclone v${version} ready: ${outputPath}`);
} else {
  if (offline && !archiveDirectory) {
    throw new Error('现有 rclone 资源校验失败，且已启用 GUANGYA_RCLONE_OFFLINE；请提供已固定 SHA-256 的本地归档目录');
  }
  const tempRoot = await fsp.mkdtemp(path.join(os.tmpdir(), 'guangya-rclone-'));
  try {
    const license = await loadLicense();
    let prepared;
    if (descriptor.arch === 'universal') {
      const [amd64, arm64] = await Promise.all([
        fetchAndExtract(tempRoot, descriptor.osName, 'amd64'),
        fetchAndExtract(tempRoot, descriptor.osName, 'arm64'),
      ]);
      const universalExecutable = path.join(tempRoot, 'rclone-universal');
      run('lipo', ['-create', amd64.executable, arm64.executable, '-output', universalExecutable]);
      prepared = { executable: universalExecutable };
    } else {
      prepared = await fetchAndExtract(tempRoot, descriptor.osName, descriptor.arch);
    }
    await replaceAtomic(prepared.executable, outputPath, process.platform === 'win32' ? undefined : 0o755);
    signPreparedBinary(outputPath);
    await writeAtomic(licensePath, license, 0o644);
    const marker = {
      version,
      triple,
      archive_sha256: archives,
      license_source_sha256: licenseSha256,
      signing_identity_sha256: signingIdentitySha256,
      output_sha256: await sha256File(outputPath),
      license_sha256: await sha256File(licensePath),
    };
    await writeAtomic(markerPath, `${JSON.stringify(marker, null, 2)}\n`, 0o644);
    console.log(`rclone v${version} prepared: ${outputPath}`);
  } finally {
    await fsp.rm(tempRoot, { recursive: true, force: true });
  }
}
