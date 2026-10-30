#!/usr/bin/env bash
# 产品依赖隔离守卫：独立产品不得把对方的领域 crate 拖进依赖树。
#
# 用法：script/check-product-deps.sh
# 退出码非 0 表示隔离被破坏（CI 会失败）。
set -euo pipefail

SHELL_FORBIDDEN='db_view v|redis_view v|mongodb_view v| db v0'
DATA_FORBIDDEN='terminal_view v|sftp_view v|remote_desktop_view v|known_hosts_view v|port_forwarding_view v'

fail=0

check() {
  local product="$1" pattern="$2" hits
  hits="$(cargo tree -p "$product" -e normal 2>/dev/null | grep -E "$pattern" || true)"
  if [[ -n "$hits" ]]; then
    echo "✗ $product 的依赖树里出现了对方领域的 crate：" >&2
    echo "$hits" >&2
    fail=1
  else
    echo "✓ $product 依赖树只包含本领域 crate"
  fi
}

check navop-shell "$SHELL_FORBIDDEN"
check navop-data "$DATA_FORBIDDEN"

exit "$fail"
