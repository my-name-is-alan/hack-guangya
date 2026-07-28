import fs from 'node:fs';
import fsp from 'node:fs/promises';
import path from 'node:path';
import { spawn, spawnSync } from 'node:child_process';

const MAX_PARALLELISM = 16;
const MAX_CACHE_SIZE_GB = 1024;
const FUSE_UNMOUNT_TIMEOUT_MS = 5_000;
const RCLONE_FLUSH_TIMEOUT_MS = 15_000;
const RCLONE_TERMINATE_TIMEOUT_MS = 5_000;
export const NATIVE_MOUNT_STOP_TIMEOUT_MS = (FUSE_UNMOUNT_TIMEOUT_MS * 2)
  + RCLONE_FLUSH_TIMEOUT_MS
  + RCLONE_TERMINATE_TIMEOUT_MS;

export function defaultNativeMountOptions(platform = process.platform) {
  return {
    rclone_path: '',
    target: platform === 'win32' ? 'X:' : '/mnt/guangya',
    access_mode: 'read_write',
    vfs_cache_mode: 'full',
    transfers: 4,
    read_streams: 4,
    cache_size_gb: 20,
  };
}

export function normalizeNativeMountOptions(value = {}, platform = process.platform) {
  const defaults = defaultNativeMountOptions(platform);
  const source = value && typeof value === 'object' ? value : {};
  // Copy only the documented allowlist. This prevents a UI payload containing
  // a password or another secret from being persisted in native_mount_options
  // and later reflected by the status endpoint.
  const options = {
    rclone_path: source.rclone_path ?? defaults.rclone_path,
    target: source.target ?? defaults.target,
    access_mode: source.access_mode ?? defaults.access_mode,
    vfs_cache_mode: source.vfs_cache_mode ?? defaults.vfs_cache_mode,
    transfers: source.transfers ?? defaults.transfers,
    read_streams: source.read_streams ?? defaults.read_streams,
    cache_size_gb: source.cache_size_gb ?? defaults.cache_size_gb,
  };
  options.rclone_path = String(options.rclone_path || '').trim();
  options.target = String(options.target || '').trim();
  options.access_mode = String(options.access_mode || '');
  options.vfs_cache_mode = String(options.vfs_cache_mode || '');
  options.transfers = Number(options.transfers);
  options.read_streams = Number(options.read_streams);
  options.cache_size_gb = Number(options.cache_size_gb);
  if (!options.target) throw new Error('请选择盘符或挂载目录');
  if (!['read_only', 'read_write'].includes(options.access_mode)) throw new Error('挂载权限必须是只读或读写');
  if (!['off', 'minimal', 'writes', 'full'].includes(options.vfs_cache_mode)) throw new Error('VFS 缓存模式无效');
  if (!Number.isInteger(options.transfers) || options.transfers < 1 || options.transfers > MAX_PARALLELISM) {
    throw new Error(`上传并行数必须为 1 到 ${MAX_PARALLELISM}`);
  }
  if (!Number.isInteger(options.read_streams) || options.read_streams < 1 || options.read_streams > MAX_PARALLELISM) {
    throw new Error(`读取并行数必须为 1 到 ${MAX_PARALLELISM}`);
  }
  if (!Number.isInteger(options.cache_size_gb) || options.cache_size_gb < 1 || options.cache_size_gb > MAX_CACHE_SIZE_GB) {
    throw new Error(`缓存上限必须为 1 到 ${MAX_CACHE_SIZE_GB} GB`);
  }
  return options;
}

export function nativeMountArguments(options, cacheDir, logFile, platform = process.platform) {
  const args = [
    'mount',
    ':webdav:',
    options.target,
    '--config', platform === 'win32' ? 'NUL' : '/dev/null',
    '--vfs-cache-mode', options.vfs_cache_mode,
    '--transfers', String(options.transfers),
    '--vfs-read-chunk-streams', String(options.read_streams),
    '--cache-dir', cacheDir,
    '--vfs-cache-max-size', `${options.cache_size_gb}G`,
    '--vfs-cache-max-age', '24h',
    '--vfs-cache-poll-interval', '1m',
    '--vfs-write-back', '5s',
    '--dir-cache-time', '5m',
    '--poll-interval', '0',
    '--buffer-size', '4M',
    '--vfs-read-ahead', '16M',
    '--vfs-read-chunk-size', '4M',
    '--log-file', logFile,
    '--log-level', 'INFO',
  ];
  if (options.access_mode === 'read_only') args.push('--read-only');
  if (platform === 'win32') args.push('--no-console', '--volname', '光鸭云盘');
  return args;
}

function platformName(platform) {
  if (platform === 'win32') return 'windows';
  if (platform === 'darwin') return 'macos';
  return 'linux';
}

function probeRclone(executable) {
  const result = spawnSync(executable, ['version'], {
    encoding: 'utf8',
    windowsHide: true,
    timeout: 10_000,
  });
  if (result.error) return { available: false, version: '', error: result.error.message };
  if (result.status !== 0) return { available: false, version: '', error: String(result.stderr || '').trim() };
  const version = String(result.stdout || '').split(/\r?\n/).map((line) => line.trim()).find(Boolean) || '';
  return { available: Boolean(version), version, error: version ? '' : 'rclone version 没有输出版本' };
}

function fusePrerequisite(platform) {
  if (platform === 'win32') {
    const available = [
      'C:\\Program Files (x86)\\WinFsp\\bin\\winfsp-x64.dll',
      'C:\\Program Files\\WinFsp\\bin\\winfsp-x64.dll',
      'C:\\Program Files (x86)\\WinFsp\\bin\\winfsp-a64.dll',
      'C:\\Program Files\\WinFsp\\bin\\winfsp-a64.dll',
    ].some((entry) => fs.existsSync(entry));
    return { available, message: available ? 'WinFsp 已就绪' : '原生挂载需要先安装 WinFsp' };
  }
  if (platform === 'darwin') {
    const available = ['/Library/Filesystems/macfuse.fs', '/Library/Filesystems/fuse-t.fs']
      .some((entry) => fs.existsSync(entry));
    return { available, message: available ? 'macFUSE/FUSE-T 已就绪' : '原生挂载需要先安装 macFUSE 或 FUSE-T' };
  }
  const available = fs.existsSync('/dev/fuse');
  return { available, message: available ? 'FUSE 设备已就绪' : '原生挂载需要 fuse3 与可访问的 /dev/fuse' };
}

function readLogTail(logFile) {
  try {
    return fs.readFileSync(logFile, 'utf8').split(/\r?\n/).filter(Boolean).slice(-4).join(' | ');
  } catch {
    return '';
  }
}

function obscurePassword(executable, password) {
  const result = spawnSync(executable, ['obscure', '-'], {
    input: `${password}\n`,
    encoding: 'utf8',
    windowsHide: true,
    timeout: 10_000,
  });
  if (result.error) throw new Error(`启动 rclone 密码处理失败：${result.error.message}`);
  if (result.status !== 0) throw new Error(`rclone 密码处理失败：${String(result.stderr || '').trim()}`);
  const obscured = String(result.stdout || '').trim();
  if (!obscured) throw new Error('rclone 没有返回处理后的密码');
  return obscured;
}

export function prepareNativeMountTarget(target, platform = process.platform) {
  if (platform === 'win32') {
    if (/^[a-z]:$/i.test(target)) {
      if (fs.existsSync(`${target}\\`)) throw new Error(`盘符 ${target} 已被占用，请选择未使用的盘符`);
      return;
    }
    if (!path.win32.isAbsolute(target)) throw new Error('挂载目录必须使用绝对路径；Windows 也可填写 X: 形式的盘符');
    const parent = path.win32.dirname(target);
    if (!parent || parent === target || !fs.existsSync(parent) || !fs.statSync(parent).isDirectory()) {
      throw new Error('Windows 挂载目录的上级目录必须已存在');
    }
    try {
      const stat = fs.lstatSync(target);
      if (stat.isSymbolicLink() || !stat.isDirectory()) {
        throw new Error('Windows 挂载目标必须是未使用的盘符或普通空目录');
      }
      if (fs.readdirSync(target).length > 0) {
        throw new Error('Windows 挂载目录必须为空；为避免覆盖现有文件，程序不会使用非空目录');
      }
      fs.rmdirSync(target);
    } catch (error) {
      if (error?.code !== 'ENOENT') throw error;
    }
    return;
  }
  if (!path.isAbsolute(target)) throw new Error('挂载目录必须使用绝对路径');
  try {
    const stat = fs.lstatSync(target);
    if (stat.isSymbolicLink() || !stat.isDirectory()) {
      throw new Error('挂载目标必须是普通空目录，不能是符号链接');
    }
    if (fs.readdirSync(target).length > 0) {
      throw new Error('挂载目录必须为空；为避免遮蔽现有文件，程序不会使用非空目录');
    }
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error;
    fs.mkdirSync(target, { recursive: true, mode: 0o700 });
  }
  if (platform !== 'win32') fs.chmodSync(target, 0o700);
}

function restoreWindowsDirectoryTarget(target, platform) {
  if (platform !== 'win32' || /^[a-z]:$/i.test(target)) return;
  try {
    if (path.win32.isAbsolute(target) && !fs.existsSync(target)) fs.mkdirSync(target);
  } catch {
    // A still-mounted WinFsp path cannot be recreated; it will disappear after unmount.
  }
}

async function waitForExit(child, timeoutMs) {
  if (child.exitCode != null) return;
  await Promise.race([
    new Promise((resolve) => child.once('exit', resolve)),
    new Promise((resolve) => setTimeout(resolve, timeoutMs)),
  ]);
}

export function createNativeMountManager({
  dataDir,
  initialOptions,
  enabled = true,
  platform = process.platform,
  defaultRclonePath = process.env.GUANGYA_RCLONE_PATH || '',
  allowCustomRclonePath = process.env.GUANGYA_ALLOW_CUSTOM_RCLONE_PATH === '1',
}) {
  let options = normalizeNativeMountOptions(initialOptions, platform);
  const configuredRclonePath = String(defaultRclonePath || '').trim();
  if (!allowCustomRclonePath && options.rclone_path && options.rclone_path !== configuredRclonePath) {
    options.rclone_path = '';
  }
  let child = null;
  let startedAt = null;
  let lastError = null;
  const cacheDir = path.join(dataDir, 'native-mount-cache');
  const logFile = path.join(dataDir, 'logs', 'native-mount.log');

  function executable() {
    return options.rclone_path || configuredRclonePath || (platform === 'win32' ? 'rclone.exe' : 'rclone');
  }

  function refresh() {
    if (child?.exitCode != null) {
      const exitCode = child.exitCode;
      child = null;
      startedAt = null;
      restoreWindowsDirectoryTarget(options.target, platform);
      const log = readLogTail(logFile);
      lastError = log ? `rclone 原生挂载已退出：${log}` : `rclone 原生挂载已退出（${exitCode}）`;
    }
  }

  function info() {
    refresh();
    const rclone = probeRclone(executable());
    const fuse = fusePrerequisite(platform);
    return {
      supported: true,
      enabled,
      available: enabled && rclone.available && fuse.available,
      running: Boolean(child),
      engine: 'rclone',
      platform: platformName(platform),
      rclone_available: rclone.available,
      fuse_available: fuse.available,
      version: rclone.version,
      prerequisite: enabled ? fuse.message : '服务端原生挂载未启用',
      ...options,
      started_at: startedAt,
      error: lastError || (!rclone.available ? rclone.error : null),
    };
  }

  function setOptions(value) {
    const normalized = normalizeNativeMountOptions(value, platform);
    if (!allowCustomRclonePath && normalized.rclone_path && normalized.rclone_path !== configuredRclonePath) {
      throw new Error('为防止执行未受信任程序，rclone 路径只能由服务端 GUANGYA_RCLONE_PATH 配置');
    }
    if (child && JSON.stringify(normalized) !== JSON.stringify(options)) {
      throw new Error('请先卸载当前原生挂载，再修改挂载参数');
    }
    options = normalized;
    return info();
  }

  async function start({ endpoint, username, password }) {
    refresh();
    if (child) return info();
    if (!enabled) throw new Error('服务端原生挂载未启用');
    const rclone = probeRclone(executable());
    if (!rclone.available) throw new Error(`未找到可用的 rclone：${rclone.error || executable()}`);
    const fuse = fusePrerequisite(platform);
    if (!fuse.available) throw new Error(fuse.message);
    await Promise.all([
      fsp.mkdir(cacheDir, { recursive: true, mode: 0o700 }),
      fsp.mkdir(path.dirname(logFile), { recursive: true, mode: 0o700 }),
    ]);
    if (platform !== 'win32') {
      await Promise.all([
        fsp.chmod(cacheDir, 0o700),
        fsp.chmod(path.dirname(logFile), 0o700),
      ]);
    }
    await fsp.writeFile(logFile, '', { mode: 0o600 });
    if (platform !== 'win32') await fsp.chmod(logFile, 0o600);
    const obscuredPassword = obscurePassword(executable(), password);
    prepareNativeMountTarget(options.target, platform);
    let spawned;
    try {
      spawned = spawn(executable(), nativeMountArguments(options, cacheDir, logFile, platform), {
        env: {
          ...process.env,
          RCLONE_WEBDAV_URL: endpoint,
          RCLONE_WEBDAV_VENDOR: 'other',
          RCLONE_WEBDAV_USER: username,
          RCLONE_WEBDAV_PASS: obscuredPassword,
        },
        stdio: 'ignore',
        windowsHide: true,
      });
    } catch (error) {
      restoreWindowsDirectoryTarget(options.target, platform);
      throw error;
    }
    child = spawned;
    startedAt = Math.floor(Date.now() / 1000);
    lastError = null;
    spawned.once('error', (error) => {
      if (child === spawned) {
        child = null;
        startedAt = null;
      }
      lastError = `rclone 原生挂载进程错误：${error.message}`;
      restoreWindowsDirectoryTarget(options.target, platform);
    });
    await new Promise((resolve) => setTimeout(resolve, 1_200));
    refresh();
    if (!child) throw new Error(lastError || 'rclone 在挂载完成前退出');
    return info();
  }

  async function stop() {
    refresh();
    if (!child) {
      startedAt = null;
      return info();
    }
    if (platform === 'darwin') {
      spawnSync('umount', [options.target], { stdio: 'ignore', timeout: FUSE_UNMOUNT_TIMEOUT_MS });
    } else if (platform !== 'win32') {
      let result = spawnSync('fusermount3', ['-u', options.target], { stdio: 'ignore', timeout: FUSE_UNMOUNT_TIMEOUT_MS });
      if (result.error || result.status !== 0) {
        result = spawnSync('fusermount', ['-u', options.target], { stdio: 'ignore', timeout: FUSE_UNMOUNT_TIMEOUT_MS });
      }
    }
    // rclone's write-back delay is 5 seconds. Give a normal unmount enough time
    // to flush dirty VFS entries before asking the process to terminate.
    await waitForExit(child, RCLONE_FLUSH_TIMEOUT_MS);
    if (child.exitCode == null) {
      child.kill('SIGTERM');
      await waitForExit(child, RCLONE_TERMINATE_TIMEOUT_MS);
    }
    if (child.exitCode == null) child.kill('SIGKILL');
    child = null;
    startedAt = null;
    lastError = null;
    restoreWindowsDirectoryTarget(options.target, platform);
    return info();
  }

  function shutdown() {
    if (child) child.kill('SIGTERM');
    child = null;
  }

  return { info, options: () => ({ ...options }), setOptions, start, stop, shutdown };
}
