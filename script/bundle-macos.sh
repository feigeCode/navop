#!/bin/bash
set -euo pipefail

APP_NAME="Navop"
BINARY_NAME="navop"
TARGET="${1:-aarch64-apple-darwin}"
VERSION="${ONETCLI_VERSION:-0.1.0}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SOURCE_PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
PROJECT_DIR="${ONETCLI_PROJECT_DIR:-$SOURCE_PROJECT_DIR}"
RESOURCE_DIR="${ONETCLI_MACOS_RESOURCE_DIR:-${PROJECT_DIR}/resources/macos}"
APP_DIR="${PROJECT_DIR}/target/${APP_NAME}.app"

echo "Bundling ${APP_NAME}.app for ${TARGET} (version: ${VERSION})..."

# Clean previous bundle
rm -rf "$APP_DIR"

# Create .app directory structure
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy binary
BINARY_PATH="${ONETCLI_BINARY_PATH:-${PROJECT_DIR}/target/${TARGET}/release/${BINARY_NAME}}"
if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at ${BINARY_PATH}"
    echo "Run: cargo build --release -p main --target ${TARGET}"
    exit 1
fi
cp "$BINARY_PATH" "$APP_DIR/Contents/MacOS/${BINARY_NAME}"

# Copy Info.plist and substitute version
sed "s/\${ONETCLI_VERSION}/${VERSION}/g" \
    "${RESOURCE_DIR}/Info.plist" \
    > "$APP_DIR/Contents/Info.plist"

# Copy icon
ICNS_PATH="${RESOURCE_DIR}/Navop.icns"
if [ ! -f "$ICNS_PATH" ]; then
    echo "Error: Icon file not found at ${ICNS_PATH}"
    exit 1
fi
cp "$ICNS_PATH" "$APP_DIR/Contents/Resources/Navop.icns"

# Write PkgInfo
echo -n "APPL????" > "$APP_DIR/Contents/PkgInfo"

# Sign the app bundle before packaging it. A real Developer ID identity is
# preferred; ad-hoc signing still binds Info.plist and the bundle identifier.
APP_SIGN_IDENTITY="${MACOS_APP_SIGN_IDENTITY:-${MACOS_SIGN_IDENTITY:--}}"
if [ "$APP_SIGN_IDENTITY" = "-" ]; then
    echo "Ad-hoc signing ${APP_NAME}.app..."
    codesign --force --deep --sign - "$APP_DIR"
else
    echo "Signing ${APP_NAME}.app with identity: ${APP_SIGN_IDENTITY}"
    codesign \
        --force \
        --deep \
        --options runtime \
        --timestamp \
        --sign "$APP_SIGN_IDENTITY" \
        "$APP_DIR"
fi
codesign --verify --deep --strict --verbose=2 "$APP_DIR"

echo "Successfully built: ${APP_DIR}"
echo "Contents:"
ls -la "$APP_DIR/Contents/"
ls -la "$APP_DIR/Contents/MacOS/"
ls -la "$APP_DIR/Contents/Resources/"
