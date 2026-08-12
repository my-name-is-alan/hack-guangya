# App 接口对接总账

> 最后核对：2026-08-04
>
> 上游基线：光鸭 PC 1.0.2 逆向资料与同日脱敏实测样本
>
> App 基线：当前 `ui/main.ts` → `ui/RootApp.vue` 活跃界面、`ui/bridge.js`、Tauri/Rust 与 Docker Web/Node 实现

本文是本项目的接口源真值索引。排查一个按钮时，应沿着“活跃 UI → bridge 命令或 Web 请求 → Tauri handler / Web route → 光鸭上游或本地服务”逐列核对。旧的 `ui/App.vue` 不在当前 Vite 入口中，因此不作为活跃 UI 契约；仍保留的兼容入口会单独标注。

PC OAuth 凭据与官方开发者凭据是两套独立身份：前者供登录画像使用；后者由用户在“设置 → 多号秒传 → Token 配置”填写，并在绑定当前账号后用于 `dapi.guangyapan.com`。本文不记录任何真实值。Bearer/refresh token、两类 `client_secret`、接收 TOKEN、HDHive 密钥、WebDAV 密码、设备 ID、OSS 临时密钥和已签名下载地址都属于运行时敏感数据，不得写入文档、日志或故障单。

## 1. 证据级别与来源优先级

矩阵使用以下标记：

| 标记 | 含义 |
|---|---|
| **L** | `api_map` 或当前仓库的隔离验证实例在 2026-08-01 使用 PC 1.0.2 当前登录态做过脱敏实测；只代表样本覆盖的请求形态、账号权益与结果。 |
| **P** | 来自同版本官方 PC 1.0.2 安装包中实际界面调用点；可以确认 method、payload 和产品语义，但仍不等于当前账号成功实测。 |
| **S** | 来自 PC 类型、APK 字符串、OpenAPI 或静态逆向，尚无对应的成功实测样本。 |
| **D** | 来自光鸭《TOKEN 上传 API 文档》v1.0（2026-07-28 首发，2026-08-03 更新）的公开开发者契约；尚未用真实开发者凭据做生产写入验证。 |
| **R** | 当前仓库已经实现该调用链并接受源码/自动化测试核对；不等于生产账号实测。 |
| **Local** | App 本地状态、文件系统、SQLite、SSE/Tauri event、WebDAV/rclone 或访问控制，不调用光鸭业务 API。 |
| **3P** | HDHive 集成契约，不属于光鸭 API。 |

来源优先级从高到低：

1. 对开发者 TOKEN 接口，优先使用光鸭《TOKEN 上传 API 文档》v1.0；
2. `H:\Soft Ware\hack_guangya_逆向\api_map\SUMMARY.md` 与 `API_COMPLETE.md` 的 2026-08-01 实测结论；
3. `api_map/live_samples/` 的脱敏响应；
4. 同一份官方 PC 1.0.2 安装包中 `app.asar` 的实际调用点；
5. 当前仓库的 Rust/Node 实现和测试；
6. `API_DETAILED.md`、`openapi.json`、`paths*.json`、`err_codes.json` 的静态信息。

当旧静态资料与新实测或同版本产品调用点冲突时，以新实测优先；没有写操作实测时，以产品调用点确认其语义。例如 Windows 当前使用 `dt: 5`，不能沿用旧草稿中的其它值。`delete_file` / `recycle_file` 的产品语义也不能只按旧常量说明猜测。

## 2. 总体调用边界

```text
Vue 活跃界面
  └─ ui/bridge.js
      ├─ Tauri：invoke(command) → src-tauri/src/main.rs / webdav.rs
      └─ Docker Web：HTTP /api/* → server/server.mjs / webdav.mjs
             ├─ account.guangyapan.com：OAuth、验证码、用户资料
             ├─ api.guangyapan.com：文件、上传任务、分享、离线、资产
             ├─ dapi.guangyapan.com：开发者文件读取、预审、TOKEN 秒传
             ├─ OSS 临时 endpoint：实际上传字节
             ├─ *.guangyacdn.com：实际下载字节
             └─ 已配置的 HDHive：投稿事件与回执
```

`get_` 开头不代表 HTTP GET。除账号资料 `GET /v1/user/me`、OSS/CDN 数据通道和 HDHive 回执查询外，当前光鸭业务接口均按 **POST + JSON** 调用。

## 3. 光鸭公共协议

### 3.1 域名和数据通道

| 用途 | 地址/来源 | 说明 |
|---|---|---|
| 账号与 OAuth | `https://account.guangyapan.com` | 登录、刷新、验证码、当前用户。 |
| 业务 API | `https://api.guangyapan.com` | 文件、资产、上传任务、分享、离线和打包任务。 |
| 开发者 API | `https://dapi.guangyapan.com` | 使用独立 `client_id/client_secret` 签名；文件读取、预审与接收 TOKEN 秒传。 |
| 下载 | 上游返回的 `*.guangyacdn.com` 签名 URL | `get_res_download_url` 实测有效期 `21600` 秒；App 不应把 URL 持久化为永久地址。 |
| 上传 | `get_res_center_token` 返回的 `fullEndPoint`/`objectPath` | 使用短期 OSS STS；优先 `fullEndPoint`，凭据仅驻留在上传过程。 |
| HDHive | 用户配置的 HTTP(S) Base URL | 第三方集成，不复用光鸭 Bearer。 |

### 3.2 Windows 业务请求头

当前桌面和 Web 服务都应生成同一份 Windows PC 请求画像：

```http
Authorization: Bearer <access-token>
Content-Type: application/json
Accept: application/json
dt: 5
av: 1.0.2
vc: 1002
x-client-id: <windows-client-id>
x-device-id: <32-lowercase-hex>
User-Agent: GuangyapanPC/1.0.2
```

- `x-client-id` 的值由受控配置提供，本文不记录。
- `x-device-id` 要稳定保存，格式是 32 位小写十六进制；`did` 仅作为当前实现保留的兼容别名。
- `traceparent` 可由客户端按请求生成，不属于业务身份。
- OAuth token/device/refresh 请求需要客户端凭据字段，但不得把值输出到 UI、日志、文档或错误消息。
- 业务与账号请求在 Web/Tauri 两端统一使用 30 秒请求超时；OSS 字节上传单独使用更长的 600 秒超时，避免把大文件传输误当作普通 API。

账号域请求不使用业务 `dt` 头。当前两端统一发送 JSON Content-Type/Accept、`x-client-id`、同一个持久 `x-device-id`、`x-client-version: 1.0.2`、`x-sdk-version: 9.0.2`、`x-protocol-version: 301`、`Accept-Language: zh-CN` 和 PC User-Agent；风控要求时再加 `X-Captcha-Token`。`GET /v1/user/me` 额外携带 Bearer。扫码、短信、refresh 和 user/me 必须共用同一 device ID，不能每次请求随机生成。

### 3.3 业务响应成功判定

已观察到的成功形态：

```json
{ "code": 0, "msg": "success", "data": {} }
```

以及：

```json
{ "msg": "success", "data": {} }
```

统一判定规则：

1. HTTP 必须是 2xx，除非调用点明确把某个业务码列为可处理状态；
2. 有 `code` 时必须能解析为整数，`code === 0` 才是一般成功；
3. 无 `code` 时，`msg` 必须是 `success`/`ok`；
4. Web 与 Tauri 为兼容旧生产响应，只有在 **同时没有 code、没有矛盾 msg，且 data 非空** 时才接受 data-only 成功；显式 `code: 0` 也可省略 msg；
5. 显式的失败 `msg` 即使带 `data` 也不能当成功；
6. 非 JSON、无法解析的业务码，以及既无 code、成功 msg 也无 data 的响应必须失败，不能静默返回空数据。

### 3.4 鉴权失效和刷新

| 情况 | 含义 | App 行为 |
|---|---|---|
| HTTP `401` / code `117` | Bearer 无效 | Web 请求层或 Tauri bridge 有 refresh token 时刷新一次并重放原请求；刷新失败才清会话并回到登录。 |
| code `110` | 未登录 | 与 117 进入同一平台分支。 |
| code `118` | token 已过期 | 与 117 进入同一平台分支。 |
| Device Flow `authorization_pending` | 用户尚未确认 | 保持登录页并按服务端间隔轮询。 |
| Device Flow `slow_down` | 轮询过快 | 延长下一次轮询间隔。 |

刷新使用账号域 `POST /v1/auth/token`、`grant_type=refresh_token`：

- Web 在业务请求发现 HTTP 401 或 110/117/118 时，最多刷新并重放一次；失败后清 access/refresh token；
- Tauri 业务 command 返回统一登录失效错误后，活跃 bridge 调用一次 `refresh_session`，成功则把原 command 重放一次；再次失效或刷新失败才调用 `clear_expired_session`；
- Tauri 也会在应用恢复和后台约 20 分钟循环中调用 `refresh_saved_session`。上传 worker 仍使用自己的持久化恢复路径；
- refresh 接口返回 400/401/403 时，两类 token 都必须作废。

UI 通过 bridge 的登录失效通知统一清空账号概览和文件状态。任何自动重放都必须有一次上限，避免失效凭据造成无限循环。

### 3.5 高频业务码

| code | 含义 | 当前处理原则 | 证据 |
|---:|---|---|---|
| `0` / 无 code + success | 成功 | 正常解包 `data`。 | L |
| `100`–`103` | 网络/内部/RPC/超时 | 有界退避重试；保留原任务。 | S/R |
| `110` / `117` / `118` | 未登录/无效 token/token 过期 | Web 请求层和 Tauri bridge 都最多刷新并重放一次；117 已实测可伴随 HTTP 401。 | 117=L；110/118=S |
| `112` | 参数错误 | 检查 `page/pageSize/fileId/gcid` 等必填字段；不盲重试。 | L |
| `143` / `146` | 文件不存在 | 丢弃陈旧 ID，重新 list；不盲重试。 | 143=L；146=S |
| `145` / `152` / `155` / `163` | 上传任务不存在、删除或过期 | 旧任务不可继续，清理 checkpoint 并重新申请上传任务。 | S/R |
| `147` | 文件仍在上传/入库 | **仅作为上传确认 pending**；继续有界轮询，不能标记完成。 | L |
| `156` | 已完成/秒传命中 | 跳过 OSS，但仍进入入库确认，拿到最终 `fileId` 后才完成。 | L/S |
| `157` / `172` / `90000` | 空间、月度或非会员文件大小限制 | 停止该项并给用户明确提示。 | S |
| `159` | 同名目录已存在 | 建目录竞争时重新 list 并定位目录 ID。 | S/R |
| `160` | 同名文件已存在 | GCID 导入会核对同名项类型和大小；完全一致可记为 existing，否则报告冲突。 | S/R |
| `164` / `248` | VIP 限制 | 明确提示会员限制，不重试。 | S |
| `200`–`213` | 分享不存在、失效、提取码、访问 token、转存限制等 | 保留具体 code/msg；分享下载的 205/206/207 不伪装为通用网络错。 | 205=L；其余 S/R |
| `241`–`251` | 直链/流量限制或对象类型错误 | 提示到官方页或补充流量，不能自动绕过；设置目标必须是根级文件夹，获取目标必须是其内部文件。 | 241/242/245=L；其余 S/R |
| HTTP `429` / `5xx` / 网络错误 | 限流或临时故障 | 有界退避重试；不可无限循环。 | R |

### 3.6 官方开发者签名与 TOKEN 上传

开发者请求固定使用 `POST + JSON`，每次重新生成签名头：

```http
client_id: <developer-client-id>
nonce: <16-32-char-unique-value>
timestamp: <unix-seconds>
sign: <lowercase-hex>
Content-Type: application/json
```

签名源串和算法必须严格为：

```text
src = "client_id=" + client_id
    + "&client_secret=" + client_secret
    + "&nonce=" + nonce
    + "&timestamp=" + timestamp

sign = lower_hex(SHA512(MD5_binary(src)))
```

第二步 SHA-512 的输入是 **MD5 的 16 字节二进制摘要**，不是 32 字符 MD5 十六进制文本。`nonce` 每次请求唯一，时间戳与服务端偏差不得超过 300 秒。完整 `client_secret` 只从本机状态库或环境变量读取，不进入 bridge 返回、SSE、日志和错误文本。Rust 与 Node 使用同一固定向量测试签名结果。

开发者高频业务码：`18001` TOKEN 不存在、`18002` TOKEN 已绑定其他上传者、`18003` 上传者与接收者相同、`18006` 文件不属于当前开发者、`18007` 接收空间不足、`18008` 目标目录不存在、`18009` 任务/凭据不匹配、`18010` 频率限制、`18011` 暂无已通过预审的文件、`18012` 超过 20 项、`18013` 服务繁忙、`18014` 文件已传过、`18020` 凭据无效、`18021` 签名失败、`18022` 签名过期、`18023` nonce 重用、`18025` 接口未授权、`18026` 开发者受限。只有 `18010`、`18013`、HTTP 429/5xx 与网络故障进入有界重试；`18011` 进入预审兜底，`18014` 作为“目标已存在”完成，其余直接终止并保留业务码。

## 4. 活跃 UI 到接口的完整矩阵

表中“Web route”为 Docker Web 服务的本地 HTTP 契约；“Tauri handler”是桌面端命令。`—` 表示该端不需要该层，而不是遗漏实现。

### 4.1 启动、访问控制、登录和概览

| UI 能力 | bridge / 直接请求 | Tauri handler | Web route | 上游或本地实现 | 状态/备注 |
|---|---|---|---|---|---|
| 启动状态 | `get_state` | `get_state` | `GET /api/state` | 本地内存、配置和 SQLite 快照 | Local/R；含队列、mapping、收藏、HDHive 与回执。 |
| 状态推送 | `bridge.subscribe` | `sync-event` | `GET /api/events`（SSE） | 本地事件总线 | Local/R。 |
| Web 管理页访问状态 | `get_access_status` | —（桌面直接放行） | `GET /api/access/status` | 本地访问码/cookie | Local/R；不是光鸭登录。 |
| 解锁 Web 管理页 | `unlock_access` | — | `POST /api/access/unlock` | 本地访问码校验、限速和 session cookie | Local/R。 |
| 修改 Web 访问码 | `update_access_code` | — | `POST /api/access/code` | 本地访问控制配置 | Local/R。 |
| 清理失效光鸭会话 | `clear_expired_session` | `clear_expired_session` | Web 在 API 失效分支内直接清理 | 删除本地 access/refresh token 和缓存 | Local/R。 |
| 获取扫码登录信息 | `bridge.login()` | `start_device_login` | `POST /api/auth/device/start` | account `POST /v1/auth/device/code` | S/R；body 含 `scope:user`、`meta.scene:pc_login`，不记录客户端凭据值。 |
| 轮询扫码结果 | `poll_device_login` | `poll_device_login` | `POST /api/auth/device/poll` | account `POST /v1/auth/token`，device-code grant | S/R；处理 pending/slow_down，成功后保存 access/refresh。 |
| 发送短信验证码 | `request_sms_code` | `request_sms_code` | `POST /api/auth/sms/send` | account `POST /v1/shield/captcha/init` → `POST /v1/auth/verification` | S/R；发码固定 `usage:SIGN_IN`、`selected_channel:VERIFICATION_PHONE`。 |
| 短信登录/注册 | `login_with_sms` | `login_with_sms` | `POST /api/auth/sms/login` | account `POST /v1/auth/verification/verify` → `/v1/auth/signin` 或 `/v1/auth/signup` | S/R；以发码返回的 `is_user` 选择 signin/signup。 |
| 自动续期 | bridge 内部 `refresh_session` | 应用恢复/后台 `refresh_saved_session` + bridge 单次重放 | 业务失效分支/后台刷新 | account `POST /v1/auth/token`，refresh-token grant | S/R；两端最多重放一次。 |
| 账号与容量概览 | `get_overview` | `get_overview` | `GET /api/overview` | business `POST /assets/v1/get_assets` + account `GET /v1/user/me` | assets=L；profile=S；资料失败时容量仍可显示。 |
| 独立资产快照 | `get_assets` | `get_assets` | `GET /api/assets` | `POST /assets/v1/get_assets` | L/R；不再把容量、VIP、SVIP 的读取绑在账号资料请求上。 |
| 全局配置（无当前 UI） | `get_global_config` | `get_global_config` | `GET /api/global-config` | `POST /misc/v1/get_global_config` | L/R；设置页已移除原权益规则展示，兼容命令暂保留但没有活跃界面消费者。 |

兼容但非活跃 UI：Web `POST /api/auth` 可注入一个 Bearer token；Tauri `open_login`/`capture_token` 是旧网页登录捕获路径。新功能不要继续依赖这些入口，主流程是扫码或短信登录。

### 4.2 文件浏览、搜索和整理

| UI 能力 | bridge | Tauri handler | Web route | 光鸭上游 | 状态/备注 |
|---|---|---|---|---|---|
| 当前目录列表/云端目录选择器 | `list_files` | `list_files` | `GET /api/files` | `POST /userres/v1/file/get_file_list` | L/R；固定带 `page/pageSize`；根目录当前用空 `parentId`，`"*"` 是跨目录全库流；目录选择器额外传 `resType:2`，确保分页只统计目录。 |
| 新建文件夹 | `create_folder` | `create_folder` | `POST /api/files/create-folder` | `POST /userres/v1/file/create_dir` | L/R；body 为 `parentId/dirName`，可选 `failIfNameExist`；当前账号返回目录对象且命令结束后已立即可见。 |
| 文件详情 | `get_file_detail` | `get_file_detail` | `GET /api/files/detail?fileId=` | `POST /userres/v1/file/get_file_detail` | L/R；展示 `fileInfo` 与可用的 `location`，143 不重试。 |
| 云端最近记录 | `list_recent_actions` | `list_recent_actions` | `GET /api/recent` | `POST /userres/v1/get_user_action` | L/R；首次 `cursor:""`，游标按不透明字符串透传；UI 展开 `actionDetails`。 |
| 全局关键字搜索（可叠加类型/后缀） | `search_files` | `search_files` | `GET /api/search` | `POST /userres/v1/file/search_files`，叠加筛选时跨页拉取后本地过滤 | S/R；请求字段为 `name/page/pageSize`。 |
| 无关键词的类型/后缀搜索 | `search_files` | `search_files` | `GET /api/search` | `POST /userres/v1/file/get_file_list`，`parentId:"*"` + `resType/fileTypes`，必要时本地精确过滤 | list=L、组合逻辑=R；分页基于过滤后结果；远端尚未耗尽时 `total` 是“仍有下一页”的下界，耗尽后才是精确总数。 |
| 复制 | `copy_files` | `copy_files` | `POST /api/files/copy` | `POST /userres/v1/file/copy_file` → `POST /userres/v1/get_task_status` | L/R；等待异步任务完成后才向 UI 返回成功；实测副本获得新 ID。 |
| 移动 | `move_files` | `move_files` | `POST /api/files/move` | `POST /userres/v1/file/move_file` → `POST /userres/v1/get_task_status` | L/R；实测移动后 ID 保持不变，源目录不再出现。 |
| UI 删除 | `delete_files` | `delete_files` | `POST /api/files/delete` | `POST /userres/v1/file/delete_file` → `POST /userres/v1/get_task_status` | L/R；普通文件上下文的产品语义是移入回收站。旧静态说明曾把两个 endpoint 的语义写反。 |
| 回收站列表 | `list_recycle_files` | `list_recycle_files` | `GET /api/recycle` | `POST /userres/v1/file/get_file_list` | L/R；固定 `dirType:4`、`orderBy:12`、`sortType:1`，而不是依赖只有空样本的 `get_restore_list`。 |
| 从回收站还原 | `restore_files` | `restore_files` | `POST /api/recycle/restore` | `POST /userres/v1/file/recycle_file` → task status | L/R；body 为 `fileIds`，完成后同时刷新原目录和回收站。 |
| 回收站彻底删除 | `permanently_delete_files` | `permanently_delete_files` | `POST /api/recycle/delete` | `POST /userres/v1/file/delete_file` → task status | L/R；与普通删除是同一个上游 endpoint，但文件当前所处上下文不同；UI 必须二次确认。 |
| 清空回收站 | `clear_recycle_bin` | `clear_recycle_bin` | `POST /api/recycle/clear` | `POST /userres/v1/file/clear_recycle_bin` → task status | P/R；空 body；收到 `taskId` 不等于清空完成。 |
| 批量重命名 | `batch_rename_files` | `batch_rename_files` | `POST /api/files/rename-batch` | 每项两阶段调用 `POST /userres/v1/file/rename` | L/R；文件和文件夹单项重命名实测 ID 保持不变；多项名称交换的两阶段与回滚由仓库测试覆盖。 |
| 选择本地保存目录 | `bridge.selectFolder()` | `select_folder` | — | 本地文件选择器 | Local/R；Web 下载由浏览器打开签名 URL。 |

文件字段约定：

- `fileId`、`parentId`、`taskId`、`shareId` 必须按字符串透传，不能转成 JavaScript Number；
- `resType: 1` 是文件，`resType: 2` 是目录；
- `fileType` 已知值：0 未知、1 图片、2 视频、3 音频、4 文档、5 压缩包、6 字幕、7 字体、8 安装包、9 种子、10 代码、11 其它；
- `get_file_list` 缺 `page` 或 `pageSize` 已实测返回 112；
- `parentId:"*"` 是首页式跨目录文件流，不等同于层级根目录；具体目录用真实 ID，根目录层级浏览当前使用空字符串。

### 4.3 手动上传、目录上传和 GCID 导入

| UI 能力 | bridge / 直接请求 | Tauri handler | Web route | 上游链 | 状态/备注 |
|---|---|---|---|---|---|
| 选择文件/文件夹 | `selectUploadFiles` / `selectUploadFolder` | `select_upload_files` / `select_upload_folder` | — | 本地文件选择器 | Local/R。 |
| 桌面上传入队 | `queue_upload_paths` | `queue_upload_paths` | — | 本地队列，随后走统一上传链 | R。 |
| 浏览器本机文件上传 | CloudView XHR | — | `POST /api/upload?parentId&fileName&relativePath&lastModified` | 先安全落到受控临时目录，再走统一上传链 | R；HTTP 202 只表示已入队，不表示云端完成。 |
| Web 服务端文件浏览 | CloudView `fetch` | — | `GET /api/server-files?path=` | 受 `GUANGYA_FILE_ROOTS` 限制的本地文件系统 | Local/R；不允许任意路径逃逸。 |
| Web 服务端文件入队 | CloudView `fetch` | — | `POST /api/server-upload` | 本地队列，随后走统一上传链 | R。 |
| 选择 GCID JSON | `select_gcid_import_file` | `select_gcid_import_file` | — | 本地文件选择器 | Local/R；桌面专属。 |
| 粘贴 JSON 暂存 | `stage_gcid_import_text` | `stage_gcid_import_text` | — | 受限大小的本地暂存文件 | Local/R。 |
| 解析 GCID 导入 | `prepare_gcid_import` | `prepare_gcid_import` | — | 本地 SQLite 导入任务 | Local/R。 |
| 查询/启动 GCID 导入 | `get_gcid_import_status` / `start_gcid_import` | 同名 handler | — | 建目录 → token → flash → 上传任务确认 | L/R；支持光鸭导出和带 `containsCid:true` 的 PikPak GCID/CID 导出，文件必须提供 GCID/CID（两者统一为大写十六进制）；只导入可秒传项目，未命中或 flash 返回 112 时记录为 missed，不上传本地字节。 |
| 独立 GCID 批量导入 CLI | `node scripts/import-guangya-gcid.mjs` | — | — | 读取桌面状态库 → 建目录 → token → flash → 上传任务确认 | L/R；与主 App 共用 Windows 协议模块、30 秒 API 超时和 110/117/118 刷新规则；GCID/CID 统一为大写十六进制，只有 147 继续轮询。 |
| 选中云端内容生成秒传 JSON | `export_gcid_json` | `export_gcid_json` | `POST /api/files/export-gcid` | 大库全库分页索引/小目录递归 → 复用详情中的 GCID → Range 采样计算 CID | R；桌面与 Web 对超过 500 个子目录的大库使用 `parentId:"*"`、每页 1000 条的文件/目录索引并发加载，再按 `fullParentIds` 筛选并重建相对路径，当前 1.5 万目录库从约 1.5 万次目录请求降为约 32 次分页请求；小目录仍最多并发扫描 24 个文件夹。文件并发处理 20 个，每个文件内部最多并发读取 3 个范围，但所有文件合计最多同时发出 24 个 CDN Range 请求。相同账号与选择会持久化完整导出快照；10 分钟新鲜窗口内先校验根目录 `utime`、递归总大小、子目录数和文件数，签名相同则直接复用快照。顶层 `utime` 不会随所有后代变化而递归更新，因此不能单独作为缓存命中依据；快照过期或聚合签名变化时重新扫描，但仍按 fileId、大小和 GCID 复用未变化文件的 CID。扫描根详情和索引分页对网络错误、超时、HTTP 408/425/429/5xx 最多做 5 次有界退避重试，登录失效和普通业务错误不重试；桌面业务 API 复用同一 HTTP Client/连接池。大于等于 60 KiB 的文件只读取头、中、尾各 20 KiB；分段读取按范围错峰重试最多 3 次。详情缺少有效 GCID 或 Range 最终失败时只把该文件写入 `skippedFiles`，绝不回退下载整文件；全部失败时才终止。扫描过程实时发送已加载页数和索引条数；采样过程使用稳定阶段名称发送当前路径、文件计数、采样字节与文件总大小进度。单文件夹导出时 `commonPath` 为文件夹名，`files[].path` 相对该文件夹。每次运行同时覆盖写入一份 JSONL 诊断日志，记录扫描分页、缓存命中、文件、Range、重试、HTTP 状态和耗时；签名 URL 查询参数及凭据会脱敏。失败或部分跳过后可在进度抽屉调用 `export_gcid_diagnostic_log`（Web：`GET /api/files/export-gcid-log`）导出日志。 |

统一普通上传链：

1. 用 `get_file_list` 定位已有远程目录；缺少层级时调用 `POST /userres/v1/file/create_dir`。159 表示并发下目录已存在，应重新查询，不能直接失败；
2. `POST /userres/v1/get_res_center_token`，body 至少含 `capacity:2`、`name`、`parentId`、`res.fileSize`；小于 1 MiB 的当前实现还带整文件 MD5；
3. token 直接返回 code 156 时视为秒传候选；大文件按当前网页算法计算大写 GCID 与 CID，再以 `{taskId,gcid,cid}` 调用 `POST /userres/v1/check_can_flash_upload`。CID 取文件头部、中间三分之一处、尾部各 20 KiB（文件小于 60 KiB 时取全文）的拼接 SHA-1；api_map 的小文件样本在该接口返回过 112，因此普通 OSS 回退必须可用；
4. 未秒传时将字节上传到返回的 `fullEndPoint/objectPath`。大文件使用 multipart；STS/断点失效时通过 `POST /userres/v1/get_res_center_resume_token` 刷新；
5. OSS 完成或秒传命中后，轮询 `POST /userres/v1/file/get_info_by_task_id`。147 只表示仍在入库；成功且拿到最终 `fileId` 才能标记 `cloud_confirmed`；
6. 145/146/152/155/163 不是 pending，应清理旧 checkpoint 并重建上传任务；网络类错误做有界退避；
7. 云端确认前再次校验本地源文件的大小、mtime 和身份。复制中的文件继续增长时不得标记完成、自动分享或执行归档/删除，而是等待稳定后的新版本重新入队。

上游证据：`get_res_center_token`、OSS PUT、`get_info_by_task_id` 的 147→成功均为 L；2026-08-11 当前网页上传器的 `check_can_flash_upload` 请求体与 GCID/CID worker 算法为 L；resume token 和成功秒传为 S/R；该接口的小文件 112 分支有 L 样本。

### 4.4 下载

| UI 能力 | bridge | Tauri handler | Web route | 光鸭/数据通道 | 状态/备注 |
|---|---|---|---|---|---|
| 我的文件单文件下载 | `get_cloud_download` | `get_cloud_download` | `POST /api/files/download` | `POST /userres/v1/get_res_download_url` → 签名 CDN GET/Range | L/R；实测字段 `signedURL`、TTL 21600 秒。桌面端对不小于 16 MiB 且支持 `206 Content-Range` 的文件使用有界并发分片，分片失败自动回退单流；Web 打开 URL。 |
| 我的文件夹/多选打包 | `get_cloud_download`，`packaged:true` | 同名 | 同上 | `POST /scheduler/v1/create_packaging_task` → 轮询 `/scheduler/v1/query_packaging_task` → CDN | S/R；当前最长等待 10 分钟。 |
| 接收分享单文件下载 | `get_received_share_download` | `get_received_share_download` | `POST /api/received-share/download` | `POST /userres/v1/get_share_download_url` → CDN | S/R；body 含分享 `accessToken`。205/206/207 等限制应保留具体提示。 |
| 接收分享打包下载 | 同上，`packaged:true` | 同名 | 同上 | scheduler create/query，额外带分享 `accessToken` | S/R。 |

下载签名 URL 是短期能力票据。不得写入长期配置、分享回执或常规日志；重试过期下载时重新申请 URL。

### 4.5 创建分享、分享管理和接收分享

| UI 能力 | bridge | Tauri handler | Web route | 光鸭/其它 | 状态/备注 |
|---|---|---|---|---|---|
| 创建当前资源分享 | `create_share` | `create_share` | `POST /api/share` | `POST /userres/v1/share_file`；可继续投递 HDHive event | L/R；每次创建当前资源的新分享，不把分享当作不可变快照；移动/删除/覆盖可能使旧链接失效。 |
| 我的分享列表与统计 | `list_shares` | `list_shares` | `GET /api/shares` | 分页聚合 `POST /userres/v1/get_share_list` | L/R；不能只显示第一页；UI 展示状态、剩余时间、浏览数和转存数，并支持筛选/批量取消。 |
| 修改分享设置 | `update_share` | `update_share` | `POST /api/shares/update` | `POST /userres/v1/update_share` | L/R；只透传官方产品使用的 `id/validateDuration/downloadType/trafficLimit`；永久有效为 `validateDuration:0`。`downloadType:0` 才使用免登录 `trafficLimit`，`1` 时固定为 `0`；当前账号的普通模式实测 code 0，免登录模式实测 code 205（权益不足），UI 保留原业务错误。 |
| 删除分享记录 | `delete_shares` | `delete_shares` | `POST /api/shares/delete` | `POST /userres/v1/delete_share` | L/R；参数是分享记录 `ids`，不是文件 ID；仅删除本轮临时分享实测成功。 |
| 清理全部失效分享 | `delete_invalid_shares` | `delete_invalid_shares` | `POST /api/shares/delete-invalid` | `POST /userres/v1/delete_invalid_share` | P/R；官方调用为空 body；属于破坏性批量操作，UI 必须确认。 |
| 打开收到的链接 | `open_received_share` | `open_received_share` | `POST /api/received-share/open` | 解析受信域名 → `POST /userres/v1/get_share_access_token` → 拉根目录 | S/R；提取码来自链接 query，access token 只在当前接收会话中使用。 |
| 浏览收到的分享 | `list_received_share_files` | `list_received_share_files` | `POST /api/received-share/files` | 游标分页聚合 `POST /userres/v1/get_share_page_files_list` | S/R。 |
| 转存收到的分享 | `restore_received_share` | `restore_received_share` | `POST /api/received-share/restore` | `POST /userres/v1/restore_share` → `/userres/v1/get_task_status` | S/R；任务完成后才报告成功。 |
| 下载收到的分享 | `get_received_share_download` | 同名 | `POST /api/received-share/download` | 见下载矩阵 | S/R。 |
| 收藏外部分享链接 | `save_share_link` | `save_share_link` | `POST /api/share-links` | 本地配置 | Local/R；不会调用光鸭“收藏”接口。 |
| 移除收藏链接 | `remove_share_link` | `remove_share_link` | `DELETE /api/share-links/{id}` | 本地配置 | Local/R。 |

直链按官方调用点与当前登录态实测补齐：`set_direct_link` / `unset_direct_link` 接收根目录一级文件夹的 `fileId`；`get_direct_link` 接收已开启文件夹内文件的 `fileId + shortLink:boolean`，Web 对应 `/api/direct-link/set|unset|get`。根级文件夹开启和关闭均实测 code 0；错误地对普通文件开启返回 242，对文件夹取链返回 245；对已开启文件夹内的文件取长链和短链均到达账号权益检查并返回 241。UI 因此只在根级文件夹显示开关，在普通文件显示长链/短链获取，并保留具体会员或流量错误。

### 4.6 离线下载

| UI 能力 | bridge | Tauri handler | Web route | 光鸭上游 | 状态/备注 |
|---|---|---|---|---|---|
| 解析云添加资源 | `resolve_offline_resource` | `resolve_offline_resource` | `POST /api/offline/resolve` | `POST /cloudcollection/v1/resolve_res` | L/R；body 为 `{url}`；返回标准化 URL、`resType`，以及链接/磁力/电驴资源摘要，创建前先展示解析结果；普通 HTTPS 资源已用本轮自有临时文件实测。 |
| 创建离线任务 | `create_offline_task` | `create_offline_task` | `POST /api/offline` | `POST /cloudcollection/v1/create_task` | L/R（普通 HTTPS）；解析成功后传标准化 `url/parentId`，可选 `fileIndexes/newName`，不把解析响应的 `resType` 伪造成创建参数；Magnet 子文件索引只使用解析结果中未被排除的有效索引。开启“文件名混淆”后，Magnet/ED2K 跳过云端预解析并默认保存全部文件；原名称从 Magnet `dn` 或 ED2K 链接本地读取，提交前移除 `dn` 或替换 ED2K 内嵌名称，同时把 `newName` 替换为随机安全名称。原名称与 `taskId` 只持久化在本机，任务成功取得 `fileId` 后调用重命名接口恢复。 |
| 查看任务 | `list_offline_tasks` | `list_offline_tasks` | `GET /api/offline` | `POST /cloudcollection/v1/list_task` | L/R；严格使用官方不透明 `cursor`、`pageSize` 与 `status[]`；UI 消费 `cursor/hasMore` 翻页，`page>0` 会在本地拒绝，不能把数字页码伪装成上游能力。上游可能在 `status=2` 后仍返回未收口的 `progress`，因此 UI 以成功状态为准显示 100%。 |
| 离线保护设置 | `get_offline_settings` / `update_offline_settings` | `get_offline_settings` / `update_offline_settings` | `GET/POST /api/settings/offline` | 本地设置 | L/R；持久化 Magnet/ED2K 文件名混淆开关，并返回等待恢复名称的任务数；关闭开关不会丢弃此前已排队的恢复任务。 |
| 取消运行中任务 | `cancel_offline_tasks` | `cancel_offline_tasks` | `POST /api/offline/cancel` | `POST /cloudcollection/v2/delete_task` | L/R；官方 PC 对运行中任务也使用 `{taskIds}` 的 v2 delete，UI 文案明确为“取消任务”；本轮只取消并清理了新建的临时任务。 |
| 清理任务记录 | `delete_offline_tasks` | `delete_offline_tasks` | `POST /api/offline/delete` | `POST /cloudcollection/v2/delete_task` | P/R；与取消同 endpoint，但 UI 只在终态显示“删除记录”。 |
| 重试任务 | `retry_offline_tasks` | `retry_offline_tasks` | `POST /api/offline/retry` | `POST /cloudcollection/v2/retry_task` | P/R；body 为去重后的 `taskIds`；失败、取消或部分完成可重试。 |
| 今日次数统计 | `get_offline_statistics` | `get_offline_statistics` | `GET /api/offline/statistics` | `POST /nd.bizcloudcollection.s/v1/get_task_statistics` | L/R；只展示服务端返回的剩余次数。 |

`resType` 契约：0 普通链接、1 magnet、2 BT 种子、3 ED2K。当前文本输入界面自动识别普通 URL、magnet 和 ED2K；BT 文件解析/上传仍没有经过字段级验证，因此本轮不伪造种子上传入口。

### 4.7 备份、自动分享与 HDHive

| UI 能力 | bridge | Tauri handler | Web route | 实际作用 | 状态/备注 |
|---|---|---|---|---|---|
| 新增备份映射 | `add_mapping` | `add_mapping` | `POST /api/mappings` | 保存本地映射、启动 watcher/扫描；发现文件后走统一上传链 | Local + 上传 S/L/R。 |
| 删除映射 | `remove_mapping` | `remove_mapping` | `DELETE /api/mappings/{id}` | 停 watcher、清待处理本地状态 | Local/R；不删除已上传的云端文件。 |
| 启停映射 | `toggle_mapping` | `toggle_mapping` | `PATCH /api/mappings/{id}` | 本地 watcher 和队列控制 | Local/R。 |
| 修改同步类型 | `update_mapping_sync_types` | 同名 | `PATCH /api/mappings/{id}` | 本地扩展名白名单；可能重新扫描 | Local/R。 |
| 修改监控模式 | `update_mapping_monitor_mode` | 同名 | `PATCH /api/mappings/{id}` | native watcher / polling | Local/R。 |
| 开关自动分享 | `update_mapping_auto_share` | 同名 | `PATCH /api/mappings/{id}` | 本地策略；确认上传后调用 `share_file` 和 HDHive | Local + S/R。 |
| 补建已有上传的自动分享 | `backfill_auto_shares` | `backfill_auto_shares` | `POST /api/mappings/{id}/auto-share-backfill` | 从本地 confirmed 记录调度 `share_file` → HDHive | R；不是重新上传文件。 |
| 保存/开关 HDHive | `update_hdhive_config` | `update_hdhive_config` | `POST /api/hdhive/config` | 保存 Base URL、启用状态和密钥 | Local/3P/R；返回值永不回显密钥。Web 另有 `GET /api/hdhive/config`，活跃 UI 通常从 state 读取。 |
| 重试分享回执 | `retry_auto_share_event` | `retry_auto_share_event` | `POST /api/auto-share/events/{event_id}/retry` | 重新投递原事件，或调用 HDHive retry；可带人工 `tmdb_id/media_type` | 3P/R。 |

HDHive 请求矩阵：

| 场景 | 方法和路径 | 说明 |
|---|---|---|
| 首次投递/原投递失败重发 | `POST /api/integrations/guangya-sync/events` | body 使用稳定 `event_id`，包含 mapping/target、share ID/URL、标题、intent 和 change hint。 |
| 查询回执 | `GET /api/integrations/guangya-sync/events/{eventId}` | 有界轮询，终态为 completed、needs_review 或 failed。 |
| HDHive 业务重试 | `POST /api/integrations/guangya-sync/events/{eventId}/retry` | 人工修正时可传 `tmdb_id` 与 `media_type`。 |

请求使用实例 ID、时间戳和 HMAC 签名头；签名密钥不进入文档、日志或 UI 返回值。手动分享使用 `mapping_id="__manual__"`；备份分享使用真实 mapping ID。UI 以 `event_id` 定位重试记录，以 `target_key` 展示分享目标。`status=needs_review` 且 `error_code=tmdb_required` 是人工补 TMDB 的稳定判据，不应只解析可变中文消息。

### 4.8 光鸭原生媒体识别与整理

| UI 能力 | bridge | Tauri handler | Web route | TMDB/云端依赖 | 状态/备注 |
|---|---|---|---|---|---|
| 读取整理状态 | `get_organizer_state` | 同名 | `GET /api/organizer` | SQLite | Local/R；返回原生引擎版本、TMDB 公开配置、监控映射和最近 100 条任务，不返回密钥。 |
| 保存识别设置 | `update_organizer_settings` | 同名 | `PUT /api/organizer/settings` | SQLite | Local/R；保存语言、匹配阈值、成人内容开关、电影/电视剧完整路径模板和分类值；页面提供三个模板预设；空密钥表示保留旧值，`TMDB_API_KEY` / `TMDB_READ_ACCESS_TOKEN` 等非空环境变量优先。 |
| 测试 TMDB | `test_organizer_connection` | 同名 | `POST /api/organizer/test` | `GET /3/configuration` | 3P/R；v3 Key 使用 `api_key`，Read Access Token 使用 Bearer，不记录完整凭据。 |
| 新增/更新监控 | `add_organizer_mapping` / `update_organizer_mapping` | 同名 | `POST/PATCH /api/organizer/mappings` | 光鸭云盘文件列表 | Local/R；来源 A 是已选择的云端目录，目标 B 必须来自全局“刮削输出”的媒体库目标；目标路径跟随全局配置更新。两者不得相同或互相包含；固定为云端轮询，识别或整理期间拒绝修改。监控不复制规则，每次识别统一读取全局二级分类、辅助识别、搜索和命名设置。 |
| 删除监控 | `remove_organizer_mapping` | 同名 | `DELETE /api/organizer/mappings/{id}` | SQLite/云端列表 | Local/R；识别或整理期间拒绝删除；删除监控和历史记录不删除 A/B 中任何云端文件。 |
| 立即扫描 | `scan_organizer_mapping` | 同名 | `POST /api/organizer/mappings/{id}/scan` | 光鸭云盘文件列表 | Local/R；A 根目录一级文件夹或单个视频为候选项，候选内部递归识别视频、字幕、音轨和花絮。 |
| 执行整理 | `run_organizer_job` | 同名 | `POST /api/organizer/jobs/{id}/run` | 光鸭云盘 copy/move/rename/upload | Local/RW；重新校验源指纹和配置签名后，只执行云盘内复制/移动、冲突策略和可选刮削；不支持硬链接/软链接或跨盘路径；移动/覆盖需确认旧分享失效风险。模板变量同时接受 `{tmdb_id}`/`{tmdbid}`、`{category}`/`{catgroy}` 和 `{Season x}`/`{Expose n}` 别名。 |
| 分享整理结果 | `share_organizer_job` | 同名 | `POST /api/organizer/jobs/{id}/share` | `POST /userres/v1/share_file` | Local/RW；仅允许已完成任务，按预览中的 `share_relative_path` 定位已存在的最终电影/剧集目录并创建新分享，不会创建缺失目录，也不要求用户逐层进入二级分类路径。每次点击生成新链接并可继续投递 HDHive。 |
| 重新识别 | `retry_organizer_job` | 同名 | `POST /api/organizer/jobs/{id}/retry` | TMDB search/details/season | 3P/R；可覆盖标题、年份、`tmdb_id`、媒体类型、季号、集号及结束集号。 |
| 删除单条历史 | `remove_organizer_job` | 同名 | `DELETE /api/organizer/jobs/{id}` | SQLite | Local/RW；运行中的任务不可删除，只删除整理记录，不触碰 A/B 云端文件。 |

整理状态机为 `recognizing → ready → running → completed`；识别歧义或必要信息不足进入 `needs_review`，网络、TMDB 或提交前的云端操作错误进入 `failed`，主体整理成功但图片/NFO 等非关键刮削失败进入 `completed_warning`。自动执行也必须先生成完整预览且匹配分数达到阈值，不能跳过预览直接修改云端文件。识别、候选评分、命名、云盘内文件转移、NFO 与图片刮削由光鸭原生引擎完成；TMDB 只提供元数据。若开启整理后分享，始终从 B 目录创建新分享并通知 HDHive，不复用 A 或历史分享，因为光鸭分享不是不可变快照。

### 4.9 队列、传输、缓存和挂载设置

| UI 能力 | bridge | Tauri handler | Web route | 上游 | 状态/备注 |
|---|---|---|---|---|---|
| 暂停/恢复队列 | `pause_queue` / `resume_queue` | 同名 | `POST /api/queue/pause` / `resume` | — | Local/R；控制本地调度，不撤销已发出的上游请求。 |
| 读取传输设置 | `get_transfer_settings` | 同名 | `GET /api/settings` | — | Local/R。 |
| 更新并发/分片设置 | `update_transfer_settings` | 同名 | `POST /api/settings/transfer` | — | Local/R；上传设置影响 OSS 并发/part size；下载并发表示同时文件任务数，桌面单文件分片会按总 HTTP 连接预算自动分配 1–4 路。 |
| 读取/更新缓存策略 | `get_cache_settings` / `update_cache_settings` | 同名 | `GET/POST /api/settings/cache` | — | Local/R。 |
| 缓存统计/清理 | `get_metadata_cache_stats` / `clear_metadata_cache` | 同名 | `GET /api/cache` / `POST /api/cache/clear` | — | Local/R；清本地目录/GCID 元数据缓存，不删除云端文件。 |
| WebDAV 状态与凭据 | `get_mount_info` / `update_mount_credentials` | 同名 | `GET /api/mount` / `POST /api/mount/credentials` | — | Local/R；WebDAV 认证独立于 Web 管理访问码和光鸭登录。 |
| 原生挂载状态/选项 | `get_native_mount_info` / `update_native_mount_options` | 同名 | `GET /api/mount/native` / `POST /api/mount/native/options` | — | Local/R；rclone/FUSE 配置。 |
| 启停原生挂载 | `start_native_mount` / `stop_native_mount` | 同名 | `POST /api/mount/native/start` / `stop` | — | Local/R；原生挂载连接本机 WebDAV。 |
| 选择挂载点/rclone | `select_native_mount_target` / `select_rclone_binary` | 同名 | Web 返回空（平台限制） | — | Local/R；桌面文件选择器。 |

### 4.10 开发者凭据与小号 TOKEN 秒传

| UI 能力 | bridge | Tauri handler | Web route | 上游/本地实现 | 状态/备注 |
|---|---|---|---|---|---|
| 读取开发者设置 | `get_developer_settings` | 同名 | `GET /api/developer/settings` | SQLite `app_state` + `developer_targets` + 当前 `/v1/user/me` | Local/R；返回模式状态、绑定账号、`client_id`、secret 是否已设置和脱敏 TOKEN，永不回显完整 secret/TOKEN。 |
| 保存开发者凭据 | `update_developer_credentials` | 同名 | `POST /api/developer/credentials` | SQLite；可由环境变量覆盖 | Local/R；空 secret 表示保留旧值；`client_id`/secret 发生变化时清除旧验证并关闭模式。 |
| 验证当前账号 | `test_developer_credentials` | 同名 | `POST /api/developer/test` | 当前 PC 会话与开发者接口分别调用 `get_file_detail` | D/R；双方读取同一个当前账号 `fileId` 才绑定账号；文档的 `FileInfo` 不含账号 ID，因此不能只看列表响应宣称匹配。 |
| 开关开发者模式 | `update_developer_mode` | 同名 | `POST /api/developer/mode` | SQLite `app_state` + 当前 `/v1/user/me` | Local/R；只有已验证账号与当前登录账号相同且验证时的 `client_id` 未变化才能开启。 |
| 新增/更新小号 TOKEN | `upsert_developer_target` | 同名 | `POST /api/developer/targets` | SQLite `developer_targets` | Local/R；编辑时空 TOKEN 表示保留旧值。 |
| 删除小号 TOKEN | `delete_developer_target` | 同名 | `DELETE /api/developer/targets/{id}` | SQLite | Local/R；该目标有进行中任务时拒绝删除。 |
| 开始小号秒传 | `start_developer_transfer` | 同名 | `POST /api/developer/transfers` | 所有权复核 → 直传 → 递归展开原文件并分批预审 → 用原始选择正式秒传 | D/R；仅开发者模式开启时可用；提交前用开发者 `get_file_detail` 复核首个源文件，顶层 `file_ids` 去重且最多 20。18011 分支不改名，递归展开叶子文件后每 20 个一批提交预审并持久化批次任务；单文件或单批失败只累计为未通过，其它批次继续。全部批次结束后仍以原始顶层选择调用正式上传，由平台只复制 pass 项并保留原文件夹结构；最终没有可上传文件时才失败。旧版遗留的名称恢复记录继续兼容恢复。 |
| 查看互传任务 | `list_developer_transfers` | 同名 | `GET /api/developer/transfers` | SQLite + SSE/Tauri event | Local/R；进行中任务随应用启动恢复。 |

上游状态机：

```text
upload_by_fileid
  ├─ accepted + task_id → upload_status: pending/running → success|failed
  ├─ 18014 → 目标已传过，按幂等完成
  └─ 18011 → pre_upload → pre_upload_status: 0/1/2 ... → 3
                                                └────────→ upload_by_fileid
```

预审状态 `0/1/2/3/4` 分别表示未开始、提交中、审核中、完成、失败；查询间隔不短于 3 秒。上传状态查询间隔为 1–3 秒。目标端同名文件由官方自动改名，不覆盖既有内容。任务表只保存源文件 ID/名称、目标配置 ID、上游 task ID、计数和脱敏错误；接收 TOKEN 只保存在目标表中。

开发者模式只为文档明确提供的 `get_file_list` / `get_file_detail` 增加读兜底：先调用当前 PC 登录态的主接口，失败后才检查模式开关、绑定账号、当前账号与已验证 `client_id`，全部一致才调用 `dapi`。创建、移动、复制、重命名、删除等写操作不切换到开发者接口。

官方绑定边界：一个接收 TOKEN 只支持“当前开发者账号 → TOKEN 接收账号”，首次成功使用后可能绑定上传者；发送者与接收者相同返回 `18003`。因此当前 UI 的“小号秒传”是明确的单向通道；真正双向互传必须为反方向再配置一套可执行凭据/TOKEN，不能把单个 TOKEN 宣称成双向能力。

## 5. WebDAV 到光鸭的映射

桌面默认只监听 `http://127.0.0.1:19090/`；Docker/Web 使用独立 WebDAV 端口和 `/dav/` 前缀，管理 API 与 WebDAV 账号不能混用。原生挂载只是让 rclone/FUSE 连接这个本地 WebDAV facade。

| WebDAV 动作 | 本地实现 | 光鸭上游 | 证据/语义 |
|---|---|---|---|
| `PROPFIND` / 目录解析 | 目录缓存、Unicode 安全路径解析 | `POST /userres/v1/file/get_file_list` | L/R；按目录分页拉全。 |
| 浏览器 `GET` 目录 | 生成本地 HTML directory index | 同上 | Local + L/R；不是把目录当文件下载。 |
| `GET` / `HEAD` 文件 | 申请 URL 后代理 Range；If-Match/If-None-Match/时间条件在 facade 用稳定 ETag 和 utime/ctime 本地判断 | `POST /userres/v1/get_res_download_url` → CDN | L/R；CDN 416 原样返回。 |
| `MKCOL` | 创建目录 | `POST /userres/v1/file/create_dir` | S/R。 |
| `PUT` | 写本地临时文件后同步等待统一上传链确认 | token → OSS → task poll | L/S/R；覆盖时先上传临时名，再把旧对象改成备份名、把新对象改成目标名，成功后删除备份；失败时尽力回滚。它仍不是上游原子替换。 |
| `DELETE` | 删除文件/目录并失效缓存 | `POST /userres/v1/file/delete_file` → task status | P/R；普通文件上下文会进入回收站。WebDAV 没有暴露回收站二次删除入口，因此不能把这里描述成无条件永久删除。 |
| `MOVE` | 需要时移动，再重命名 | `move_file` → task status；`rename` | S/R。 |
| `COPY` | 复制；目标名变化时定位副本再重命名 | `copy_file` → task status；可选 `rename` | S/R。 |
| `OPTIONS` / 锁能力 | 明确声明 DAV class 1；不宣称未实现的 class 2 | — | Local/R；`LOCK` / `UNLOCK` 返回 405，避免生成不受写操作约束的假锁。 |

WebDAV 层需要把上游错误翻译成合适 HTTP 状态；不能把所有上游失败都长期压成 400。对挂载客户端可安全重试的网络/5xx 与永久的 112/143/权限错误应保留差异。

## 6. 关键状态与完成门槛

### 6.1 上传

```text
queued/waiting-file
  → token-created
  → flash-hit ───────────────┐
  → OSS multipart complete ──┤
                              └→ cloud poll: 147 ... → success + fileId
                                                       → source recheck
                                                       → cloud_confirmed
                                                       → auto-share/post-process
```

“OSS 成功”“code 156”“进度 100%”都不是业务完成门槛。只有云端返回最终 `fileId` 且源文件没有在上传期间变化，才可写入 confirmed 历史、自动分享、归档或删除本地源文件。

### 6.2 文件操作与打包

- copy/move/delete/restore/clear 的第一响应通常只给 `taskId`；必须轮询 `/userres/v1/get_task_status` 到完成；普通删除和回收站彻底删除使用 `delete_file`，回收站还原使用 `recycle_file`；
- 打包下载使用 scheduler create/query，拿到签名 URL 才可开始真实下载；
- 超时只表示 App 未在窗口内观察到完成，不能伪造成功；用户刷新列表后再以云端状态为准。

### 6.3 分享与 HDHive

```text
share_file 成功
  → 得到 share_id/share_url
  → 保存本地 event_id 回执
  → HDHive POST accepted
  → GET receipt: processing
      ├─ completed
      ├─ needs_review + error_code=tmdb_required → 人工补 TMDB → retry
      └─ failed → retry
```

光鸭分享成功、HDHive 投递失败时必须保留光鸭分享结果和 `delivery_failed` 回执，不能整体显示为“分享失败”。

## 7. UI 依赖的数据语义

### 7.1 资产/VIP

`POST /assets/v1/get_assets` 的关键字段：

| 字段 | 语义 |
|---|---|
| `totalSpaceSize` / `usedSpaceSize` | 字节数；UI 计算剩余空间和百分比。 |
| `vipStatus` | **1 非 VIP、2 VIP 有效、3 VIP 已过期**。不能用 truthy/falsey 判断。 |
| `svipStatus` | 与 `vipStatus` 使用相同的 1/2/3 状态语义；字段缺失时按未开通展示。 |
| `vipExpireTime` | Unix 秒；仅在有值时格式化。 |
| `vipLeftTime` | 剩余秒。 |
| `systemTime` | 服务端 Unix 秒，可用于避免本机时钟偏差。 |
| `highSpeedTraffic.total` / `highSpeedTraffic.remained` | 高速流量总量与剩余量，单位为字节。 |
| `totalDirectLinkTraffic` / `freeDirectLinkTraffic` | 直链流量总量与当前可用量，单位为字节。 |
| `totalShareGuestTraffic` / `freeShareGuestTraffic` | 免登录分享流量总量与当前可用量，单位为字节。 |

账号 ID 从 `/v1/user/me` 的实际资料字段读取并按字符串显示；不要把内部 ID 转成 Number。

### 7.2 下载 URL

- 我的文件实测主字段是 `signedURL`；
- 分享下载静态契约常见 `downloadUrl`/`downloadURL`，实现应兼容 `signedURL`/`signedUrl`，但不得把非 URL 字段当作地址；
- `speedupSignature` 是加速 SDK 凭据，不是播放/下载 URL；
- CDN 支持 Range 的能力由实际响应决定，App 不自行拼签名参数。

## 8. api_map 已实测但当前 App 未直接使用的接口

这些接口不是“接漏”，而是目前没有活跃 UI 消费者；若以后启用，应新增 bridge/handler/route 和针对性测试，而不是在组件里直连：

| 接口 | api_map 结果 | 可用于 |
|---|---|---|
| `POST /userres/v1/file/get_vod_download_url` | L | 在线播放/302，需 `fileId + gcid`。 |
| `POST /misc/v1/get_banner_list` | L（data 可空） | 官方 Banner。 |

## 9. 已知验证边界

1. `api_map` 是逆向与本机样本，不是官方稳定 SLA；L 只覆盖样本中的账号、时间和参数组合。
2. 当前官方登录态的隔离实测已覆盖资产/全局配置/概览、根目录与目录筛选、详情、最近记录、回收站列表、上传与下载签名、文件夹创建、重命名、复制、移动、删除/还原/彻底删除、分享创建/列表/编辑/删除、直链文件夹开关，以及普通 HTTPS 云添加的统计/解析/创建/列表/取消。所有写操作只使用本轮临时对象，结果只记录状态和业务码。
3. HDHive、WebDAV、备份 watcher、访问码、缓存和 rclone 是本项目契约；它们的仓库测试不能替代真实光鸭/HDHive/操作系统挂载验证。
4. Web 与 Tauri 必须保持协议一致，但返回形态可因平台不同而不同：桌面下载写本地文件并发进度事件，Web 返回短期 URL 交给浏览器；桌面选本地路径，Web 只能访问白名单服务端目录或浏览器上传流。
5. 删除语义必须按官方 PC 调用点而不是旧常量注释判断：普通删除和回收站彻底删除都调用 `delete_file`，回收站还原调用 `recycle_file`；三类操作都要等待 task 终态。
6. 离线页面已覆盖 resolve/list/create/cancel/delete/retry/statistics；本轮真实验证仅覆盖普通 HTTPS 资源且最多创建一个任务。Magnet 会从解析结果的 `subfiles` 中排除 `excludeIndices` 后提交 `fileIndexes`，该分支目前由契约测试覆盖；独立 `.torrent` 文件解析和上传的 multipart/body 尚未字段级验证，因此仍不开放种子文件入口。
7. GCID JSON 导入界面目前仅 Tauri 可用，Web bridge 不提供对应命令；仓库另有读取桌面状态库的独立 CLI，但它不是 Web 页面能力。
8. 上传历史、pending task 和自动分享目标当前按 App 实例/数据目录保存，没有把 OAuth 账号 subject 放进主键；主动切换账号前应使用独立数据目录或完成历史隔离/核对，不能默认复用旧账号记录。
9. scheduler 打包查询以“出现签名 URL”为成功条件；字符串失败状态或非零 `errorCode` 会立即终止，其它尚未识别的数字终态仍可能等到 10 分钟超时。
10. WebDAV 只声明实际支持的 DAV class 1，网络/超时/上游 5xx 已映射为 502/504；部分未分类的业务错误仍可能落成本地 HTTP 400。
11. 所有错误必须保留可诊断的 HTTP/业务码，但错误文本不得包含 Bearer、refresh token、OAuth secret、开发者 `client_secret`、接收 TOKEN、HDHive secret、OSS STS 或签名 URL。
12. 本轮未调用账号级的 `clear_recycle_bin` 和 `delete_invalid_share`，因为登录账号原本就有回收站与分享记录；这两个破坏性批量接口继续只保留官方调用点与仓库契约测试证据。免登录分享在当前账号返回 205，文件取直链返回 241，属于当前账号权益限制，不能改写成接口成功或由 App 绕过。
13. 开发者 TOKEN 上传链当前证据为官方文档 + 仓库签名/编译/桥接测试（D/R），未使用真实 `client_id`、接收 TOKEN 做生产写入。发布前应以自有两个测试账号验证 18011→预审→秒传、18014 幂等、重启恢复和 TOKEN 绑定边界；验证期间不得把凭据写入 fixture 或日志。

## 10. 新增或修改接口时的同步清单

1. 先在本文件确认上游 host、method、payload、成功形态和业务码；静态发现先标 S。
2. 同时更新 Rust 与 Node 的公共请求画像、鉴权失效判断和响应解析；不得在 Vue 组件直接调用光鸭域名。
3. 给活跃 UI 增加 bridge 命令；桌面加入 Tauri handler，Web 加同语义 `/api/*` route。平台能力不同必须显式返回限制，不能静默 `null`。
4. 异步上游必须定义“已受理、处理中、业务完成”三个状态；不能在拿到 taskId 或 HTTP 200 时提前完成。
5. 为成功、非 JSON、缺 code、失败 msg、110/117/118、112、143、147 和平台分支补测试。
6. 只有取得脱敏 live sample 后才能把 S 升级为 L；在本文件记录测试日期和精确参数边界。
7. 检查日志、fixture、截图和文档中没有任何凭据或可复用签名 URL。
