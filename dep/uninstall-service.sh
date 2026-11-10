#!/bin/bash
# 卸载 systemd 服务（不删除部署目录与数据）
# 用法: sudo ./dep/uninstall-service.sh
set -euo pipefail

# shellcheck source=_common.sh
source "$(cd "$(dirname "$0")" && pwd)/_common.sh"

if [[ $EUID -ne 0 ]]; then
  echo "请使用 root 或 sudo 执行: sudo $0"
  exit 1
fi

systemctl stop "${SERVICE_UNIT}" 2>/dev/null || true
systemctl disable "${SERVICE_UNIT}" 2>/dev/null || true

if [[ -f "$SERVICE_FILE" ]]; then
  rm -f "$SERVICE_FILE"
  systemctl daemon-reload
  systemctl reset-failed "${SERVICE_UNIT}" 2>/dev/null || true
  echo "已卸载 systemd 服务: ${SERVICE_NAME}"
else
  echo "未找到服务文件: $SERVICE_FILE"
fi
