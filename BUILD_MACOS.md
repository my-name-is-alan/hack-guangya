# macOS 打包说明

这份源码已经包含 macOS 专用的 Tauri 打包配置，Apple Silicon（M1/M2/M3/M4）可以直接构建 ARM64 应用。

## 1. 安装系统依赖

```bash
xcode-select --install
```

安装 Node.js 24 或更高版本、Rust stable，然后启用项目锁定的 pnpm 版本：

```bash
corepack enable
corepack prepare pnpm@11.15.1 --activate
rustup target add aarch64-apple-darwin
```

## 2. 安装依赖并测试

在解压后的源码根目录执行：

```bash
pnpm install --frozen-lockfile
pnpm ui:test
pnpm server:test
cargo test --manifest-path src-tauri/Cargo.toml
```

## 3. M 芯片打包

```bash
pnpm tauri build --target aarch64-apple-darwin
```

产物位于：

- `target/aarch64-apple-darwin/release/bundle/macos/`
- `target/aarch64-apple-darwin/release/bundle/dmg/`

如果只是本机测试、暂时没有 Apple Developer 签名身份，可以执行：

```bash
pnpm tauri build --target aarch64-apple-darwin --no-sign
```

## 4. 同时支持 Intel 和 M 芯片（可选）

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
pnpm tauri build --target universal-apple-darwin
```

未签名应用只适合本机测试或内部传输；公开分发需要配置 Apple Developer ID，并按 Apple 要求完成签名与公证。

## 5. 挂载云盘目录

推荐先安装 macFUSE 或 FUSE-T，然后在“设置 → 挂载 → 原生挂载”中选择目标目录、只读/读写、缓存和并行参数，点击“开始挂载”。应用包会携带官方 rclone，退出应用时自动卸载。

也可以使用 WebDAV 兼容模式。启动应用并登录后，在“设置 → 挂载”设置独立的 WebDAV 用户名和至少 12 位密码。服务只监听本机，默认地址为 `http://127.0.0.1:19090/`，然后执行：

```bash
mkdir -p "$HOME/Guangya"
mount_webdav "http://127.0.0.1:19090/" "$HOME/Guangya"
```

系统会提示输入挂载页显示的用户名和密码。也可以在 Finder 中选择“前往 → 连接服务器”，粘贴同一地址。卸载时执行：

```bash
umount "$HOME/Guangya"
```
