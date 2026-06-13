#!/bin/bash
set -euo pipefail

if [ "$#" -ne 2 ]; then
    echo "Usage: $0 <target-triple> <destination-ipc-drivers-dir>" >&2
    exit 2
fi

TARGET="$1"
DEST_ROOT="$2"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

DRIVER_ID="duckdb"
DRIVER_BINARY_NAME="duckdb_driver"
if [[ "$TARGET" == *windows* ]]; then
    DRIVER_BINARY_NAME="duckdb_driver.exe"
fi

DRIVER_BINARY_PATH="${PROJECT_DIR}/target/${TARGET}/release/${DRIVER_BINARY_NAME}"
DRIVER_SOURCE_DIR="${PROJECT_DIR}/crates/duckdb_driver"
DRIVER_DEST_DIR="${DEST_ROOT}/${DRIVER_ID}"

if [ ! -f "$DRIVER_BINARY_PATH" ]; then
    echo "Error: DuckDB IPC driver binary not found at ${DRIVER_BINARY_PATH}" >&2
    echo "Run: cargo build --release -p duckdb_driver --target ${TARGET}" >&2
    exit 1
fi

mkdir -p "$DRIVER_DEST_DIR"
cp "$DRIVER_BINARY_PATH" "$DRIVER_DEST_DIR/${DRIVER_BINARY_NAME}"
cp "${DRIVER_SOURCE_DIR}/driver.json" "$DRIVER_DEST_DIR/driver.json"

if [ -d "${DRIVER_SOURCE_DIR}/locales" ]; then
    rm -rf "$DRIVER_DEST_DIR/locales"
    cp -R "${DRIVER_SOURCE_DIR}/locales" "$DRIVER_DEST_DIR/locales"
fi

if [[ "$TARGET" != *windows* ]]; then
    chmod +x "$DRIVER_DEST_DIR/${DRIVER_BINARY_NAME}"
fi

echo "Packaged IPC driver '${DRIVER_ID}' into ${DRIVER_DEST_DIR}"
