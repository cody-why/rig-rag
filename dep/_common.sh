#!/bin/bash
# 内部公共变量，勿直接执行
DEP_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=config.sh
source "${DEP_DIR}/config.sh"
APP_DIR="$(cd "${DEP_DIR}/.." && pwd)"
SERVICE_NAME="${APP_NAME}"
SERVICE_UNIT="${SERVICE_NAME}.service"
SERVICE_FILE="/etc/systemd/system/${SERVICE_UNIT}"
BIN_PATH="${APP_DIR}/${APP_BIN}"

run_systemctl() {
  if [[ $EUID -eq 0 ]]; then
    systemctl "$@"
  else
    sudo systemctl "$@"
  fi
}

require_service() {
  if ! systemctl cat "${SERVICE_UNIT}" &>/dev/null; then
    echo "未安装 systemd 服务，请先执行: sudo ${DEP_DIR}/install-service.sh"
    exit 1
  fi
}
