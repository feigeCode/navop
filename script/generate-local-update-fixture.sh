#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VERSION="${1:-99.0.0}"
SIZE_MB="${2:-8}"
OUTPUT_DIR="${REPO_ROOT}/target/local-update-simulation"
PAYLOAD_DIR="${OUTPUT_DIR}/payload"
PACKAGE_PATH="${OUTPUT_DIR}/navop-local-update.tar.gz"
MANIFEST_PATH="${OUTPUT_DIR}/latest.json"

if ! [[ "${SIZE_MB}" =~ ^[1-9][0-9]*$ ]]; then
  echo "错误：包大小必须是大于 0 的整数（MB）。" >&2
  exit 1
fi

if [[ "${VERSION}" == *$'\n'* || "${VERSION}" == *$'\r'* ]]; then
  echo "错误：版本号不能包含换行符。" >&2
  exit 1
fi

rm -rf "${OUTPUT_DIR}"
mkdir -p "${PAYLOAD_DIR}"

cat >"${PAYLOAD_DIR}/README.txt" <<EOF
Navop local update simulation fixture
Version: ${VERSION}

This archive is intentionally not installable. It only exercises the local
manifest, download progress, cancellation, and SHA256 verification flows.
EOF

dd if=/dev/urandom of="${PAYLOAD_DIR}/payload.bin" bs=1048576 count="${SIZE_MB}" 2>/dev/null
tar -czf "${PACKAGE_PATH}" -C "${PAYLOAD_DIR}" .

if command -v shasum >/dev/null 2>&1; then
  SHA256="$(shasum -a 256 "${PACKAGE_PATH}" | awk '{print $1}')"
elif command -v sha256sum >/dev/null 2>&1; then
  SHA256="$(sha256sum "${PACKAGE_PATH}" | awk '{print $1}')"
else
  echo "错误：未找到 shasum 或 sha256sum，无法生成 SHA256。" >&2
  exit 1
fi

escaped_package_path="${PACKAGE_PATH//\\/\\\\}"
escaped_package_path="${escaped_package_path//\"/\\\"}"
escaped_version="${VERSION//\\/\\\\}"
escaped_version="${escaped_version//\"/\\\"}"
RELEASE_NOTES="$(cat <<EOF
## Navop ${VERSION} 本地更新模拟

### 新功能

- 全新的更新提示窗口，重新调整标题、图标、版本信息和操作按钮布局。
- 更新说明支持 Markdown 标题、列表、引用和行内代码。
- 本地更新源支持绝对路径、相对路径以及 \`file://\` URL。
- 本地更新包会显示实时读取进度，并在完成后验证 SHA256。
- 自动检查更新和跳过版本设置现在可以持久化保存。

### 体验改进

- 优化长篇更新说明的滚动区域，窗口尺寸不会随内容无限增长。
- 下载阶段使用独立的紧凑窗口，突出显示进度和当前状态。
- 取消更新时会立即反馈取消状态，并清理未完成的临时文件。
- 下载失败、完整性校验失败和用户取消现在具有不同的提示信息。
- 简体中文、繁体中文和英文界面使用一致的文案结构。

### 问题修复

- 修复自定义更新接口缺少发布说明时显示空白的问题。
- 修复多个备用下载地址重复尝试同一个地址的问题。
- 修复取消下载后可能残留 \`.part\` 文件的问题。
- 修复本地更新源被网络连通性检查阻塞的问题。
- 修复启用 GitHub 更新功能时无法优先使用本地 manifest 的问题。

### 本地模拟验证

1. 读取本地 \`latest.json\` 更新清单。
2. 展示版本号和这份较长的发布说明。
3. 从本地复制模拟更新包并更新进度。
4. 对复制完成的文件执行 SHA256 完整性校验。
5. 完成后仅关闭模拟窗口，不启动安装器。

> 安全提示：该模拟不会替换、覆盖或修改当前 Navop 应用。

感谢参与 Navop 更新流程测试。
EOF
)"
escaped_release_notes="${RELEASE_NOTES//\\/\\\\}"
escaped_release_notes="${escaped_release_notes//\"/\\\"}"
escaped_release_notes="${escaped_release_notes//$'\r'/\\r}"
escaped_release_notes="${escaped_release_notes//$'\n'/\\n}"

cat >"${MANIFEST_PATH}" <<EOF
{
  "version": "${escaped_version}",
  "release_notes": "${escaped_release_notes}",
  "download_url": "${escaped_package_path}",
  "sha256": "${SHA256}"
}
EOF

echo "本地更新模拟文件已生成："
echo "  Manifest: ${MANIFEST_PATH}"
echo "  Package:  ${PACKAGE_PATH}"
echo "  SHA256:   ${SHA256}"
echo
echo "启动 Navop："
printf '  NAVOP_UPDATE_URL=%q cargo run -p main\n' "${MANIFEST_PATH}"
echo
echo "然后打开：设置 → 更新 → 检查更新"
