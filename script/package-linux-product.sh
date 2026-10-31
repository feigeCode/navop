#!/usr/bin/env bash
set -euo pipefail

PRODUCT="${1:?usage: package-linux-product.sh <navop-shell|navop-data> [target]}"
TARGET="${2:-x86_64-unknown-linux-gnu}"
VERSION="${NAVOP_VERSION:-0.0.0}"
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

ARCHIVE="${PROJECT_DIR}/target/${BINARY_NAME}-${VERSION}-${TARGET}.tar.gz"
tar -czf "$ARCHIVE" -C "$STAGE_DIR" .

PORTABLE_DIR="${PROJECT_DIR}/target/${PRODUCT}-portable"
rm -rf "$PORTABLE_DIR"
mkdir -p "$PORTABLE_DIR"
cp "$BINARY_PATH" "$PORTABLE_DIR/${BINARY_NAME}"
printf '%s\n' "$APP_NAME" > "$PORTABLE_DIR/${PORTABLE_MARKER}"
PORTABLE_ARCHIVE="${PROJECT_DIR}/target/${BINARY_NAME}-${VERSION}-${TARGET}-portable.tar.gz"
tar -czf "$PORTABLE_ARCHIVE" -C "$PORTABLE_DIR" .

echo "Successfully built: ${ARCHIVE}"
echo "Successfully built: ${PORTABLE_ARCHIVE}"
