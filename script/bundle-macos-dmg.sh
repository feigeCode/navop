#!/bin/bash
set -euo pipefail

APP_NAME="Navop"
TARGET="${1:-aarch64-apple-darwin}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${ONETCLI_PROJECT_DIR:-$(cd "$SCRIPT_DIR/.." && pwd)}"
APP_DIR="${PROJECT_DIR}/target/${APP_NAME}.app"
TMP_DIR="${PROJECT_DIR}/target/dmg"
DMG_NAME="navop-${TARGET}.dmg"
DMG_PATH="${PROJECT_DIR}/${DMG_NAME}"
DMG_RETRIES="${ONETCLI_DMG_RETRIES:-3}"
DMG_RETRY_DELAY="${ONETCLI_DMG_RETRY_DELAY:-5}"

if [ ! -d "$APP_DIR" ]; then
    echo "Error: App bundle not found at ${APP_DIR}"
    echo "Run: script/bundle-macos.sh ${TARGET}"
    exit 1
fi

rm -rf "$TMP_DIR"
mkdir -p "$TMP_DIR"
cp -R "$APP_DIR" "$TMP_DIR/${APP_NAME}.app"
ln -s /Applications "$TMP_DIR/Applications"

# 生成可分发的压缩 DMG（UDZO）
create_dmg() {
    local attempt
    local tmp_dmg
    local content_mb
    local dmg_size_mb

    # 显式给出镜像大小：hdiutil 自动估算偏小时会以
    # "hdiutil: create failed - No space left on device" 失败（即使磁盘还有空位）。
    # 首次 = 内容 + 512MB 余量，重试按倍数放大。
    content_mb="$(du -sm "$TMP_DIR" | awk '{print $1}')"

    for attempt in $(seq 1 "$DMG_RETRIES"); do
        tmp_dmg="${PROJECT_DIR}/target/${DMG_NAME}.tmp.${attempt}.$$.dmg"
        dmg_size_mb=$(( (content_mb + ${ONETCLI_DMG_SLACK_MB:-512}) * attempt ))
        rm -f "$tmp_dmg" "$DMG_PATH"
        echo "hdiutil create: content=${content_mb}MB, image=${dmg_size_mb}MB (attempt ${attempt}/${DMG_RETRIES})"

        if hdiutil create \
            -volname "${APP_NAME}" \
            -srcfolder "$TMP_DIR" \
            -ov \
            -size "${dmg_size_mb}m" \
            -format UDZO \
            "$tmp_dmg"; then
            mv -f "$tmp_dmg" "$DMG_PATH"
            return 0
        fi

        rm -f "$tmp_dmg" "$DMG_PATH"
        if [ "$attempt" -lt "$DMG_RETRIES" ]; then
            echo "hdiutil create failed; retrying in ${DMG_RETRY_DELAY}s (${attempt}/${DMG_RETRIES})..."
            sleep "$DMG_RETRY_DELAY"
        fi
    done

    echo "Error: failed to create DMG after ${DMG_RETRIES} attempt(s)"
    return 1
}

create_dmg

# 释放 staging 拷贝（约等于 .app 大小），后续 tar/上传步骤会用到空间
rm -rf "$TMP_DIR"

# 可选：如果提供签名身份，则对 DMG 执行签名
if [ -n "${MACOS_SIGN_IDENTITY:-}" ]; then
    echo "Signing DMG with identity: ${MACOS_SIGN_IDENTITY}"
    codesign --force --sign "${MACOS_SIGN_IDENTITY}" "$DMG_PATH"
fi

echo "Successfully built DMG: ${DMG_PATH}"
ls -lh "$DMG_PATH"
