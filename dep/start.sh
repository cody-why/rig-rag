#!/bin/bash
# 通过 systemd 启动 / 重启
# 用法:
#   ./dep/start.sh           # 启动
#   ./dep/start.sh restart   # 重启
set -euo pipefail

# shellcheck source=_common.sh
source "$(cd "$(dirname "$0")" && pwd)/_common.sh"
ACTION="${1:-start}"

case "$ACTION" in
  start|restart) ;;
  -h|--help)
    echo "用法: $0 [start|restart]"
    exit 0
    ;;
  *)
    echo "未知参数: $ACTION（支持 start|restart）"
    exit 1
    ;;
esac

require_service
run_systemctl "$ACTION" "${SERVICE_UNIT}"
systemctl --no-pager --full status "${SERVICE_UNIT}" || true

if [[ "$ACTION" == "restart" ]]; then
  echo "已重启 ${SERVICE_NAME}。日志: ${DEP_DIR}/logs.sh"
else
  echo "已启动 ${SERVICE_NAME}。日志: ${DEP_DIR}/logs.sh"
fi
