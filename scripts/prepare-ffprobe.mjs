import { chmod, copyFile, mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import process from 'node:process';
import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const installer = require('@ffprobe-installer/ffprobe');
const resourceDir = path.resolve('src-tauri/resources');
const executableName = process.platform === 'win32' ? 'ffprobe.exe' : 'ffprobe';
const destination = path.join(resourceDir, executableName);

await mkdir(resourceDir, { recursive: true });
await copyFile(installer.path, destination);
if (process.platform !== 'win32') await chmod(destination, 0o755);
await writeFile(path.join(resourceDir, 'ffprobe.version.json'), `${JSON.stringify({ version: installer.version, platform: process.platform, arch: process.arch }, null, 2)}\n`);
console.log(`FFprobe ${installer.version} prepared: ${destination}`);
