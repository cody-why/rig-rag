#!/bin/bash
# 通过 systemd 停止服务
set -euo pipefail

# shellcheck source=_common.sh
source "$(cd "$(dirname "$0")" && pwd)/_common.sh"

require_service
run_systemctl stop "${SERVICE_UNIT}"
echo "已停止 ${SERVICE_NAME}"
