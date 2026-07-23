# macOS 打包说明

这份源码已经包含 macOS 专用的 Tauri 打包配置，Apple Silicon（M1/M2/M3/M4）可以直接构建 ARM64 应用。

## 1. 安装系统依赖

```bash
xcode-select --install
```

安装 Node.js 20 或更高版本、Rust stable，然后启用项目锁定的 pnpm 版本：

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
