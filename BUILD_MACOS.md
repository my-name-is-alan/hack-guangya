# macOS 打包与发布

项目通过 `src-tauri/tauri.macos.conf.json` 生成 `.app` 和 `.dmg`。`pnpm package:macos` 是统一入口：没有 Apple 证书时构建 ad-hoc 签名的本机测试包；完整提供 Developer ID 和 Apple 公证凭据时构建签名、公证并 staple 的发布包。

## 1. 安装构建环境

```bash
xcode-select --install
corepack enable
corepack prepare pnpm@11.15.1 --activate
rustup target add aarch64-apple-darwin
pnpm install --frozen-lockfile
```

需要 Node.js 24 或更高版本与 Rust stable。建议打包前先执行：

```bash
pnpm check
pnpm ui:build
```

## 2. 本机测试包

不要设置下文的 Apple 签名变量，直接执行：

```bash
pnpm package:macos
```

脚本会按当前 Mac 选择 `aarch64-apple-darwin` 或 `x86_64-apple-darwin`，并验证 `.app` 签名与 DMG 完整性。无证书时 Tauri 使用 ad-hoc 签名；这类包适合本机测试，不适合公开分发。

首次为某个架构构建时，脚本会下载并校验固定版本的 rclone；后续会复核本地二进制和许可证 hash，校验通过便不联网。离线构建可保留已经校验的 `src-tauri/resources`，或把官方归档放到独立目录后执行：

```bash
GUANGYA_RCLONE_OFFLINE=1 \
GUANGYA_RCLONE_ARCHIVE_DIR=/path/to/rclone-archives \
pnpm package:macos
```

如果要构建 Universal 2，先安装两个 Rust target，再覆盖目标：

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
MACOS_TARGET=universal-apple-darwin pnpm package:macos
```

产物位于 `target/<target>/release/bundle/macos/` 和 `target/<target>/release/bundle/dmg/`。

## 3. Developer ID 正式发布

先在当前用户钥匙串中安装有私钥的 `Developer ID Application` 证书，可用以下命令确认身份名称：

```bash
security find-identity -v -p codesigning
```

在当前终端或 CI 的加密 Secret 中提供变量，不要写入脚本、源码、构建日志或提交到 Git。使用 Apple ID 公证时：

```bash
export APPLE_SIGNING_IDENTITY='Developer ID Application: 组织名 (TEAMID)'
export APPLE_ID='用于公证的 Apple ID'
export APPLE_PASSWORD='Apple 专用密码'
export APPLE_TEAM_ID='Apple Developer Team ID'
pnpm package:macos
```

也可使用 App Store Connect API Key 代替 Apple ID：

```bash
export APPLE_SIGNING_IDENTITY='Developer ID Application: 组织名 (TEAMID)'
export APPLE_API_ISSUER='Issuer ID'
export APPLE_API_KEY='Key ID'
export APPLE_API_KEY_PATH='/安全目录/AuthKey_KEYID.p8'
pnpm package:macos
```

正式模式会拒绝非 `Developer ID Application` 身份或缺项凭据。Tauri 完成签名和公证后，脚本会再对 `.app` 和 `.dmg` 执行 staple/验证，并使用 Gatekeeper 评估 `.app`。

## 4. 发布前人工确认

- 用 `open -n "<应用路径>"` 从 `.app` 启动，检查 Logo、登录、文件选择和窄窗口布局。
- 打开 DMG 后再启动其中的 `.app`，不要只测试 `target` 目录内的应用。
- 确认 `xcrun stapler validate "<app>"` 和 `xcrun stapler validate "<dmg>"` 都成功。
- 计算并发布 DMG SHA-256，不要发布 ad-hoc 或未公证的包。

## 5. 挂载云盘目录

推荐先安装 macFUSE 或 FUSE-T，然后在“设置 → 挂载 → 原生挂载”中选择目标目录、只读/读写、缓存和并行参数，点击“开始挂载”。应用包会携带经过固定 SHA-256 校验的官方 rclone，退出应用时自动卸载。

也可以使用 WebDAV 兼容模式。启动应用并登录后，在“设置 → 挂载”设置独立的 WebDAV 用户名和至少 12 位密码。服务只监听本机，默认地址为 `http://127.0.0.1:19090/`，然后执行：

```bash
mkdir -p "$HOME/Guangya"
mount_webdav "http://127.0.0.1:19090/" "$HOME/Guangya"
```

系统会提示输入挂载页显示的用户名和密码。也可以在 Finder 中选择“前往 → 连接服务器”，粘贴同一地址。卸载时执行：

```bash
umount "$HOME/Guangya"
```
