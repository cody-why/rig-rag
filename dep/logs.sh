#!/bin/bash
# 查看 systemd journal 日志
# 用法:
#   ./dep/logs.sh          # 实时跟踪
#   ./dep/logs.sh -n 200   # 最近 200 行后跟踪
#   ./dep/logs.sh --no-follow
set -euo pipefail

# shellcheck source=_common.sh
source "$(cd "$(dirname "$0")" && pwd)/_common.sh"
FOLLOW=1
LINES=100

while [[ $# -gt 0 ]]; do
  case "$1" in
    -n|--lines)
      LINES="${2:-100}"
      shift 2
      ;;
    --no-follow|-N)
      FOLLOW=0
      shift
      ;;
    -h|--help)
      echo "用法: $0 [-n N] [--no-follow]"
      exit 0
      ;;
    *)
      echo "未知参数: $1"
      exit 1
      ;;
  esac
done

require_service

if [[ "$FOLLOW" -eq 1 ]]; then
  exec journalctl -u "${SERVICE_NAME}" -n "$LINES" -f --no-pager
else
  exec journalctl -u "${SERVICE_NAME}" -n "$LINES" --no-pager
fi
