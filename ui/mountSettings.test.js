import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const [settingsView, mountPanel, bridge, server, desktopPermissions] = await Promise.all([
  readFile(new URL('./views/SystemSettingsView.vue', import.meta.url), 'utf8'),
  readFile(new URL('./components/settings/MountSettingsPanel.vue', import.meta.url), 'utf8'),
  readFile(new URL('./bridge.js', import.meta.url), 'utf8'),
  readFile(new URL('../server/server.mjs', import.meta.url), 'utf8'),
  readFile(new URL('../src-tauri/permissions/app.toml', import.meta.url), 'utf8'),
])

test('设置页暴露跨平台 WebDAV 挂载入口和 CRUD 说明', () => {
  assert.match(settingsView, /key="mount"/)
  assert.match(settingsView, /<MountSettingsPanel/)
  assert.match(mountPanel, /Windows/)
  assert.match(mountPanel, /macOS/)
  assert.match(mountPanel, /Linux/)
  assert.match(mountPanel, /Docker \/ rclone/)
  assert.match(mountPanel, /读取、列目录、创建、覆盖、重命名、移动、复制和删除/)
})

test('挂载菜单提供 rclone FUSE 原生挂载、权限、并行和缓存控制', () => {
  assert.match(mountPanel, /原生挂载（rclone \/ FUSE）/)
  assert.match(mountPanel, /只读/)
  assert.match(mountPanel, /读写/)
  assert.match(mountPanel, /上传并行/)
  assert.match(mountPanel, /读取并行/)
  assert.match(mountPanel, /VFS 缓存/)
  assert.match(mountPanel, /start_native_mount/)
  assert.match(mountPanel, /stop_native_mount/)
})

test('原生挂载每次启动都要求 WebDAV 密码且图标操作可访问', () => {
  assert.doesNotMatch(mountPanel, /v-if="!isTauri" label="当前 WebDAV 挂载密码"/)
  assert.match(mountPanel, /if \(nativePassword\.value\.length < 12\)/)
  assert.match(mountPanel, /password:\s*nativePassword\.value/)
  assert.match(mountPanel, /仅用于本次启动原生挂载，不会写入配置/)
  assert.match(mountPanel, /:readonly="isTauri"/)
  assert.match(mountPanel, /自定义程序必须通过文件选择器批准，只在本次应用运行期间有效/)
  for (const label of ['选择挂载目录', '选择 rclone 可执行文件', '复制 WebDAV 地址', '复制 WebDAV 用户名']) {
    assert.match(mountPanel, new RegExp(`aria-label="${label}"`))
  }
  assert.match(mountPanel, /\.input-icon-button \{[^}]*width:\s*40px;[^}]*height:\s*40px;/)
  assert.match(mountPanel, /\.input-icon-button:focus-visible \{/)
})

test('设置导航和挂载表单在窄窗口切换为纵向内容且不制造横向滚动', () => {
  assert.match(settingsView, /window\.matchMedia\('\(max-width: 960px\)'\)/)
  assert.match(settingsView, /:tab-position="tabPosition"/)
  assert.match(settingsView, /overflow-x:\s*hidden/)
  assert.match(mountPanel, /\.two-columns, \.three-columns \{ grid-template-columns: 1fr/)
  assert.match(mountPanel, /overflow-x:\s*hidden/)
})

test('桌面与 Docker Web bridge 都能读取挂载信息', () => {
  assert.match(bridge, /get_mount_info/)
  assert.match(bridge, /webRequest\('\/api\/mount'\)/)
  assert.match(bridge, /update_mount_credentials/)
  assert.match(bridge, /webRequest\('\/api\/mount\/credentials'/)
  assert.match(bridge, /get_native_mount_info/)
  assert.match(bridge, /webRequest\('\/api\/mount\/native'\)/)
  assert.match(server, /url\.pathname === '\/api\/mount'/)
  assert.match(server, /webdavEndpoint/)
  assert.match(server, /webdav_access_control/)
  assert.match(mountPanel, /保存账号密码/)
  assert.match(mountPanel, /不直接暴露公网/)
})

test('桌面 ACL 放行全部 WebDAV 与原生挂载命令', () => {
  for (const command of [
    'get_mount_info',
    'update_mount_credentials',
    'get_native_mount_info',
    'update_native_mount_options',
    'start_native_mount',
    'stop_native_mount',
    'select_native_mount_target',
    'select_rclone_binary',
  ]) {
    assert.match(desktopPermissions, new RegExp(`"${command}"`))
  }
})
