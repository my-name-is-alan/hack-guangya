# 光鸭云盘工作台

这是一个基于 Tauri 2 的 Windows 桌面端，同时提供 Docker Web 和 Ubuntu 原生 Web 服务。它使用光鸭云盘公开网站当前的 OAuth Device Code 和文件接口，重点解决官方客户端没有“监控文件夹自动备份”的问题。

> [!IMPORTANT]
> 这是非官方社区工具，与光鸭云盘官方没有隶属或授权关系。公开仓库只包含源码、测试和构建配置；登录令牌、Hdhive 密钥、SQLite 状态库、监控/归档目录、安装包及构建缓存均不提交。请从 `.env.example` 创建本地 `.env`，不要把真实凭据写入仓库。

## 桌面端能力

- 应用内扫码登录和验证码登录：二维码、用户验证码和轮询状态都在工作台内显示，不保存账号密码；授权会话保存到当前系统用户的本地 SQLite，重启后自动恢复。
- 云盘文件管理：浏览根目录和子目录，拖入文件或文件夹上传，并支持批量复制、剪切、移动、删除和创建分享；文件操作会等待云端异步任务完成后再刷新。
- 批量重命名：规则按顺序链式执行，支持普通替换、正则替换、前后缀、序号模板及大小写转换，执行前实时预览并检查重名。
- 桌面操作习惯：支持右键菜单，以及 `Ctrl+A`、`Ctrl+C`、`Ctrl+X`、`Ctrl+V`、`F2`、`Delete` 快捷键。
- 离线下载：提交磁力、HTTP、HTTPS、ED2K 地址到云端任务。
- 分享收藏：保存分享 URL 到本机配置，支持复制和移除；不会保存光鸭密码或 OSS 临时密钥。
- 备份任务：可创建多个监控目录，可视化选择云端目标文件夹，保留本地目录结构并自动创建子目录；同步范围直接按文件后缀配置，支持视频、图片、字幕、音频快捷填充，也可以输入任意自定义后缀，默认填入图片、视频、音频的常用后缀。本地磁盘使用系统文件事件，网盘映射盘、NAS 或同步盘可切换为每 5 秒轮询；重复文件事件不会再次加入正在上传的任务。
- 账户概览：展示账号昵称、账号 ID、手机号、已用/剩余空间、VIP 状态和到期时间（接口返回哪些字段就展示哪些）。
- 上传完成后的源文件策略：保留（默认）、移动到归档目录、删除源文件。删除策略只有显式选择后才执行，并且上传期间源文件发生变化时不会删除或移动。
- 上传队列：最多两个并发上传，可暂停和继续；界面会显示准备目录、申请凭证、OSS 分片进度和云端入库状态；OSS 上传或秒传完成后立即把文件指纹和云端任务 ID 持久化到 SQLite，不依赖后续入库轮询，重启后不会重复上传未变更文件。
- 上传完成自动分享：按同步根目录第一层聚合。根目录单文件直接分享文件；`tvname/season 1/s01.mkv`、`tvname/season 2/s02.mkv` 始终复用 `tvname` 文件夹分享。目标无排队/上传且静默 30 秒后通知 Hdhive；现有任务升级后默认关闭，已有内容只通过“补建已有内容”显式处理。

## 开发和打包

源码构建和 Web 服务要求 Node.js 24 或更高版本。

```powershell
pnpm install
pnpm tauri dev
pnpm tauri build
```

安装包：`target/release/bundle/nsis/光鸭文件夹同步_0.1.13_x64-setup.exe`

## Docker Web

先准备管理账号。用户名默认是 `admin`；请生成独立的强随机密码，复制 `.env.example` 为 `.env` 并填入 `GUANGYA_ADMIN_PASSWORD`：

```bash
cp .env.example .env
openssl rand -hex 24
# 把输出填到 .env 的 GUANGYA_ADMIN_PASSWORD= 后面
docker compose up -d --build
```

Docker 会明确监听 `0.0.0.0:8080`，未设置管理密码或密码留空时 Compose 会拒绝启动。打开 `http://localhost:8080`，使用上述管理账号登录。需要从其他机器访问时，请同时限制防火墙来源；跨不可信网络应在前面配置 HTTPS 反向代理，避免通过明文 HTTP 传输管理凭据。

默认挂载关系为：

- `./watch` → `/watch`：允许添加为监控任务的目录；
- `./archive` → `/archive`：Docker 版归档策略可使用的目录；
- `./docker-data` → `/data`：任务和分享收藏配置。

Docker Web 不能直接读取浏览器所在电脑的任意本地目录。默认 `GUANGYA_FILE_ROOTS=/watch,/archive`，只能浏览和监控明确挂载到这两个容器目录中的文件。网页支持扫码登录，也可以通过 `GUANGYA_TOKEN` 环境变量或备用登录方式注入令牌；登录会话和上传历史保存在 `/data/state.sqlite3`。

Hdhive 联动可在网页“备份任务”页设置，也可通过环境变量配置：`HDHIVE_BASE_URL`、`HDHIVE_GUANGYA_SYNC_SECRET`、`HDHIVE_GUANGYA_SYNC_INSTANCE_ID`。实例 ID 未设置时会首次生成并持久化到 `/data/state.sqlite3`；密钥不通过状态接口返回。`GUANGYA_AUTO_SHARE_QUIET_MS` 可调整聚合静默时间，默认 30000 毫秒。

多个光鸭账号需要运行多个同步实例，每个实例使用不同的 `/data` 卷和实例 ID。先在 Hdhive 管理后台“光鸭同步”页面添加实例与投稿账号绑定，再把后台一次性生成的密钥填入对应同步端；不要让多个容器共用状态库。

网页端点击“上传文件”或“上传文件夹”时可以选择两种来源：

- 浏览器本地文件：从正在访问网页的电脑选择，文件通过浏览器传到容器后上传；
- 服务器挂载文件：浏览容器中 `GUANGYA_FILE_ROOTS` 允许的目录，直接从服务器上传，文件夹会递归处理并保留目录结构。

需要浏览服务器上的其他目录时，请把目录挂载进容器，并把容器内绝对路径加入 `GUANGYA_FILE_ROOTS`。网页接口会拒绝访问允许根目录之外的路径以及越界的符号链接。

### 本机直接运行 Web 服务

```bash
pnpm install --frozen-lockfile
pnpm web
```

不设置 `GUANGYA_ADMIN_PASSWORD` 时，服务只允许监听回环地址，适合在本机打开。需要远程访问时必须同时显式设置非回环监听地址和强密码，例如：

```bash
HOST=0.0.0.0 \
GUANGYA_ADMIN_USERNAME=admin \
GUANGYA_ADMIN_PASSWORD='替换为强随机密码' \
pnpm web
```

## Ubuntu 原生 Web 服务

生成不依赖 Docker、Node.js 或 pnpm，可直接安装为 systemd 服务的 Ubuntu x86_64 部署包：

```powershell
pnpm package:ubuntu
```

输出位于 `release/guangya-sync-native-ubuntu-x64-0.1.13.tar.gz`，解压后执行 `sudo ./install.sh`。安装包自带 Node.js 24 Linux 运行时和全部生产依赖。安装器会生成强随机管理密码、以 `0600` 权限写入 `/etc/guangya-sync.env`，并且只在首次生成时显示一次。Ubuntu 原生版默认只允许网页浏览 `/var/lib/guangya-sync/watch` 和 `/var/lib/guangya-sync/archive`；需要增加其他目录时使用 `GUANGYA_FILE_ROOTS` 设置白名单。应用自己的 `DATA_DIR` 始终隐藏，避免误选并上传包含登录会话的状态库。详细说明见包内 `README.md`。

## 接口边界

光鸭云盘目前没有公开稳定的第三方 API 承诺。本项目按 2026-07-21 公开网站行为接入，当前默认 OAuth Device Code client ID 为 `aMe-8VSlkrbQXpUR`，接口、字段和风控要求可能变化。上传凭证只用于服务端返回的目标 OSS 桶和对象路径，不写日志、不落盘。
