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

test('虚拟库生成纯路径 STRM，并由独立 Emby 代理端口按路径决定 302', () => {
  assert.match(mountPanel, /Emby 虚拟库（STRM）/)
  assert.match(mountPanel, /视频和音频生成同名/)
  assert.match(mountPanel, /保留元数据/)
  assert.match(mountPanel, /排除所有元数据，只生成 STRM/)
  assert.match(mountPanel, /内容是云端纯路径/)
  assert.match(mountPanel, /继续使用原始 8096 不会触发光鸭/)
  assert.match(mountPanel, /普通请求和未命中的播放请求转发/)
  assert.match(mountPanel, /http:\/\/127\.0\.0\.1:18096/)
  for (const command of [
    'get_virtual_library_info',
    'update_virtual_library_settings',
    'upsert_virtual_library_mapping',
    'remove_virtual_library_mapping',
    'sync_virtual_library',
    'select_virtual_library_target',
  ]) {
    assert.match(bridge, new RegExp(command))
    assert.match(desktopPermissions, new RegExp(`"${command}"`))
  }
  assert.match(server, /\/api\/virtual-library/)
  assert.match(server, /embyProxyServer/)
})
