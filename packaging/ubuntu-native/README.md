# 光鸭文件夹同步 Ubuntu 原生服务

此安装包自带 Node.js 24、rclone Linux x64 运行时和生产依赖，不需要安装 Docker、Node.js 或 pnpm。支持使用 systemd 的 Ubuntu 20.04、22.04、24.04 x86_64；安装器会在缺少时通过 apt 安装 FFprobe/FFmpeg 与 FUSE3。

## 安装

```bash
tar -xzf guangya-sync-native-ubuntu-x64-0.1.41.tar.gz
cd guangya-sync-native-ubuntu-x64-0.1.41
sudo ./install.sh
```

安装器会生成 48 位十六进制强随机管理密码，以 `0600` 权限写入 `/etc/guangya-sync.env`，并只在生成时显示一次；请当场保存。用户名默认是 `admin`。浏览器打开 `http://服务器IP:8080` 并使用这组管理账号登录。

原生包默认设置 `HOST=0.0.0.0`，会监听所有网络接口。如有防火墙，只应向可信来源放行实际使用的端口；跨不可信网络应配置 HTTPS 反向代理。反向代理与服务位于同一台机器时，也可把 `HOST` 改为 `127.0.0.1`，避免直接暴露服务端口。非回环监听必须配置非空的 `GUANGYA_ADMIN_PASSWORD`，否则服务会拒绝启动。

## 配置服务器目录

默认目录：

- SQLite 和配置：`/var/lib/guangya-sync/data`
- 网页服务器文件选择器：默认仅可浏览 `/var/lib/guangya-sync/watch` 和 `/var/lib/guangya-sync/archive`；应用自身的 `DATA_DIR` 会被隐藏，以保护 SQLite 登录会话
- 默认监控目录：`/var/lib/guangya-sync/watch`
- 上传后归档目录：`/var/lib/guangya-sync/archive`
- Emby STRM 虚拟库：`/var/lib/guangya-sync/virtual-library`

如果只想开放指定目录，或要修改默认监控与归档目录，请编辑 `/etc/guangya-sync.env`。多个可浏览根目录使用英文逗号分隔：

```bash
GUANGYA_FILE_ROOTS=/mnt/media,/srv/incoming,/mnt/archive
GUANGYA_WATCH_ROOT=/mnt/media
GUANGYA_ARCHIVE_ROOT=/mnt/archive
```

服务以 `guangya-sync` 用户运行。选择器会自动略过它无法读取的目录；请按服务器现有用户组和权限策略，授予媒体目录读取权限以及归档目录写入权限，然后重启：

```bash
sudo guangya-sync restart
```

## 原生挂载

网页“设置 → 挂载”支持由服务托管 rclone/FUSE 原生挂载。默认目标为：

```text
/var/lib/guangya-sync/mount
```

可在菜单中选择只读/读写、VFS 缓存、上传并行、读取并行和缓存空间上限。服务端只保存 WebDAV 密码哈希，每次服务重启后启动原生挂载都需要重新输入当前 WebDAV 挂载密码。修改默认目标可编辑：

```bash
GUANGYA_NATIVE_MOUNT_TARGET=/var/lib/guangya-sync/mount
```

读文件默认按 `GUANGYA_WEBDAV_REDIRECT=auto` 把 GET 302 重定向到云盘直链（rclone 直连 CDN，数据不再经过本服务中转）；对 Windows WebClient、macOS Finder、davfs2 等已知不支持重定向的客户端自动回退为服务器中转。设置为 `off` 可强制全部中转。

## Emby STRM 虚拟库

网页“设置 → 挂载 → Emby 虚拟库”可把云端视频和音频映射为同名 `.strm`，并可按每个目录决定是否下载 NFO、海报和字幕。STRM 内容是带签名的播放直链 `http(s)://<STRM 直链地址>/strm/<fileId>?sign=…`：Emby 只需把虚拟库目录加入媒体库（仅此一个目录，不需要挂载盘）。

播放推荐让客户端连接 **Emby 兼容网关**端口（默认 `18096`）：普通请求完整转发到 `GUANGYA_EMBY_UPSTREAM` 指向的 Emby，命中签名直链媒体源的原画播放请求直接 302 到云盘 CDN，播放数据不经过 Emby 和本服务。直连 Emby 原生端口（如 8096）也能播放，部分客户端的播放数据会经 Emby 服务器中转。

先在“设置 → 挂载 → Emby 虚拟库”填写 STRM 直链地址——Emby 服务器和播放设备都能访问到本服务的地址（管理端口或网关端口均可），也可通过配置文件做首次初始化：

```bash
GUANGYA_VIRTUAL_LIBRARY_ROOT=/var/lib/guangya-sync/virtual-library
GUANGYA_STRM_BASE_URL=http://192.168.1.10:18096
GUANGYA_EMBY_UPSTREAM=http://127.0.0.1:8096
# 网关默认只监听 127.0.0.1；局域网设备直连时放开并用防火墙限制来源
GUANGYA_EMBY_PROXY_HOST=0.0.0.0
GUANGYA_EMBY_PROXY_ALLOW_NON_LOOPBACK=1
```

管理账号和监听地址也保存在同一配置文件中：

```bash
HOST=0.0.0.0
GUANGYA_ADMIN_USERNAME=admin
GUANGYA_ADMIN_PASSWORD=安装时生成的强随机密码
```

修改管理密码后需要重启服务。不要把 `/etc/guangya-sync.env` 的内容粘贴到日志或问题报告中。

弱网环境下，OSS 分片默认等待 10 分钟并自动重试 3 次。可在 `/etc/guangya-sync.env` 调整：

```bash
GUANGYA_OSS_TIMEOUT_MS=600000
GUANGYA_OSS_RETRY_MAX=3
GUANGYA_OSS_PARALLEL=3
```

OSS 上传完成后，服务会继续等待光鸭云端异步入库，遇到“文件上传中”等处理中响应会自动退避重试，默认最多等待 10 分钟：

```bash
GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS=600000
GUANGYA_CLOUD_CONFIRM_POLL_MS=1000
```

## Hdhive 自动投稿

可在“备份任务”页面配置，也可编辑 `/etc/guangya-sync.env`：

```bash
HDHIVE_BASE_URL=https://你的-hdhive-地址
HDHIVE_GUANGYA_SYNC_SECRET=与Hdhive服务端一致的HMAC密钥
HDHIVE_GUANGYA_SYNC_INSTANCE_ID=
GUANGYA_AUTO_SHARE_QUIET_MS=30000
```

实例 ID 留空时首次启动自动生成并保存到 SQLite，升级或重启不会变化。现有备份任务默认不开启自动分享；请在界面逐个开启，需要处理历史上传记录时再点击“补建已有内容”。

## 管理

```bash
guangya-sync status
sudo guangya-sync restart
sudo guangya-sync logs
guangya-sync version
```

升级时重新执行新版 `sudo ./install.sh`，已有 `/etc/guangya-sync.env` 和 `/var/lib/guangya-sync/data/state.sqlite3` 不会被覆盖；旧配置缺少监听地址、管理用户名或管理密码等安全项时，安装器会补齐。仅当管理密码缺失或为空时才会生成并显示新密码，已有密码绝不会输出。旧版配置若仍是 `GUANGYA_FILE_ROOTS=/`，升级会尊重这项现有配置；建议手动改为实际需要的目录白名单。

卸载但保留配置和数据：

```bash
sudo ./uninstall.sh
```

同时删除配置和数据：

```bash
sudo ./uninstall.sh --purge-data
```
