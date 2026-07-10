#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET="retry-test-target"
FAKE_BIN_DIR="$(mktemp -d "${TMPDIR:-/tmp}/onetcli-dmg-test.XXXXXX")"
TEST_PROJECT_DIR="${FAKE_BIN_DIR}/project"
APP_DIR="${TEST_PROJECT_DIR}/target/Navop.app"
DMG_PATH="${TEST_PROJECT_DIR}/navop-${TARGET}.dmg"
ATTEMPTS_FILE="${FAKE_BIN_DIR}/attempts"

cleanup() {
    rm -rf "$FAKE_BIN_DIR"
}
trap cleanup EXIT

mkdir -p "$APP_DIR/Contents/MacOS"
cat > "${FAKE_BIN_DIR}/hdiutil" <<'HDIUTIL'
#!/usr/bin/env bash
set -euo pipefail

attempts_file="${ONETCLI_TEST_HDIUTIL_ATTEMPTS_FILE:?}"
attempts=0
if [ -f "$attempts_file" ]; then
    attempts="$(cat "$attempts_file")"
fi
attempts=$((attempts + 1))
printf '%s' "$attempts" > "$attempts_file"

if [ "$attempts" -eq 1 ]; then
    echo "hdiutil: create failed - Resource busy" >&2
    exit 1
fi

output="${@: -1}"
if [[ "$output" != *.dmg ]]; then
    output="${output}.dmg"
fi
printf 'fake dmg' > "$output"
HDIUTIL
chmod +x "${FAKE_BIN_DIR}/hdiutil"

PATH="${FAKE_BIN_DIR}:$PATH" \
ONETCLI_PROJECT_DIR="$TEST_PROJECT_DIR" \
ONETCLI_TEST_HDIUTIL_ATTEMPTS_FILE="$ATTEMPTS_FILE" \
ONETCLI_DMG_RETRIES=2 \
ONETCLI_DMG_RETRY_DELAY=0 \
    "$PROJECT_DIR/script/bundle-macos-dmg.sh" "$TARGET"

if [ "$(cat "$ATTEMPTS_FILE")" != "2" ]; then
    echo "Expected hdiutil to be retried exactly once"
    exit 1
fi

if [ ! -f "$DMG_PATH" ]; then
    echo "Expected DMG at $DMG_PATH"
    exit 1
fi
