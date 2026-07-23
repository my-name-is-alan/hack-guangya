#!/usr/bin/env bash
set -euo pipefail

if [ "$(id -u)" -ne 0 ]; then
  echo "请使用 sudo ./uninstall.sh 卸载。" >&2
  exit 1
fi

systemctl disable --now guangya-sync.service 2>/dev/null || true
rm -f /etc/systemd/system/guangya-sync.service /usr/local/bin/guangya-sync
systemctl daemon-reload
rm -rf /opt/guangya-sync

if [ "${1:-}" = "--purge-data" ]; then
  rm -rf /var/lib/guangya-sync
  rm -f /etc/guangya-sync.env
  userdel guangya-sync 2>/dev/null || true
  groupdel guangya-sync 2>/dev/null || true
  echo "服务、配置和数据已删除。"
else
  echo "服务已卸载，/var/lib/guangya-sync 和 /etc/guangya-sync.env 已保留。"
fi
