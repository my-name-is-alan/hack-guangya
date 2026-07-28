import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { access, readFile } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const readRepoFile = (relativePath) => readFile(path.join(repoRoot, relativePath), 'utf8');

async function exists(relativePath) {
  try {
    await access(path.join(repoRoot, relativePath));
    return true;
  } catch {
    return false;
  }
}

async function localRuntimeImports(entryRelativePath) {
  const pending = [entryRelativePath];
  const visited = new Set();
  const imports = new Set();
  const importPattern = /(?:import|export)\s+(?:[^'"\n]*?\s+from\s+)?['"]([^'"]+)['"]/g;

  while (pending.length) {
    const current = pending.pop();
    if (visited.has(current)) continue;
    visited.add(current);
    const source = await readRepoFile(current);
    for (const match of source.matchAll(importPattern)) {
      if (!match[1].startsWith('.')) continue;
      const resolved = path.posix.normalize(path.posix.join(path.posix.dirname(current), match[1]));
      imports.add(resolved);
      if (/\.(?:m?js)$/.test(resolved)) pending.push(resolved);
    }
  }
  return imports;
}

test('Ubuntu package includes Vite assets and every server import outside server/', async () => {
  const dockerfile = await readRepoFile('Dockerfile.ubuntu-native');
  assert.match(dockerfile, /COPY src-tauri\/icons\/128x128\.png \.\/src-tauri\/icons\/128x128\.png/);

  const imports = await localRuntimeImports('server/server.mjs');
  const outsideServer = [...imports].filter((relativePath) => !relativePath.startsWith('server/'));
  assert.ok(outsideServer.length > 0, 'expected at least one server runtime import outside server/');
  for (const relativePath of outsideServer) {
    assert.equal(await exists(relativePath), true, `missing runtime import source: ${relativePath}`);
    const escaped = relativePath.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    assert.match(
      dockerfile,
      new RegExp(`COPY\\s+${escaped}\\s+\\./${escaped}`),
      `Dockerfile must copy external server import ${relativePath}`,
    );
    const topLevelDirectory = relativePath.split('/')[0];
    assert.match(
      dockerfile,
      new RegExp(`cp -a /build/${topLevelDirectory} "\\$\\{root\\}/app/${topLevelDirectory}"`),
      `runtime archive must include ${topLevelDirectory}/`,
    );
  }
});

test('Vite uses the TypeScript entry and legacy monolith entries stay deleted', async () => {
  const index = await readRepoFile('ui/index.html');
  assert.match(index, /<script type="module" src="\.\/main\.ts"><\/script>/);
  assert.equal(await exists('ui/main.js'), false);
  assert.equal(await exists('ui/App.vue'), false);
});

test('release metadata uses one project version without changing dependency versions', async () => {
  const packageMetadata = JSON.parse(await readRepoFile('package.json'));
  const tauriMetadata = JSON.parse(await readRepoFile('src-tauri/tauri.conf.json'));
  const runtimeMetadata = JSON.parse(await readRepoFile('packaging/ubuntu-native/runtime-package.json'));
  const cargoManifest = await readRepoFile('src-tauri/Cargo.toml');
  const cargoLock = await readRepoFile('Cargo.lock');
  const compose = await readRepoFile('docker-compose.yml');
  const version = packageMetadata.version;

  assert.equal(version, '0.1.16');
  assert.equal(tauriMetadata.version, version);
  assert.equal(runtimeMetadata.version, version);
  assert.match(cargoManifest, new RegExp(`\\[package\\][\\s\\S]*?^version = "${version}"`, 'm'));
  assert.match(cargoLock, new RegExp(`name = "guangya-folder-sync"\\nversion = "${version}"`));
  assert.match(compose, new RegExp(`guangya-sync:${version}`));
});

test('macOS local package explicitly seals the completed app with an ad-hoc identity', async () => {
  const script = await readRepoFile('scripts/package-macos.sh');
  assert.match(script, /export APPLE_SIGNING_IDENTITY="-"/);
  assert.match(script, /build_marker=.*mktemp[\s\S]*pnpm prepare:rclone[\s\S]*pnpm ui:build[\s\S]*pnpm tauri build/);
  assert.match(script, /candidate\/Contents\/Info\.plist" -nt "\$build_marker"/);
  assert.match(script, /"\$candidate" -nt "\$build_marker"/);
  assert.match(script, /"\$\{#fresh_apps\[@\]\}" -ne 1/);
  assert.match(script, /"\$\{#fresh_dmgs\[@\]\}" -ne 1/);
  assert.match(script, /app_path="\$\{fresh_apps\[0\]\}"/);
  assert.match(script, /dmg_path="\$\{fresh_dmgs\[0\]\}"/);
  assert.match(script, /find .*Contents\/Resources.*-name rclone/);
  assert.match(script, /find .*Contents\/Resources.*-name rclone-COPYING\.txt/);
  assert.match(script, /"\$embedded_rclone" version/);
  assert.match(script, /codesign --verify --deep --strict/);
  assert.match(script, /hdiutil verify/);
});

test('desktop main window uses a restrictive CSP instead of granting injected content native reach', async () => {
  const tauriMetadata = JSON.parse(await readRepoFile('src-tauri/tauri.conf.json'));
  const csp = tauriMetadata.app?.security?.csp;
  assert.equal(typeof csp, 'string');
  assert.match(csp, /default-src 'self'/);
  assert.match(csp, /script-src 'self'/);
  assert.match(csp, /object-src 'none'/);
  assert.match(csp, /frame-ancestors 'none'/);
  assert.doesNotMatch(csp, /script-src[^;]*'unsafe-inline'/);
  assert.doesNotMatch(csp, /script-src[^;]*'unsafe-eval'/);
  assert.match(csp, /connect-src 'self' ipc: http:\/\/ipc\.localhost ws:\/\/127\.0\.0\.1:1420/);
  assert.doesNotMatch(csp, /connect-src[^;]*https:/);
});

test('rclone desktop preparation is pinned, cache-verified, atomic and supports explicit offline builds', async () => {
  const script = await readRepoFile('scripts/prepare-rclone.mjs');
  assert.match(script, /archiveSha256 = Object\.freeze/);
  assert.match(script, /rclone-v1\.74\.4-(?:linux|osx|windows)-amd64\.zip/);
  assert.match(script, /outputHash === marker\.output_sha256/);
  assert.match(script, /licenseHash === marker\.license_sha256/);
  assert.match(script, /GUANGYA_RCLONE_OFFLINE/);
  assert.match(script, /GUANGYA_RCLONE_ARCHIVE_DIR/);
  assert.match(script, /fsp\.rename\(temporary, target\)/);
  assert.match(script, /暂不支持的 rclone 架构/);
  assert.match(script, /licenseSha256 = '8cd2e9e750b90a04b7d82dbbca3930c696ae0309d7c10464f90a44f45754cd04'/);
  assert.match(script, /rclone-v\$\{version\}-COPYING\.txt/);
  assert.match(script, /raw\.githubusercontent\.com\/rclone\/rclone\/v\$\{version\}\/COPYING/);
  assert.match(script, /actual !== licenseSha256/);
  assert.match(script, /APPLE_SIGNING_IDENTITY/);
  assert.match(script, /signing_identity_sha256/);
  assert.match(script, /'--force', '--options', 'runtime'/);
  assert.match(script, /'--verify', '--strict'/);
  assert.doesNotMatch(script, /findFile\(extractDir, 'COPYING'\)/);
  assert.doesNotMatch(script, /SHA256SUMS/);
});

test('Ubuntu x64 bundle is cross-built for amd64 and carries runtime imports and licenses', async () => {
  const [packageScript, dockerfile, service, server, nativeMount, compose] = await Promise.all([
    readRepoFile('scripts/package-ubuntu-native.mjs'),
    readRepoFile('Dockerfile.ubuntu-native'),
    readRepoFile('packaging/ubuntu-native/guangya-sync.service'),
    readRepoFile('server/server.mjs'),
    readRepoFile('server/native-mount.mjs'),
    readRepoFile('docker-compose.yml'),
  ]);
  assert.match(packageScript, /'--platform', 'linux\/amd64'/);
  assert.match(dockerfile, /COPY ui\/shareLink\.js \.\/ui\/shareLink\.js/);
  assert.match(dockerfile, /rclone-COPYING\.txt/);
  assert.match(dockerfile, /fe435e0c36228e7c2f116a8701f01127bb1f694005fc11d1f27186c8bca4115d/);
  assert.doesNotMatch(dockerfile, /FROM rclone\/rclone:/);
  assert.match(service, /^NoNewPrivileges=false$/m);
  assert.doesNotMatch(service, /^NoNewPrivileges=true$/m);
  assert.match(service, /^TimeoutStopSec=60$/m);
  assert.match(nativeMount, /export const NATIVE_MOUNT_STOP_TIMEOUT_MS/);
  assert.match(server, /gracefulShutdownTimeoutMs = NATIVE_MOUNT_STOP_TIMEOUT_MS \+ 15_000/);
  assert.match(compose, /^\s+stop_grace_period: 60s$/m);
});

test('Ubuntu installer stages upgrades, stops the old process and explicitly restarts', async () => {
  const installerPath = path.join(repoRoot, 'packaging/ubuntu-native/install.sh');
  const installer = await readRepoFile('packaging/ubuntu-native/install.sh');
  const syntax = spawnSync('bash', ['-n', installerPath], { encoding: 'utf8' });
  assert.equal(syntax.status, 0, syntax.stderr || syntax.stdout);
  assert.match(installer, /mktemp -d \/opt\/\.guangya-sync\.install\.XXXXXX/);
  assert.match(installer, /trap rollback_install EXIT/);
  assert.doesNotMatch(installer, /systemctl enable --now/);

  const switchIndex = installer.indexOf('mv "$STAGING_DIR" "$INSTALL_DIR"');
  const stopIndex = installer.lastIndexOf('systemctl stop guangya-sync.service', switchIndex);
  const restartIndex = installer.indexOf('systemctl restart guangya-sync.service', switchIndex);
  assert.ok(stopIndex >= 0 && stopIndex < switchIndex, 'old service must stop before the atomic switch');
  assert.ok(restartIndex > switchIndex, 'new service must explicitly restart after the atomic switch');
});

test('Docker runtime obtains rclone from a fixed-hash archive instead of a mutable image tag', async () => {
  const dockerfile = await readRepoFile('Dockerfile');
  assert.doesNotMatch(dockerfile, /FROM rclone\/rclone:/);
  assert.match(dockerfile, /rclone-v\$\{RCLONE_VERSION\}-linux-\$\{archive_arch\}\.zip/);
  assert.match(dockerfile, /sha256sum -c/);
  for (const digest of [
    'fe435e0c36228e7c2f116a8701f01127bb1f694005fc11d1f27186c8bca4115d',
    '97685285c9ad6a0cf17d5844115d2a67245af6444db672187074bd9c358de419',
  ]) assert.match(dockerfile, new RegExp(digest));
});
