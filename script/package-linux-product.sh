#!/usr/bin/env bash
set -euo pipefail

# 打包独立产品（navop-shell / navop-data）的 Linux 产物。
#
# 用法: package-linux-product.sh <navop-shell|navop-data> [target] \
#         [archive-out] [public-out] [portable-out]
#
# 输出名参数由 release.yml 传入，用于把产物对齐到公开命名
# {product}-{version}-{public_label}（如 navop-shell-0.22.0-linux-x64.tar.gz）；
# 缺省时退回 {binary}-{version}-{target} 本地命名，便于手动调用。
PRODUCT="${1:?usage: package-linux-product.sh <navop-shell|navop-data> [target] [archive-out] [public-out] [portable-out]}"
TARGET="${2:-x86_64-unknown-linux-gnu}"
ARCHIVE_OUT="${3:-}"
PUBLIC_OUT="${4:-}"
PORTABLE_OUT="${5:-}"
VERSION="${NAVOP_VERSION:-${ONETCLI_VERSION:-0.0.0}}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${ONETCLI_PROJECT_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"

case "$PRODUCT" in
    navop-shell)
        BINARY_NAME="navop-shell"
        APP_NAME="Navop Shell"
        PORTABLE_MARKER="navop-shell.portable"
        ;;
    navop-data)
        BINARY_NAME="navop-data"
        APP_NAME="Navop Data"
        PORTABLE_MARKER="navop-data.portable"
        ;;
    *)
        echo "Error: unknown product '${PRODUCT}' (expected navop-shell, navop-data)" >&2
        exit 1
        ;;
esac

BINARY_PATH="${ONETCLI_BINARY_PATH:-${PROJECT_DIR}/target/${TARGET}/release/${BINARY_NAME}}"
if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: binary not found at ${BINARY_PATH}" >&2
    echo "Run: cargo build --release -p ${PRODUCT} --target ${TARGET}" >&2
    exit 1
fi

DESKTOP_PATH="${PROJECT_DIR}/resources/linux/${PRODUCT}.desktop"
if [ ! -f "$DESKTOP_PATH" ]; then
    echo "Error: desktop entry not found at ${DESKTOP_PATH}" >&2
    exit 1
fi

STAGE_DIR="${PROJECT_DIR}/target/${PRODUCT}-linux-package"
rm -rf "$STAGE_DIR"
mkdir -p "$STAGE_DIR/usr/bin" \
    "$STAGE_DIR/usr/share/applications" \
    "$STAGE_DIR/usr/share/icons/hicolor/128x128/apps" \
    "$STAGE_DIR/usr/share/icons/hicolor/256x256/apps" \
    "$STAGE_DIR/usr/share/icons/hicolor/512x512/apps"

cp "$BINARY_PATH" "$STAGE_DIR/usr/bin/${BINARY_NAME}"
cp "$DESKTOP_PATH" "$STAGE_DIR/usr/share/applications/"
for size in 128 256 512; do
    cp "${PROJECT_DIR}/resources/linux/${PRODUCT}-${size}.png" \
        "$STAGE_DIR/usr/share/icons/hicolor/${size}x${size}/apps/${BINARY_NAME}.png"
done

# tar 参数与 release.yml 全量 Package (Linux) 步骤保持一致：固定
# mtime/owner 让同一 tag 的产物字节级可复现，sha256 才能对得上。
ARCHIVE="${ARCHIVE_OUT:-${PROJECT_DIR}/target/${BINARY_NAME}-${VERSION}-${TARGET}.tar.gz}"
tar \
    --sort=name \
    --mtime='UTC 1970-01-01' \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    -czf "$ARCHIVE" -C "$STAGE_DIR" .
if [ -n "$PUBLIC_OUT" ]; then
    cp "$ARCHIVE" "$PUBLIC_OUT"
fi

PORTABLE_DIR="${PROJECT_DIR}/target/${PRODUCT}-portable"
rm -rf "$PORTABLE_DIR"
mkdir -p "$PORTABLE_DIR"
cp "$BINARY_PATH" "$PORTABLE_DIR/${BINARY_NAME}"
printf '%s\n' "$APP_NAME" > "$PORTABLE_DIR/${PORTABLE_MARKER}"
PORTABLE_ARCHIVE="${PORTABLE_OUT:-${PROJECT_DIR}/target/${BINARY_NAME}-${VERSION}-${TARGET}-portable.tar.gz}"
tar \
    --sort=name \
    --mtime='UTC 1970-01-01' \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    -czf "$PORTABLE_ARCHIVE" -C "$PORTABLE_DIR" .

echo "Successfully built: ${ARCHIVE}"
[ -n "$PUBLIC_OUT" ] && echo "Successfully built: ${PUBLIC_OUT}"
echo "Successfully built: ${PORTABLE_ARCHIVE}"
