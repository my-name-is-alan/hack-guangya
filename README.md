# 光鸭云盘工作台

这是一个基于 Tauri 2 的 Windows 桌面端，同时提供 Docker Web 和 Ubuntu 原生 Web 服务。它按光鸭 Windows PC 客户端的实测 OAuth Device Code 与文件协议接入，重点解决官方客户端没有“监控文件夹自动备份”的问题。

> [!IMPORTANT]
> 这是非官方社区工具，与光鸭云盘官方没有隶属或授权关系。公开仓库只包含源码、测试和构建配置；登录令牌、Hdhive 密钥、SQLite 状态库、监控/归档目录、安装包及构建缓存均不提交。请从 `.env.example` 创建本地 `.env`，不要把真实凭据写入仓库。

## 桌面端能力

- 应用内扫码登录和验证码登录：二维码、用户验证码和轮询状态都在工作台内显示，不保存账号密码；授权会话保存到当前系统用户的本地 SQLite，重启后自动恢复。
- 桌面端自动更新：启动时默认检查 GitHub 最新正式版，也可在“设置 → 更新”手动检查；发现新版本后展示版本说明和下载进度，经确认后校验签名并安装。
- 云盘文件管理：浏览根目录和子目录，拖入文件或文件夹上传，并支持新建文件夹、详情、云端最近、批量复制、剪切、移动、重命名、移入回收站和创建分享；回收站支持批量恢复、彻底删除和清空，异步文件操作完成后才刷新。
- 当前账号开发者模式与小号秒传：在“多号秒传”面板填写当前登录账号自己的官方开发者 `client_id` / `client_secret`；所有权验证通过后，文件列表/详情可在主接口失败时使用开发者接口兜底，并可保存多个小号接收 TOKEN 进行秒传。
- 本地目录挂载：通过 WebDAV 将整个云盘映射为 Windows 盘符、macOS Finder 目录或 Linux/FUSE 目录；支持列目录、读取、新建、覆盖、重命名、移动、复制和删除，Docker Web 同样提供 `/dav/`。
- 批量重命名：规则按顺序链式执行，支持普通替换、正则替换、前后缀、序号模板及大小写转换，执行前实时预览并检查重名。
- 桌面操作习惯：支持右键菜单，以及 `Ctrl+A`、`Ctrl+C`、`Ctrl+X`、`Ctrl+V`、`F2`、`Delete` 快捷键。
- 云添加：先解析磁力、HTTP、HTTPS、ED2K 资源，再选择云端目录并创建任务；展示进度、错误和当日次数，支持取消、重试和清理记录，磁力资源会按解析结果提交可用子文件序号。
- 分享管理：查看状态、剩余有效期、访问/转存/下载统计和流量，支持批量取消、编辑有效期与免登录流量上限、清理失效记录；根级文件夹详情中开关直链文件夹，内部文件详情中获取长链或短链。分享收藏仍保存在本机配置，不保存光鸭密码或 OSS 临时密钥。
- 备份任务：可创建多个监控目录，可视化选择云端目标文件夹，保留本地目录结构并自动创建子目录；同步范围直接按文件后缀配置，支持视频、图片、字幕、音频快捷填充，也可以输入任意自定义后缀，默认填入图片、视频、音频的常用后缀。本地磁盘使用系统文件事件，网盘映射盘、NAS 或同步盘可切换为每 5 秒轮询；重复文件事件不会再次加入正在上传的任务。
- 光鸭原生媒体整理：独立于 MoviePilot 的原生云盘引擎，只在同一个光鸭云盘内执行 A → B 复制/移动，不调用 MoviePilot API、不经过本地挂载盘，也不存在跨盘整理。可创建多个云盘目录监控（每 15 秒轮询 A 目录），内置电影/电视剧解析、TMDB 候选评分与人工复核、字幕/音轨/预告片/花絮同步和冲突预览；目标路径支持分类、国家、年份、标题、TMDB ID、季集、版本、清晰度等字段自由组合。元数据刮削默认关闭，开启后只刮选中的 NFO、海报、背景图或季海报类型。移动/覆盖前必须确认已有分享可能失效；整理后若启用分享，会从 B 目录创建新分享并通知 HDHive，不复用 A 目录旧分享。
- 账户配置：展示账号昵称、账号 ID、手机号与空间；严格绑定当前账号的开发者模式和多号秒传配置在独立面板中管理；不再展示 VIP/SVIP、流量和权益规则。
- 上传完成后的源文件策略：保留（默认）、移动到归档目录、删除源文件。删除策略只有显式选择后才执行，并且上传期间源文件发生变化时不会删除或移动。
- 上传队列：上传、下载任务并发数均可在 1–8 之间设置，可暂停和继续；桌面端下载不小于 16 MiB 的单个大文件时会探测 CDN Range 能力，并在总连接预算 8 路内自动使用 1–4 路并发分片，不支持 Range 或分片失败时回退单流；界面会显示准备目录、下载分片、OSS 分片进度和云端入库状态；OSS 上传或秒传完成后立即把文件指纹和云端任务 ID 持久化到 SQLite，不依赖后续入库轮询，重启后不会重复上传未变更文件。
- 上传完成自动分享：按同步根目录第一层聚合。根目录单文件直接分享文件；`tvname/season 1/s01.mkv`、`tvname/season 2/s02.mkv` 始终复用 `tvname` 文件夹分享。目标无排队/上传且静默 30 秒后通知 Hdhive；现有任务升级后默认关闭，已有内容只通过“补建已有内容”显式处理。

## 云盘内原生媒体整理

在“媒体识别与整理”中先选择光鸭云盘内的来源 A 文件夹和目标 B 文件夹。A、B 必须是两个已存在的云端文件夹，不能相同或互相包含；整理器只提交光鸭官方的云端复制、移动、重命名、建目录和上传接口，不读取或搬运本地路径。

默认模板示例（保存的是相对于 B 的路径）：

```text
电影：{category}/{country}/{year}/{title} ({year}) [tmdb-{tmdb_id}]/{title} ({year}){edition}{quality}{part}.{ext}
电视剧：{category}/{country}/{year}/{title} ({year}) [tmdb-{tmdb_id}]/Season {season:02}/{title}.S{season:02}E{episode:02}{episode_end}.{ext}
```

可用字段包括 `{category}`、兼容拼写 `{catgroy}`、`{country}`、`{year}`、`{title}`、`{original_title}`、`{tmdb_id}`（兼容 `{tmdbid}`）、`{season}`、`{episode}`、`{episode_end}`、`{Season x}`、`{Expose n}`、`{edition}`、`{quality}`、`{part}` 和 `{ext}`；字段可以自由组合，系统会清理非法文件名字符并拒绝 `.`/`..` 路径跳转。页面提供三个预设，也可直接编辑完整相对路径。

整理监控是云端轮询，不是本机文件事件监听。两个 WebDAV/原生挂载共用同一云端目录时，写操作会立即失效本进程缓存；服务端目录缓存最长保留 15 秒，rclone 默认目录缓存为 2 秒、VFS 变化轮询为 5 秒，通常几秒内即可看到另一挂载创建的新文件夹。客户端自身的离线缓存仍可能需要手动刷新。

刮削开关默认关闭。打开后先选择要执行的类型（默认预选电影 NFO、剧集 NFO、海报、背景图），不会把所有可用元数据全部下载；刮削失败会记录为警告并保留已完成的主体整理。上传任务可在“上传后自动整理”中关联 A 目录：关闭时沿用原上传后自动分享流程，开启时先等待 A → B 完成，再从 B 创建新分享，避免把即将移动的 A 目录提交给 HDHive。

光鸭分享不是不可变快照。移动、删除或覆盖云端文件可能使旧分享失效或内容不再完整，因此整理器对 `move`/`overwrite` 强制风险确认，整理后的分享始终是 B 目录的新链接；A 目录或历史分享不会被复用。

## 开发和打包

源码构建和 Web 服务要求 Node.js 24 或更高版本。

```powershell
pnpm install
pnpm tauri dev
pnpm tauri build
```

安装包：`target/release/bundle/nsis/光鸭文件夹同步_0.1.28_x64-setup.exe`

正式更新包必须使用长期保存的同一把 Tauri 私钥签名。构建机设置 `TAURI_SIGNING_PRIVATE_KEY`（可填私钥内容或私钥文件路径）和可选的 `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` 后执行构建，再生成 GitHub Release 所需文件：

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY=Get-Content "C:\安全目录\hack-guangya.key" -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""
pnpm tauri build
pnpm package:updater
```

把 `release/` 中的安装包、`.sig`、`.sha256` 和 `latest.json` 一起上传到 `v版本号` Release。客户端固定读取最新 Release 的 `latest.json`；签名私钥丢失后，已安装版本将不能信任新密钥签出的更新包，因此必须离线备份且不能提交仓库。

## 开发者接口与小号秒传

打开“设置 → 多号秒传 → Token 配置”，填写当前登录账号在光鸭开发者后台生成的 `client_id` 和 `client_secret`。点击“验证当前账号”时，应用先用当前登录态读取一个真实 `fileId`，再用开发者凭据读取同一个文件；两次都成功才记录账号绑定并允许打开开关。切换登录账号、修改 `client_id` 或验证状态失效时，开发者模式立即停用，避免浏览或转存其他账号的文件。

模式开启后，普通 PC 接口仍是文件列表与详情的主路径；只有主接口读取失败，才使用 `https://dapi.guangyapan.com` 的同名只读接口兜底。新建、移动、重命名、删除等未出现在开发者文档中的操作仍使用当前 PC 登录会话。Docker Web 也可以通过 `GUANGYA_DEVELOPER_CLIENT_ID`、`GUANGYA_DEVELOPER_CLIENT_SECRET` 注入凭据，但仍必须在“多号秒传 → Token 配置”完成当前账号验证和模式启用。

小号在光鸭中创建接收 TOKEN，并给 TOKEN 授权一个目标目录；在“多号秒传 → Token 配置”添加该 TOKEN。之后在“文件”中选中最多 20 个文件或文件夹，点击“小号秒传”并选择接收小号；任务状态可在“多号秒传 → 任务记录”查看。提交前应用会再次用开发者凭据确认所选文件属于已绑定账号，然后调用 `upload_by_fileid`；若文件尚未通过审核（业务码 `18011`），自动调用 `pre_upload`，后台轮询预审完成后继续秒传。任务及脱敏统计保存在本机 SQLite，应用重启后会恢复未完成任务；完整 `client_secret` 和接收 TOKEN 不通过状态接口回显。

> [!NOTE]
> 官方 TOKEN 模型是单向绑定：一个接收 TOKEN 只支持“当前开发者账号 → TOKEN 所属小号”。同一个 TOKEN 不能反向传回；真正双向互传需要另一方向也具备开发者凭据和接收 TOKEN。接口规则以[官方 TOKEN 上传文档](https://wcn6ijfe07e0.feishu.cn/wiki/R6Z2weFwKiwnuBktcoacoDAHnZg)为准。

## WebDAV 本地挂载

登录光鸭后，打开“设置 → 挂载”设置独立的 WebDAV 用户名和密码，并查看各平台命令。桌面端默认只监听：

```text
http://127.0.0.1:19090/
```

默认用户名是 `guangya`。用户名和密码都能在挂载设置页修改；密码至少 12 位且不会由状态接口回显，已有盘符或目录在修改后需要重新连接。端口冲突时可在启动前设置 `GUANGYA_WEBDAV_PORT`。

常用挂载方式：

```powershell
# Windows：* 会安全地提示输入密码
net use Z: "http://127.0.0.1:19090/" /user:guangya * /persistent:yes
```

```bash
# macOS：也可在 Finder 中选择“前往 → 连接服务器”
mkdir -p "$HOME/Guangya"
mount_webdav "http://127.0.0.1:19090/" "$HOME/Guangya"

# Linux：先安装 davfs2
sudo mkdir -p /mnt/guangya
sudo mount -t davfs "http://127.0.0.1:19090/" /mnt/guangya
```

WebDAV 是在线文件系统，不是离线镜像。打开大文件时由系统 WebDAV 客户端或 rclone 的 VFS 缓存负责按需读取；写入会继续走现有的 OSS 分片上传与云端入库确认。Windows 内置 WebClient 被策略禁用或需要更稳定的大文件缓存时，建议改用 `rclone mount --vfs-cache-mode full`。

## 原生挂载（rclone / FUSE）

“设置 → 挂载”默认提供软件托管的原生挂载模式。桌面安装包内置经过 SHA256 校验的官方 rclone `v1.74.4`，无需单独配置 remote：

- Windows 使用 WinFsp，可填写 `X:` 等未占用盘符，也可选择 `G:\guangya` 这类空目录；Windows 目录挂载要求叶子路径在启动时不存在，因此程序只会临时移除空目录并在卸载后恢复，非空目录会被拒绝且绝不覆盖；
- macOS 使用 macFUSE 或 FUSE-T，目标为本机绝对目录；
- Linux 使用 FUSE3，目标为本机绝对目录；
- 退出软件时会停止托管的 rclone 进程并自动卸载。

挂载菜单可选择只读或读写、VFS 缓存模式、上传并行数、读取分块并行数和缓存空间上限。读写模式建议使用“完整缓存”或“仅写入缓存”；只读模式会把 `--read-only` 直接传给 rclone，从文件系统层拒绝新建、覆盖、删除和重命名。

原生挂载仍通过只监听本机的 WebDAV 服务访问光鸭接口，不会新增公网端口。rclone 密码通过标准输入转换并仅注入子进程环境，不写入 rclone 配置文件。

## Docker Web

Docker Hub 镜像：[`94xhzy/guangya-sync`](https://hub.docker.com/r/94xhzy/guangya-sync)，推荐固定使用版本标签：

```bash
docker pull 94xhzy/guangya-sync:0.1.28
```

先准备管理账号。用户名默认是 `admin`；请生成独立的强随机密码，复制 `.env.example` 为 `.env` 并填入 `GUANGYA_ADMIN_PASSWORD`：

```bash
cp .env.example .env
openssl rand -hex 24
# 把输出填到 .env 的 GUANGYA_ADMIN_PASSWORD= 后面
docker compose pull
docker compose up -d
```

完整的环境变量、目录挂载、HDHive、升级、回滚、备份与反向代理配置见 [DOCKER.md](./DOCKER.md)。

Docker 会明确监听 `0.0.0.0:8080`，未设置管理密码或密码留空时 Compose 会拒绝启动。打开 `http://localhost:8080`，使用上述管理账号登录。需要从其他机器访问时，请同时限制防火墙来源；跨不可信网络应在前面配置 HTTPS 反向代理，避免通过明文 HTTP 传输管理凭据。

Docker WebDAV 使用独立端口和独立账号密码，不复用管理端口或管理员凭据。Compose 默认只把端口发布到服务器本机回环地址：

```text
http://127.0.0.1:19090/dav/
```

启动后在“设置 → 挂载”设置 WebDAV 用户名和密码；也可用 `GUANGYA_WEBDAV_USERNAME`、`GUANGYA_WEBDAV_PASSWORD` 完成首次初始化。管理端口 `8080` 不再提供 `/dav/`。不要把 `19090` 改成 `0.0.0.0` 公网发布；需要从其他电脑挂载时，使用 VPN/组网或 SSH 本地端口转发，再连接客户端自己的 `127.0.0.1:19090`：

```bash
ssh -N -L 19090:127.0.0.1:19090 user@服务器地址
```

Linux Docker 主机如确实需要把原生 FUSE 挂载暴露给宿主机，可显式叠加权限文件：

```bash
mkdir -p mount
docker compose -f docker-compose.yml -f docker-compose.fuse.yml up -d
```

该覆盖文件会授予 `/dev/fuse`、`SYS_ADMIN` 和共享挂载传播，仅应在可信的 Linux Docker 主机使用。默认 Compose 不授予这些权限；Docker Desktop for Windows/macOS 应使用桌面程序原生挂载，而不是尝试把 Linux VM 内的 FUSE 挂载传播到宿主系统。

默认挂载关系为：

- `./watch` → `/watch`：本地备份任务的源目录；
- `./archive` → `/archive`：Docker 版“上传后移动到归档”策略的目录；
- `./media` → `/media`：可选的本地备份允许根目录，不是云盘内原生整理的目标；
- `./docker-data` → `/data`：任务和分享收藏配置。

Docker Web 不能直接读取浏览器所在电脑的任意本地目录。默认 `GUANGYA_FILE_ROOTS=/watch,/archive,/media`，只能浏览和操作明确挂载到这些容器目录中的文件。网页支持扫码和短信验证码登录；也可以在启动前通过 `GUANGYA_TOKEN` 环境变量注入令牌。登录会话和上传历史保存在 `/data/state.sqlite3`。

媒体整理的 TMDB 凭据可在“整理”页配置，也可设置 `TMDB_API_KEY` 或 `TMDB_READ_ACCESS_TOKEN`；`TMDB_LANGUAGE`、`TMDB_IMAGE_LANGUAGE`、`TMDB_API_BASE` 和 `TMDB_IMAGE_BASE` 可调整元数据语言及 API/图片镜像。整理页会从登录账号的云盘文件夹选择 A/B，并通过云端 file ID 递归扫描和执行；Docker 容器不需要把 `/watch` 或 `/media` 映射成整理目标。`/watch`、`/archive`、`/media` 仍只用于本地备份任务和服务器文件上传。

HDHive 联动可在“设置 → HDHive”中配置，也可通过环境变量配置：`HDHIVE_BASE_URL`、`HDHIVE_GUANGYA_SYNC_SECRET`、`HDHIVE_GUANGYA_SYNC_INSTANCE_ID`。设置页会显示并可复制当前实例 ID；未显式设置时会首次生成并持久化到 `/data/state.sqlite3`，密钥不通过状态接口返回。先在 HDHive 管理后台“光鸭同步 → 添加账号”中填写此实例 ID 和已绑定账号的 Telegram 数字 ID，再把后台一次性生成的 HMAC 密钥填回同步端。`GUANGYA_AUTO_SHARE_QUIET_MS` 可调整聚合静默时间，默认 30000 毫秒。

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

输出位于 `release/guangya-sync-native-ubuntu-x64-0.1.28.tar.gz`，解压后执行 `sudo ./install.sh`。安装包自带 Node.js 24 Linux 运行时和全部生产依赖。安装器会生成强随机管理密码、以 `0600` 权限写入 `/etc/guangya-sync.env`，并且只在首次生成时显示一次。Ubuntu 原生版默认只允许网页浏览 `/var/lib/guangya-sync/watch` 和 `/var/lib/guangya-sync/archive`；需要增加其他目录时使用 `GUANGYA_FILE_ROOTS` 设置白名单。应用自己的 `DATA_DIR` 始终隐藏，避免误选并上传包含登录会话的状态库。详细说明见包内 `README.md`。

## 接口边界

光鸭云盘目前没有公开稳定的第三方 API 承诺。本项目按 2026-08-01 的 PC 实测接口契约接入，默认使用 Windows PC OAuth profile；接口、字段和风控要求可能变化。上传凭证只用于服务端返回的目标 OSS 桶和对象路径，不写日志、不落盘。

完整的活跃 UI → Web/Tauri → 光鸭上游接口矩阵、响应判定和实测边界见 [接口对接总账](./docs/API_INTEGRATION.md)。
