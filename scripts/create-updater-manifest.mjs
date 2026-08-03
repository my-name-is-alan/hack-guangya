import { createHash } from 'node:crypto'
import { promises as fs } from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { fileURLToPath } from 'node:url'

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..')
const metadata = JSON.parse(await fs.readFile(path.join(repoRoot, 'package.json'), 'utf8'))
const version = String(metadata.version)
const bundleDirectories = [
  path.join(repoRoot, 'src-tauri', 'target', 'release', 'bundle', 'nsis'),
  path.join(repoRoot, 'target', 'release', 'bundle', 'nsis'),
]

async function findSignedInstaller() {
  for (const directory of bundleDirectories) {
    const entries = await fs.readdir(directory).catch(() => [])
    const installerName = entries.find((name) => name.endsWith(`_${version}_x64-setup.exe`))
      || entries.find((name) => name.endsWith(`_${version}_x64-setup.exe.zip`))
    if (!installerName) continue
    const installer = path.join(directory, installerName)
    const signature = `${installer}.sig`
    await fs.access(signature)
    return { installer, signature }
  }
  throw new Error(`找不到 v${version} 的已签名 NSIS 更新包，请先使用 TAURI_SIGNING_PRIVATE_KEY 执行 pnpm tauri build`)
}

const { installer, signature } = await findSignedInstaller()
const releaseDirectory = path.join(repoRoot, 'release')
const releaseName = `guangya-folder-sync_${version}_x64-setup.exe${installer.endsWith('.zip') ? '.zip' : ''}`
const releaseInstaller = path.join(releaseDirectory, releaseName)
const releaseSignature = `${releaseInstaller}.sig`
await fs.mkdir(releaseDirectory, { recursive: true })
await fs.copyFile(installer, releaseInstaller)
await fs.copyFile(signature, releaseSignature)

const installerBytes = await fs.readFile(releaseInstaller)
const sha256 = createHash('sha256').update(installerBytes).digest('hex')
await fs.writeFile(`${releaseInstaller}.sha256`, `${sha256}  ${releaseName}\n`)

const manifest = {
  version,
  notes: process.env.GUANGYA_UPDATE_NOTES || `光鸭文件夹同步 v${version}`,
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      signature: (await fs.readFile(releaseSignature, 'utf8')).trim(),
      url: `https://github.com/my-name-is-alan/hack-guangya/releases/download/v${version}/${releaseName}`,
    },
  },
}
await fs.writeFile(path.join(releaseDirectory, 'latest.json'), `${JSON.stringify(manifest, null, 2)}\n`)

console.log(`Updater release ready: ${path.relative(repoRoot, releaseInstaller)}`)
console.log(`SHA256: ${sha256}`)
