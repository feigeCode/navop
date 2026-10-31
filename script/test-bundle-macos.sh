#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
TARGET="bundle-test-target"
TEST_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/navop-app-bundle-test.XXXXXX")"
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

bundle_product() {
    local product="$1"
    local app_name="$2"
    local binary_name="$3"
    local bundle_id="$4"
    local include_docs="$5"
    local project_dir="${TEST_ROOT}/project-${product}"
    local app_dir="${project_dir}/target/${app_name}.app"

    PATH="${TEST_ROOT}/bin:$PATH" \
    ONETCLI_PRODUCT="$product" \
    ONETCLI_PROJECT_DIR="$project_dir" \
    ONETCLI_MACOS_RESOURCE_DIR="${PROJECT_DIR}/resources/macos" \
    ONETCLI_BINARY_PATH="$FAKE_BINARY" \
    ONETCLI_TEST_CODESIGN_LOG="$CODESIGN_LOG" \
    ONETCLI_VERSION="9.8.7" \
        "$PROJECT_DIR/script/bundle-macos.sh" "$TARGET" >/dev/null

    test -x "${app_dir}/Contents/MacOS/${binary_name}"
    grep -q '<key>NSLocalNetworkUsageDescription</key>' "${app_dir}/Contents/Info.plist"
    grep -q '<string>9.8.7</string>' "${app_dir}/Contents/Info.plist"
    grep -q "<string>${bundle_id}</string>" "${app_dir}/Contents/Info.plist"
    grep -A1 '<key>CFBundleExecutable</key>' "${app_dir}/Contents/Info.plist" \
        | grep -q "<string>${binary_name}</string>"
    grep -A1 '<key>CFBundleName</key>' "${app_dir}/Contents/Info.plist" \
        | grep -q "<string>${app_name}</string>"

    if [ "$include_docs" = "1" ]; then
        grep -q '<key>CFBundleDocumentTypes</key>' "${app_dir}/Contents/Info.plist"
        grep -q '<string>duckdb</string>' "${app_dir}/Contents/Info.plist"
        grep -q '<string>md</string>' "${app_dir}/Contents/Info.plist"
        grep -q '<string>cast</string>' "${app_dir}/Contents/Info.plist"
        grep -q '<string>Viewer</string>' "${app_dir}/Contents/Info.plist"
        grep -q '<string>org.asciinema.cast</string>' "${app_dir}/Contents/Info.plist"
        grep -q '<string>application/x-asciicast</string>' "${app_dir}/Contents/Info.plist"
    else
        ! grep -q '<key>CFBundleDocumentTypes</key>' "${app_dir}/Contents/Info.plist"
        ! grep -q '<key>UTImportedTypeDeclarations</key>' "${app_dir}/Contents/Info.plist"
    fi
}

bundle_product navop "Navop" "navop" "com.onetcli.app" 1
bundle_product navop-shell "Navop Shell" "navop-shell" "com.navop.shell" 0
bundle_product navop-data "Navop Data" "navop-data" "com.navop.data" 0

grep -q -- "--sign - .*Navop.app" "$CODESIGN_LOG"
grep -q -- "--verify --deep --strict --verbose=2 .*Navop.app" "$CODESIGN_LOG"
