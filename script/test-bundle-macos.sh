#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET="bundle-test-target"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/navop-app-bundle-test.XXXXXX")"
TEST_PROJECT_DIR="${TEST_ROOT}/project"
APP_DIR="${TEST_PROJECT_DIR}/target/Navop.app"
FAKE_BINARY="${TEST_ROOT}/navop"
CODESIGN_LOG="${TEST_ROOT}/codesign.log"

cleanup() {
    rm -rf "$TEST_ROOT"
}
trap cleanup EXIT

mkdir -p "$TEST_ROOT/bin"
printf '#!/usr/bin/env bash\nexit 0\n' > "$FAKE_BINARY"
chmod +x "$FAKE_BINARY"

cat > "$TEST_ROOT/bin/codesign" <<'CODESIGN'
#!/usr/bin/env bash
set -euo pipefail
printf '%s\n' "$*" >> "${ONETCLI_TEST_CODESIGN_LOG:?}"
CODESIGN
chmod +x "$TEST_ROOT/bin/codesign"

PATH="${TEST_ROOT}/bin:$PATH" \
ONETCLI_PROJECT_DIR="$TEST_PROJECT_DIR" \
ONETCLI_MACOS_RESOURCE_DIR="${PROJECT_DIR}/resources/macos" \
ONETCLI_BINARY_PATH="$FAKE_BINARY" \
ONETCLI_TEST_CODESIGN_LOG="$CODESIGN_LOG" \
ONETCLI_VERSION="9.8.7" \
    "$PROJECT_DIR/script/bundle-macos.sh" "$TARGET"

grep -q '<key>NSLocalNetworkUsageDescription</key>' "$APP_DIR/Contents/Info.plist"
grep -q '<string>9.8.7</string>' "$APP_DIR/Contents/Info.plist"
grep -q -- "--sign - $APP_DIR" "$CODESIGN_LOG"
grep -q -- "--verify --deep --strict --verbose=2 $APP_DIR" "$CODESIGN_LOG"
