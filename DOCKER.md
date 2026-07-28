# Docker Web 部署配置

本文档适用于 Docker Hub 镜像 `94xhzy/guangya-sync:0.1.15`。容器提供光鸭云盘 Web 管理界面、服务器目录监控、断点续传、自动分享与 HDHive 投稿。

## 1. 准备目录和配置

服务器需要 Docker Engine 24+ 与 Docker Compose v2。把仓库中的 `docker-compose.yml` 和 `.env.example` 放到同一目录：

```bash
mkdir -p guangya-sync/{docker-data,watch,archive}
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
GUANGYA_IMAGE=94xhzy/guangya-sync:0.1.15
GUANGYA_HTTP_PORT=8080
GUANGYA_ADMIN_USERNAME=admin
GUANGYA_ADMIN_PASSWORD=替换为上面生成的强随机密码
```

`GUANGYA_ADMIN_PASSWORD` 为空时 Compose 会拒绝启动。`.env` 包含敏感信息，不要提交到 Git，也不要发到聊天或工单。

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
| `./watch` | `/watch` | 可浏览和创建监控任务的源目录 |
| `./archive` | `/archive` | “上传后移动到归档”策略的目标目录 |

必须持久化 `/data`。升级或重建容器不会丢数据，但删除 `docker-data` 会清除登录会话、上传指纹和任务配置。

需要监控其他宿主机目录时，在 `docker-compose.yml` 增加只读或读写挂载，并把容器路径加入 `GUANGYA_FILE_ROOTS`。例如：

```yaml
environment:
  GUANGYA_FILE_ROOTS: /watch,/archive,/media
volumes:
  - /srv/media:/media:ro
```

不要把 `/data` 加入 `GUANGYA_FILE_ROOTS`，也不要让多个容器同时使用同一个 `/data`。

## 4. 光鸭登录

推荐启动后在网页中扫码或使用验证码登录，登录状态会保存在 `/data/state.sqlite3`。

也可以通过 `.env` 注入已有令牌：

```dotenv
GUANGYA_TOKEN=
GUANGYA_TOKEN_REFRESH_MS=1200000
```

令牌为空不影响容器启动。不要把真实令牌写进 `docker-compose.yml` 或提交到仓库。

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
GUANGYA_IMAGE=94xhzy/guangya-sync:0.1.15
```

当前 `0.1.15`/`latest` 发布摘要：

```dotenv
GUANGYA_IMAGE=94xhzy/guangya-sync@sha256:aa5e855d3c2800095335a73bad03dcdb07c32cc355b1a192852da228131d741a
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
docker image inspect 94xhzy/guangya-sync:0.1.15
```
