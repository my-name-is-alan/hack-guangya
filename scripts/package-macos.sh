#!/usr/bin/env bash
set -euo pipefail

if [ "$(uname -s)" != "Darwin" ]; then
  echo "macOS 安装包只能在 macOS 上构建。" >&2
  exit 1
fi

case "$(uname -m)" in
  arm64) default_target=aarch64-apple-darwin ;;
  x86_64) default_target=x86_64-apple-darwin ;;
  *) echo "不支持的 macOS 架构：$(uname -m)" >&2; exit 1 ;;
esac

target="${MACOS_TARGET:-$default_target}"
signing_identity="${APPLE_SIGNING_IDENTITY:-}"
has_apple_id_credentials=false
has_api_key_credentials=false

if [ -n "${APPLE_ID:-}" ] || [ -n "${APPLE_PASSWORD:-}" ] || [ -n "${APPLE_TEAM_ID:-}" ]; then
  if [ -z "${APPLE_ID:-}" ] || [ -z "${APPLE_PASSWORD:-}" ] || [ -z "${APPLE_TEAM_ID:-}" ]; then
    echo "Apple ID 公证需要同时设置 APPLE_ID、APPLE_PASSWORD 和 APPLE_TEAM_ID。" >&2
    exit 1
  fi
  has_apple_id_credentials=true
fi

if [ -n "${APPLE_API_ISSUER:-}" ] || [ -n "${APPLE_API_KEY:-}" ] || [ -n "${APPLE_API_KEY_PATH:-}" ]; then
  if [ -z "${APPLE_API_ISSUER:-}" ] || [ -z "${APPLE_API_KEY:-}" ] || [ -z "${APPLE_API_KEY_PATH:-}" ]; then
    echo "App Store Connect API 公证需要同时设置 APPLE_API_ISSUER、APPLE_API_KEY 和 APPLE_API_KEY_PATH。" >&2
    exit 1
  fi
  if [ ! -f "$APPLE_API_KEY_PATH" ]; then
    echo "APPLE_API_KEY_PATH 不存在：$APPLE_API_KEY_PATH" >&2
    exit 1
  fi
  has_api_key_credentials=true
fi

if [ -z "$signing_identity" ] && { [ "$has_apple_id_credentials" = true ] || [ "$has_api_key_credentials" = true ]; }; then
  echo "已提供公证凭据，但缺少 APPLE_SIGNING_IDENTITY。" >&2
  exit 1
fi

release_build=false
if [ -n "$signing_identity" ]; then
  case "$signing_identity" in
    "Developer ID Application:"*) ;;
    *) echo "正式分发必须使用 Developer ID Application 签名身份。" >&2; exit 1 ;;
  esac
  if [ "$has_apple_id_credentials" = false ] && [ "$has_api_key_credentials" = false ]; then
    echo "Developer ID 正式发布还需要 Apple ID 或 App Store Connect API 公证凭据。" >&2
    exit 1
  fi
  release_build=true
  echo "将构建 Developer ID 签名、公证并 staple 的 macOS 发布包。"
else
  echo "未设置 APPLE_SIGNING_IDENTITY；将构建仅供本机测试的 ad-hoc 签名包。"
  # Rust's linker adds an executable-only ad-hoc signature on Apple Silicon,
  # but that does not seal Info.plist or bundle resources. Tell Tauri to sign
  # the completed .app explicitly so the app copied into the DMG is valid too.
  export APPLE_SIGNING_IDENTITY="-"
fi

build_marker="$(mktemp "${TMPDIR:-/tmp}/guangya-package-macos.XXXXXX")"
cleanup() {
  rm -f "$build_marker"
}
trap cleanup EXIT

# Run the two Tauri hooks explicitly so a hook failure cannot fall through and
# accidentally validate a stale bundle left by an earlier build. The marker is
# created first so only artifacts produced by this invocation are accepted.
pnpm prepare:rclone
pnpm ui:build
pnpm tauri build --target "$target" --bundles app,dmg --ci

bundle_root="target/${target}/release/bundle"
fresh_apps=()
for candidate in "$bundle_root"/macos/*.app; do
  if [ -d "$candidate" ] && [ "$candidate/Contents/Info.plist" -nt "$build_marker" ]; then
    fresh_apps+=("$candidate")
  fi
done
fresh_dmgs=()
for candidate in "$bundle_root"/dmg/*.dmg; do
  if [ -f "$candidate" ] && [ "$candidate" -nt "$build_marker" ]; then
    fresh_dmgs+=("$candidate")
  fi
done
if [ "${#fresh_apps[@]}" -ne 1 ] || [ "${#fresh_dmgs[@]}" -ne 1 ]; then
  echo "本次构建应生成且只生成一个新 .app 和一个新 .dmg；实际找到 ${#fresh_apps[@]} 个 App、${#fresh_dmgs[@]} 个 DMG。" >&2
  exit 1
fi
app_path="${fresh_apps[0]}"
dmg_path="${fresh_dmgs[0]}"
app_binary="$(find "$app_path/Contents/MacOS" -maxdepth 1 -type f -print -quit)"
embedded_rclone="$(find "$app_path/Contents/Resources" -type f -name rclone -print -quit)"
embedded_license="$(find "$app_path/Contents/Resources" -type f -name rclone-COPYING.txt -print -quit)"
if [ -z "$app_binary" ]; then
  echo "新 App 缺少主程序。" >&2
  exit 1
fi
if [ -z "$embedded_rclone" ] || [ -z "$embedded_license" ]; then
  echo "新 App 缺少经过校验的 rclone 或其 COPYING 许可证。" >&2
  exit 1
fi
"$embedded_rclone" version >/dev/null

codesign --verify --deep --strict --verbose=2 "$app_path"
hdiutil verify "$dmg_path"

if [ "$release_build" = true ]; then
  xcrun stapler staple "$app_path"
  xcrun stapler staple "$dmg_path"
  xcrun stapler validate "$app_path"
  xcrun stapler validate "$dmg_path"
  spctl --assess --type execute --verbose=2 "$app_path"
fi

echo "App: $app_path"
echo "DMG: $dmg_path"
