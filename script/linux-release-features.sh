#!/usr/bin/env bash
#
# 打印 Linux 发布包使用的 Cargo feature 列表（逗号分隔，单行）。
#
# 这是 Linux 发布形态的唯一真源：release 工作流用它构建，CI 用它编译同一形态，
# 免得两边各写一份列表后悄悄漂移。
#
# `embedded-webview` 默认不在列表里：HTML 预览的宿主是 WebKitGTK 4.1
# （`libwebkit2gtk-4.1.so.0`），而这个库无法随依赖包提供 —— 带 4.1 的发行版都
# 构建在 glibc 2.39+ 上、与发布包的 2.28 基线不相配，且 `WebKitWebProcess` 是
# 按编译期路径解析的独立 ELF，外挂不了。发行版真的提供了它时仍然应该用上，所以
# 设 NAVOP_LINUX_EMBEDDED_WEBVIEW=1 可以把它加回来 —— 但必须先由 pkg-config
# 确认构建主机链接得到它。其余默认 feature 保持不变。
set -euo pipefail

features="wasm-components,windows-native-rdp,shell-plugins"

if [ "${NAVOP_LINUX_EMBEDDED_WEBVIEW:-0}" = "1" ]; then
  if ! pkg-config --exists webkit2gtk-4.1; then
    echo "::error::NAVOP_LINUX_EMBEDDED_WEBVIEW=1 but webkit2gtk-4.1 is not visible to pkg-config" >&2
    exit 1
  fi
  features="${features},embedded-webview"
fi

printf '%s\n' "${features}"
