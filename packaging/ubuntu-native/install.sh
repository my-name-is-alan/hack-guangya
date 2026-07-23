#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "请使用 sudo ./install.sh 安装。" >&2
  exit 1
fi

case "$(uname -m)" in
  x86_64|amd64) ;;
  *) echo "当前安装包只支持 Ubuntu x86_64。" >&2; exit 1 ;;
esac

if ! command -v systemctl >/dev/null 2>&1; then
  echo "未检测到 systemd，无法安装系统服务。" >&2
  exit 1
fi

SOURCE_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR=/opt/guangya-sync
STATE_DIR=/var/lib/guangya-sync
SERVICE_USER=guangya-sync
SERVICE_GROUP=guangya-sync

if ! getent group "$SERVICE_GROUP" >/dev/null; then
  groupadd --system "$SERVICE_GROUP"
fi
if ! id "$SERVICE_USER" >/dev/null 2>&1; then
  useradd --system --gid "$SERVICE_GROUP" --home-dir "$STATE_DIR" --shell /usr/sbin/nologin "$SERVICE_USER"
fi

install -d -m 0755 "$INSTALL_DIR"
cp -a "$SOURCE_DIR/app/." "$INSTALL_DIR/"
chown -R root:root "$INSTALL_DIR"
chmod 0755 "$INSTALL_DIR/node/bin/node"

install -d -o "$SERVICE_USER" -g "$SERVICE_GROUP" -m 0750 \
  "$STATE_DIR" "$STATE_DIR/data" "$STATE_DIR/watch" "$STATE_DIR/archive"

if [ ! -f /etc/guangya-sync.env ]; then
  install -o root -g root -m 0600 "$SOURCE_DIR/guangya-sync.env" /etc/guangya-sync.env
else
  grep -q '^GUANGYA_FILE_ROOTS=' /etc/guangya-sync.env || printf '\nGUANGYA_FILE_ROOTS=/\n' >> /etc/guangya-sync.env
  grep -q '^GUANGYA_OSS_TIMEOUT_MS=' /etc/guangya-sync.env || printf 'GUANGYA_OSS_TIMEOUT_MS=600000\n' >> /etc/guangya-sync.env
  grep -q '^GUANGYA_OSS_RETRY_MAX=' /etc/guangya-sync.env || printf 'GUANGYA_OSS_RETRY_MAX=3\n' >> /etc/guangya-sync.env
  grep -q '^GUANGYA_OSS_PARALLEL=' /etc/guangya-sync.env || printf 'GUANGYA_OSS_PARALLEL=3\n' >> /etc/guangya-sync.env
  grep -q '^GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS=' /etc/guangya-sync.env || printf 'GUANGYA_CLOUD_CONFIRM_TIMEOUT_MS=600000\n' >> /etc/guangya-sync.env
  grep -q '^GUANGYA_CLOUD_CONFIRM_POLL_MS=' /etc/guangya-sync.env || printf 'GUANGYA_CLOUD_CONFIRM_POLL_MS=1000\n' >> /etc/guangya-sync.env
fi
install -o root -g root -m 0644 "$SOURCE_DIR/guangya-sync.service" /etc/systemd/system/guangya-sync.service
install -o root -g root -m 0755 "$SOURCE_DIR/guangya-syncctl" /usr/local/bin/guangya-sync

systemctl daemon-reload
systemctl enable --now guangya-sync.service
sleep 1

if ! systemctl is-active --quiet guangya-sync.service; then
  journalctl -u guangya-sync.service -n 60 --no-pager >&2 || true
  exit 1
fi

echo "安装完成。"
echo "访问地址：http://服务器IP:8080"
echo "状态命令：guangya-sync status"
echo "日志命令：guangya-sync logs"
echo "配置文件：/etc/guangya-sync.env"
