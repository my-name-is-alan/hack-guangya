# Docker Web 部署配置

本文档适用于 Docker Hub 镜像 `94xhzy/guangya-sync:0.1.30`。容器提供光鸭云盘 Web 管理界面、服务器目录监控、断点续传、媒体整理、自动分享与 HDHive 投稿。

## 1. 准备目录和配置

服务器需要 Docker Engine 24+ 与 Docker Compose v2。把仓库中的 `docker-compose.yml` 和 `.env.example` 放到同一目录：

```bash
mkdir -p guangya-sync/{docker-data,watch,archive,media}
cd guangya-sync
cp /path/to/docker-compose.yml .
cp /path/to/.env.example .env
chmod 600 .env
```

生成管理密码：

```bash
openssl rand -hex 24
```

把结果写入 `.env`：

```dotenv
GUANGYA_IMAGE=94xhzy/guangya-sync:0.1.30
GUANGYA_HTTP_PORT=8080
GUANGYA_ADMIN_USERNAME=admin
GUANGYA_ADMIN_PASSWORD=替换为上面生成的强随机密码
GUANGYA_WEBDAV_PORT=19090
GUANGYA_WEBDAV_USERNAME=guangya
GUANGYA_WEBDAV_PASSWORD=
TMDB_API_KEY=
TMDB_READ_ACCESS_TOKEN=
TMDB_LANGUAGE=zh-CN
TMDB_IMAGE_LANGUAGE=zh,null,en
# 可选，留空时使用官方地址；也可在网页设置中填写
TMDB_API_BASE=
TMDB_IMAGE_BASE=
```

`GUANGYA_ADMIN_PASSWORD` 为空时 Compose 会拒绝启动。`.env` 包含敏感信息，不要提交到 Git，也不要发到聊天或工单。

WebDAV 与管理界面使用独立凭据。`GUANGYA_WEBDAV_PASSWORD` 可以留空，启动后在网页“设置 → 挂载”中设置；若在 `.env` 中填写，它只用于状态库尚未初始化时的首次配置。

## 2. 启动和检查

```bash
docker compose pull
docker compose up -d
docker compose ps
docker compose logs --tail=100 guangya-sync
```

浏览器打开 `http://服务器IP:8080`。跨公网使用时应限制防火墙来源，并通过 HTTPS 反向代理访问。

容器内端口固定为 `8080`，只修改宿主机端口时设置：

```dotenv
GUANGYA_HTTP_PORT=18080
```

## 3. 数据与目录挂载

默认挂载：

| 宿主机目录 | 容器目录 | 用途 |
| --- | --- | --- |
| `./docker-data` | `/data` | 登录会话、上传记录、断点、分享与任务配置 |
| `./watch` | `/watch` | 本地备份任务的源目录 |
| `./archive` | `/archive` | “上传后移动到归档”策略的目标目录 |
| `./media` | `/media` | 可选的本地备份允许根目录；云盘内原生整理不写入这里 |

必须持久化 `/data`。升级或重建容器不会丢数据，但删除 `docker-data` 会清除登录会话、上传指纹和任务配置。

需要监控其他宿主机目录时，在 `docker-compose.yml` 增加只读或读写挂载，并把容器路径加入 `GUANGYA_FILE_ROOTS`。例如：

```yaml
environment:
  GUANGYA_FILE_ROOTS: /watch,/archive,/media
volumes:
  - /srv/media:/media:ro
```

不要把 `/data` 加入 `GUANGYA_FILE_ROOTS`，也不要让多个容器同时使用同一个 `/data`。

### 3.1 把云盘挂载到宿主机或其他容器

Docker WebDAV 使用独立的容器端口 `19090`。仓库提供的 Compose 固定把它发布到宿主机回环地址，不会直接暴露到公网：

```text
http://127.0.0.1:19090/dav/
```

- 用户名和密码：在“设置 → 挂载”中设置，不复用管理员凭据；
- 密码：至少 12 位，状态接口不会回显；
- 认证：HTTP Basic；
- CRUD：`PROPFIND/PROPPATCH/GET/HEAD/PUT/MKCOL/MOVE/COPY/DELETE`；
- 文件锁：提供兼容系统文件管理器的 `LOCK/UNLOCK`。

管理端口 `8080` 不提供 `/dav/`。请保留 Compose 中这一行的 `127.0.0.1`，不要改成 `0.0.0.0`：

```yaml
ports:
  - "127.0.0.1:${GUANGYA_WEBDAV_PORT:-19090}:19090"
```

Linux 宿主机可以使用 `davfs2`：

```bash
sudo apt-get install -y davfs2
sudo mkdir -p /mnt/guangya
sudo mount -t davfs http://127.0.0.1:19090/dav/ /mnt/guangya
```

需要大文件随机读取或让其他容器共享挂载点时，推荐 rclone：

```bash
rclone config create guangya webdav \
  url http://127.0.0.1:19090/dav/ \
  vendor other \
  user guangya \
  pass "$(rclone obscure '替换为独立WebDAV密码')"

rclone mount guangya: /mnt/guangya \
  --vfs-cache-mode full \
  --dir-cache-time 2s \
  --vfs-cache-poll-interval 5s \
  --poll-interval 0
```

两个挂载点通过同一个 WebDAV 服务访问云端时，WebDAV 写操作会主动失效服务端目录缓存；来自另一个进程或实例的变化则按短 TTL 重新读取。服务端新鲜缓存为 2 秒、过期后台刷新窗口为 15 秒，rclone 目录缓存建议保持 2 秒，通常几秒内可看到另一挂载创建的新文件夹。若客户端仍显示旧目录，先执行挂载客户端自己的刷新或重新进入目录。

在同一个 Compose 网络中的其他容器可直接访问私有地址 `http://guangya-sync:19090/dav/`。如果要在容器内执行 FUSE 挂载，需要显式提供 `/dev/fuse` 和相应权限；普通业务容器优先直接使用 WebDAV，不要无条件开启 `privileged`。

需要从其他电脑挂载时，不要为 WebDAV 配置公网端口或公网反向代理。请使用 VPN/组网，或先建立 SSH 本地端口转发：

```bash
ssh -N -L 19090:127.0.0.1:19090 user@服务器地址
```

随后在客户端连接 `http://127.0.0.1:19090/dav/`。这样 WebDAV Basic 凭据不会直接经过公网明文传输。

WebDAV 直接操作光鸭云端，不会把文件内容永久复制到 `/data`；只有写入过程中的临时文件和现有断点/状态记录会短暂使用 `/data`。因此 `/data` 仍需要足够空间容纳单个正在写入的最大文件。

### 3.2 Linux Docker 原生 FUSE 挂载（显式启用）

镜像内置 rclone 与 FUSE3 用户态组件，但基础 Compose 不授予 `/dev/fuse` 或 `SYS_ADMIN`。只有可信的 Linux Docker 主机需要把挂载目录传播给宿主机时，才叠加仓库提供的覆盖文件：

```bash
mkdir -p mount
docker compose \
  -f docker-compose.yml \
  -f docker-compose.fuse.yml \
  up -d
```

启动后进入网页“设置 → 挂载 → 原生挂载”，目标填写 `/mnt/guangya`，选择只读/读写、VFS 缓存、上传并行、读取并行和缓存上限，再输入当前 WebDAV 挂载密码启动。宿主机目录由 `GUANGYA_NATIVE_MOUNT_ROOT` 控制，默认 `./mount`。

服务端只持久化 WebDAV 密码哈希，因此每次容器重启后启动原生挂载都需要重新输入密码；密码不会写入 rclone 配置。停止容器前应先在菜单中卸载，容器退出也会终止托管进程。

> [!WARNING]
> `docker-compose.fuse.yml` 包含 `SYS_ADMIN`、`/dev/fuse` 和 `apparmor:unconfined`。不要把它作为默认生产配置，也不要用于不可信镜像。Docker Desktop for Windows/macOS 无法用这种方式把 Linux VM 内挂载可靠地传播成宿主机盘符，请改用对应桌面客户端。

## 4. 光鸭登录

推荐启动后在网页中扫码或使用验证码登录，登录状态会保存在 `/data/state.sqlite3`。

也可以通过 `.env` 注入已有令牌：

```dotenv
GUANGYA_TOKEN=
GUANGYA_TOKEN_REFRESH_MS=1200000
```

令牌为空不影响容器启动。不要把真实令牌写进 `docker-compose.yml` 或提交到仓库。

### 4.1 开发者接口与小号 TOKEN

推荐在网页“设置 → 账号 → 开发者模式”中填写当前登录账号自己的开发者凭据，完成同一文件所有权验证后开启模式，再添加小号接收 TOKEN。也可以用环境变量托管开发者凭据：

```dotenv
GUANGYA_DEVELOPER_CLIENT_ID=
GUANGYA_DEVELOPER_CLIENT_SECRET=
```

环境变量存在时对应字段不能从页面覆盖，但不能绕过账号绑定：仍需登录该 `client_id` 所属账号，在账号页验证并开启模式。小号接收 TOKEN 仍在设置页中添加，完整 TOKEN 与 `client_secret` 均不会通过状态接口回显，但会保存在持久化的 `/data/state.sqlite3`，因此应限制 `docker-data` 的读取权限并纳入加密备份策略。一个 TOKEN 只支持当前开发者账号向该小号授权目录发送；反向互传需要另一方向独立配置。

### 4.2 光鸭原生媒体整理

媒体整理由光鸭自身完成，不需要部署 MoviePilot，也不使用容器内本地路径搬运。整理范围严格限定为同一个光鸭云盘内的来源 A 文件夹到目标 B 文件夹；A/B 通过云端目录 ID 选择，不能相同或互相包含。

TMDB 凭据可以在网页“整理”中配置，也可以由环境变量托管；API Key 与 Read Access Token 二选一：

```dotenv
TMDB_API_KEY=替换为TMDB的v3 API Key
TMDB_READ_ACCESS_TOKEN=
TMDB_LANGUAGE=zh-CN
TMDB_IMAGE_LANGUAGE=zh,null,en
```

也可以把 `TMDB_API_KEY` 留空并填写 `TMDB_READ_ACCESS_TOKEN`。凭据只保存在 `/data/state.sqlite3` 或环境变量中，状态接口与事件不会回显完整值；环境变量非空时优先于页面设置。

`/watch`、`/archive`、`/media` 只服务于本地备份任务和服务器文件上传；云盘内原生整理不读取这些容器路径。打开“整理 → 添加目录监控”后，分别从光鸭云盘选择来源 A 文件夹和目标 B 文件夹，系统保存它们的云端目录 ID。

光鸭会每 15 秒递归分析 A 的一级候选（文件夹或视频），忽略样片，识别电影/电视剧、季集、版本和清晰度，查询 TMDB 后生成完整相对路径、转移、字幕/音轨/花絮同步及刮削预览。可选 `copy`（推荐）或云盘内 `move`；`move` 和覆盖冲突策略必须确认已有分享可能失效。执行时先创建 B 下的目标目录，再通过云端 copy/move/rename 完成事务；中途失败会按已记录的步骤回滚，不能回滚的部分会明确告警。

命名模板相对于 B 根目录保存，支持 `{category}`/`{catgroy}`、`{country}`、`{year}`、`{title}`、`{original_title}`、`{tmdb_id}`/`{tmdbid}`、`{season}`、`{episode}`、`{episode_end}`、`{Season x}`、`{Expose n}`、`{edition}`、`{quality}`、`{part}`、`{ext}` 等字段。页面提供三个预设，电影和电视剧模板可分别自由组合；模板必须包含目录和文件名，并拒绝越界路径。

元数据刮削默认关闭。开启后只执行选中的类型，初始预选电影 NFO、剧集 NFO、海报、背景图；单集 NFO、季海报等类型需明确勾选。NFO/图片写入 B 目录失败会进入 `completed_warning`，不会撤销已经成功的媒体文件转移。

如果备份任务没有关联“上传后自动整理”，则继续执行原来的上传完成后自动分享逻辑。关联 A 目录后，上传确认只触发 A 目录扫描；若备份任务打开自动分享，整理完成后才从 B 目录创建新分享并通知 HDHive，绝不会先分享 A。光鸭分享不是不可变快照，移动、删除或覆盖会让旧链接失效，所以整理器不会复用 A 或历史分享。

## 5. HDHive 配置

HDHive 可以在网页“设置 → HDHive”中配置和关闭；也可以使用环境变量：

```dotenv
HDHIVE_BASE_URL=https://hdhive.example.com
HDHIVE_GUANGYA_SYNC_SECRET=替换为HDHive生成的实例密钥
HDHIVE_GUANGYA_SYNC_INSTANCE_ID=每个同步实例使用独立ID
HDHIVE_ALLOWED_HOSTS=hdhive.example.com
GUANGYA_AUTO_SHARE_QUIET_MS=30000
```

- `HDHIVE_ALLOWED_HOSTS` 可填写逗号分隔的主机名或 `主机名:端口`，用于限制投稿目标。
- 每个光鸭账号应使用独立容器、独立 `/data`、独立实例 ID 和密钥。
- 自动分享会等待同一顶层目录的文件全部完成云端入库，并在静默窗口结束后创建或更新分享。

## 6. 上传参数

默认值适合普通网络环境：

```dotenv
GUANGYA_UPLOAD_CONCURRENCY=2
GUANGYA_DOWNLOAD_CONCURRENCY=2
GUANGYA_OSS_TIMEOUT_MS=600000
GUANGYA_OSS_RETRY_MAX=3
GUANGYA_OSS_PARALLEL=3
GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS=600000
GUANGYA_CLOUD_CONFIRM_POLL_MS=1000
GUANGYA_FILE_STABILITY_MS=1200
GUANGYA_FILE_BUSY_RETRY_MS=3000
```

- 上传、下载并发范围为 `1–8`，也能在网页设置中修改。
- `GUANGYA_FILE_STABILITY_MS` 控制文件停止变化多久后开始上传。
- 文件被其他程序占用时，会按 `GUANGYA_FILE_BUSY_RETRY_MS` 重试。
- OSS 分片断点保存在 `/data`；容器重启后会继续未完成的上传。

## 7. 升级、固定版本与回滚

升级到 `.env` 指定的镜像：

```bash
docker compose pull
docker compose up -d
docker compose logs --tail=100 guangya-sync
```

生产环境建议固定版本标签或不可变摘要：

```dotenv
GUANGYA_IMAGE=94xhzy/guangya-sync:0.1.30
```

确认当前 `0.1.30` 与 `latest` 的远端摘要：

```bash
docker buildx imagetools inspect 94xhzy/guangya-sync:0.1.30
docker buildx imagetools inspect 94xhzy/guangya-sync:latest
```

回滚时把 `GUANGYA_IMAGE` 改回已验证的旧标签或摘要，再执行：

```bash
docker compose pull
docker compose up -d
```

## 8. 备份与恢复

停止写入后备份 `/data`：

```bash
docker compose stop guangya-sync
tar -C . -czf guangya-sync-data-$(date +%F-%H%M%S).tar.gz docker-data
docker compose start guangya-sync
```

恢复时先停止容器，替换 `docker-data` 后再启动。不要只复制正在写入的 `state.sqlite3`，应连同 SQLite 的 `-wal`、`-shm` 文件或整个目录一起备份。

## 9. Nginx HTTPS 反向代理示例

```nginx
server {
    listen 443 ssl http2;
    server_name sync.example.com;

    ssl_certificate     /etc/letsencrypt/live/sync.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/sync.example.com/privkey.pem;

    client_max_body_size 0;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;

    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_buffering off;
    }
}
```

同时用防火墙限制宿主机 `8080` 端口只允许本机或可信内网访问。

## 10. 常用命令

```bash
docker compose ps
docker compose logs -f guangya-sync
docker compose restart guangya-sync
docker compose down
docker compose pull
docker image inspect 94xhzy/guangya-sync:0.1.30
```
