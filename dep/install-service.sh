#!/bin/bash
# CentOS / RHEL systemd 自启动安装
# 用法: sudo ./dep/install-service.sh
set -euo pipefail

# shellcheck source=_common.sh
source "$(cd "$(dirname "$0")" && pwd)/_common.sh"
TEMPLATE="${DEP_DIR}/app.service"

if [[ $EUID -ne 0 ]]; then
  echo "请使用 root 或 sudo 执行: sudo $0"
  exit 1
fi

if [[ ! -x "${BIN_PATH}" ]]; then
  echo "未找到可执行文件: ${BIN_PATH}"
  echo "请先将二进制放到部署目录并 chmod +x（可在 dep/config.sh 改 APP_BIN）"
  exit 1
fi

if [[ ! -f "$TEMPLATE" ]]; then
  echo "未找到单元模板: $TEMPLATE"
  exit 1
fi

if [[ ! -f "${APP_DIR}/.env" ]]; then
  echo "警告: ${APP_DIR}/.env 不存在，请按需准备环境配置"
fi

# shellcheck disable=SC2086
if [[ -n "${APP_DIRS}" ]]; then
  for d in ${APP_DIRS}; do
    mkdir -p "${APP_DIR}/${d}"
  done
fi

sed \
  -e "s|__APP_DIR__|${APP_DIR}|g" \
  -e "s|__APP_BIN__|${APP_BIN}|g" \
  -e "s|__APP_NAME__|${APP_NAME}|g" \
  -e "s|__APP_DESC__|${APP_DESC}|g" \
  "$TEMPLATE" > "$SERVICE_FILE"
chmod 644 "$SERVICE_FILE"

systemctl daemon-reload
systemctl enable "${SERVICE_UNIT}"
systemctl restart "${SERVICE_UNIT}"

echo "已安装并启动 systemd 服务: ${SERVICE_NAME}"
echo "  部署目录: ${APP_DIR}"
echo "  二进制:   ${BIN_PATH}"
echo "  状态: systemctl status ${SERVICE_NAME}"
echo "  日志: ${DEP_DIR}/logs.sh  或  journalctl -u ${SERVICE_NAME} -f"
echo "  停止: ${DEP_DIR}/stop.sh"
echo "  开机自启: 已 enable"
