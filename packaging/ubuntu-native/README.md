# 光鸭文件夹同步 Ubuntu 原生服务

此安装包自带 Node.js 24 Linux x64 运行时和生产依赖，不需要安装 Docker、Node.js 或 pnpm。支持使用 systemd 的 Ubuntu 20.04、22.04、24.04 x86_64。

## 安装

```bash
tar -xzf guangya-sync-native-ubuntu-x64-0.1.12.tar.gz
cd guangya-sync-native-ubuntu-x64-0.1.12
sudo ./install.sh
```

浏览器打开 `http://服务器IP:8080`。如有防火墙，请放行实际使用的端口。

## 配置服务器目录

默认目录：

- SQLite 和配置：`/var/lib/guangya-sync/data`
- 网页服务器文件选择器：从 `/` 开始，可浏览 `guangya-sync` 服务用户有权限读取的目录；应用自身的 `DATA_DIR` 会被隐藏，以保护 SQLite 登录会话
- 默认监控目录：`/var/lib/guangya-sync/watch`
- 上传后归档目录：`/var/lib/guangya-sync/archive`

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

升级时重新执行新版 `sudo ./install.sh`，已有 `/etc/guangya-sync.env` 和 `/var/lib/guangya-sync/data/state.sqlite3` 不会被覆盖；旧配置缺少新增配置项时，安装器只会追加对应默认值。

卸载但保留配置和数据：

```bash
sudo ./uninstall.sh
```

同时删除配置和数据：

```bash
sudo ./uninstall.sh --purge-data
```
