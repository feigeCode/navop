#!/usr/bin/env bash
set -Eeuo pipefail

# 用法：
#   script/beta-release.sh --base 0.23.0 --notes-file /private/tmp/navop-v0.23.0-beta.1-notes.md
#   script/beta-release.sh --next --notes-file /private/tmp/navop-v0.23.0-beta.2-notes.md
#   script/beta-release.sh --next --notes-file ... --dry-run
#
# 作用：在 dev 分支上一条命令发测试版。脚本会把 main/Cargo.toml 与三个
# app crate（apps/navop、apps/navop-shell、apps/navop-data）的
# [package].version 统一改成 X.Y.Z-beta.N（--base 从 beta.1 起步，--next
# 递增当前 beta 序号），同步 Cargo.lock 与 CHANGELOG.md，提交后创建并推送
# v* tag。统一版本：一个 tag 同发三个产品，版本必须一起 bump。
#
# tag 推送会触发 release-trigger.yml -> release.yml。预发布 tag 自动走 beta
# 通道：GitHub Release 打上 --prerelease，R2 更新清单写到
# updates/beta/latest.json，稳定清单 updates/latest.json 完全不受影响。
#
# 前置条件（缺一不可）：
#   1. 当前分支是 dev（beta 版本线只走 dev，main 只承载稳定发版）；
#   2. 工作区干净（--dry-run 只提示不中止）；
#   3. 本地与远端都不存在同名 tag；
#   4. --notes-file 指向包含双语段落的发布说明（release.yml 会硬校验
#      CHANGELOG.md 里的条目，缺了连构建都不会启动）。
#
# 版本线约定：beta 的 base 是「下一个稳定版」，例如 0.23.0-beta.N -> 0.23.0，
# 不新开 0.22.1-beta 这类补丁线。

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"
cd "${REPO_ROOT}"

BASE=""
NEXT="false"
NOTES_FILE=""
DRY_RUN="false"
REMOTE="${REMOTE:-cnb}"
MAIN_MANIFEST="${MAIN_MANIFEST:-${REPO_ROOT}/main/Cargo.toml}"
APP_MANIFESTS=(
  "${REPO_ROOT}/apps/navop/Cargo.toml"
  "${REPO_ROOT}/apps/navop-shell/Cargo.toml"
  "${REPO_ROOT}/apps/navop-data/Cargo.toml"
)
LOCKFILE="${LOCKFILE:-${REPO_ROOT}/Cargo.lock}"
CHANGELOG="${CHANGELOG:-CHANGELOG.md}"
COMMITTED="false"
TAGGED="false"
PUSHED="false"

usage() {
  cat <<'EOF'
用法：
  script/beta-release.sh --base <X.Y.Z> --notes-file <PATH> [--dry-run]
  script/beta-release.sh --next --notes-file <PATH> [--dry-run]

参数：
  --base <X.Y.Z>     把版本设为 <X.Y.Z>-beta.1（<X.Y.Z> 必须是下个稳定版）
  --next             把当前 <X.Y.Z>-beta.N 递增为 <X.Y.Z>-beta.(N+1)
  --notes-file <PATH> 发布说明文件（必填，changelog.py upsert 的输入）
  --dry-run          只打印将要执行的操作，不写任何文件、不提交、不打 tag
  -h, --help         显示本帮助

环境变量：
  REMOTE             推送远端，默认 cnb
  MAIN_MANIFEST      Cargo.toml 路径，默认 main/Cargo.toml
  LOCKFILE           Cargo.lock 路径，默认 Cargo.lock
  CHANGELOG          CHANGELOG.md 路径，默认 CHANGELOG.md

示例：
  script/beta-release.sh --base 0.23.0 \
    --notes-file /private/tmp/navop-v0.23.0-beta.1-notes.md
EOF
}

die() {
  echo "错误：$*" >&2
  exit 1
}

on_error() {
  local code=$?
  trap - ERR
  echo "" >&2
  echo "错误：beta 发布流程中止（exit ${code}），未完成的部分可按下面回滚：" >&2
  if [[ "${PUSHED}" == "true" ]]; then
    echo "  git push ${REMOTE} --delete ${TAG}   # 删除已推送的远端 tag" >&2
  fi
  if [[ "${TAGGED}" == "true" ]]; then
    echo "  git tag -d ${TAG}" >&2
  fi
  if [[ "${COMMITTED}" == "true" ]]; then
    echo "  git reset --hard HEAD~1                # 回退版本 + changelog 提交" >&2
    echo "  git push --force-with-lease ${REMOTE} dev   # 若 dev 已推送" >&2
  else
    echo "  git checkout -- ${MAIN_MANIFEST} ${LOCKFILE} ${CHANGELOG}" >&2
  fi
  exit "${code}"
}
trap on_error ERR

need_value() {
  if [[ $# -lt 2 || -z "${2:-}" ]]; then
    die "参数 $1 需要一个值。"
  fi
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      need_value "$@"
      BASE="$2"
      shift 2
      ;;
    --next)
      NEXT="true"
      shift
      ;;
    --notes-file)
      need_value "$@"
      NOTES_FILE="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN="true"
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "错误：未知参数 $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

# 1. 参数与发布说明校验（正式执行前全部做完，避免写了一半再失败）。
if [[ "${NEXT}" == "true" && -n "${BASE}" ]]; then
  die "--base 与 --next 只能二选一。"
fi
if [[ "${NEXT}" == "false" && -z "${BASE}" ]]; then
  die "必须指定 --base <X.Y.Z> 或 --next。例如：script/beta-release.sh --base 0.23.0 --notes-file <PATH>"
fi
if [[ -n "${BASE}" && ! "${BASE}" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  die "--base 必须是稳定版本号 X.Y.Z（例如 0.23.0）：beta 的 base 是下一个稳定版，不是 0.22.1 这样的补丁线。"
fi
if [[ -z "${NOTES_FILE}" ]]; then
  echo "错误：缺少 --notes-file。beta 发布必须提供双语发布说明，release.yml 会硬校验 CHANGELOG.md 条目。" >&2
  echo "示例：script/beta-release.sh --base 0.23.0 --notes-file /private/tmp/navop-v0.23.0-beta.1-notes.md" >&2
  exit 1
fi
if [[ ! -f "${NOTES_FILE}" || ! -s "${NOTES_FILE}" ]]; then
  die "发布说明文件不存在或为空：${NOTES_FILE}"
fi

read_manifest_version() {
  local manifest="${1:-${MAIN_MANIFEST}}"
  awk -F'"' '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && /^version = "/ { print $2; exit }
  ' "${manifest}"
}

read_lock_version() {
  local package_name="${1:-main}"
  awk -v want="${package_name}" '
    /^\[\[package\]\]$/ { in_package = 1; name = ""; next }
    in_package && /^name = "/ { split($0, parts, "\""); name = parts[2] }
    in_package && /^version = "/ {
      split($0, parts, "\"")
      if (name == want) { print parts[2]; exit }
    }
  ' "${LOCKFILE}"
}

current_version="$(read_manifest_version)"
if [[ -z "${current_version}" ]]; then
  die "无法从 ${MAIN_MANIFEST} 读取 [package].version。"
fi
for app_manifest in "${APP_MANIFESTS[@]}"; do
  app_version="$(read_manifest_version "${app_manifest}")"
  if [[ -z "${app_version}" ]]; then
    die "无法从 ${app_manifest} 读取 [package].version。"
  fi
  if [[ "${app_version}" != "${current_version}" ]]; then
    die "统一版本校验失败：${app_manifest} 为 ${app_version}，main 为 ${current_version}。请先手动对齐再发 beta。"
  fi
done
echo "统一版本校验通过：main 与三个 app crate 均为 ${current_version}"

# 2. 计算目标版本。
if [[ "${NEXT}" == "true" ]]; then
  if [[ ! "${current_version}" =~ ^([0-9]+\.[0-9]+\.[0-9]+)-beta\.([0-9]+)$ ]]; then
    die "当前版本 ${current_version} 不是 X.Y.Z-beta.N，--next 只能递增 beta 序号；首次发测试版请用 --base X.Y.Z。"
  fi
  BASE="${BASH_REMATCH[1]}"
  VERSION="${BASE}-beta.$((10#${BASH_REMATCH[2]} + 1))"
else
  case "${current_version}" in
    "${BASE}-beta."*)
      die "当前已是 ${current_version}，继续发测试版请用 --next。"
      ;;
  esac
  VERSION="${BASE}-beta.1"
fi
TAG="v${VERSION}"

echo "本次 beta 发布：${VERSION}（当前 ${current_version}，tag ${TAG}）"

# 3. 前置校验：分支、工作区、tag 唯一性。
BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "${BRANCH}" != "dev" ]]; then
  die "beta 版本线必须走 dev（当前：${BRANCH}）。稳定版请走 dev -> main 发布 PR + script/release-tag.sh。"
fi
echo "分支校验通过：dev"

if [[ -n "$(git status --porcelain)" ]]; then
  if [[ "${DRY_RUN}" == "true" ]]; then
    echo "警告：工作区不干净，--dry-run 继续预览；正式执行前请先提交或暂存变更。" >&2
  else
    die "工作区不干净，请先提交或暂存变更（git status --porcelain 必须为空）。"
  fi
fi

if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
  die "本地已存在标签 ${TAG}。如需重发测试版，请用 --next 递增序号。"
fi
if git ls-remote --exit-code --tags "${REMOTE}" "refs/tags/${TAG}" >/dev/null 2>&1; then
  die "远端 ${REMOTE} 已存在标签 ${TAG}。如需重发测试版，请用 --next 递增序号。"
fi
echo "tag 唯一性校验通过：${TAG} 在本地与 ${REMOTE} 均不存在"

# 4. --dry-run：只打印计划，绝不写文件/提交/打 tag。
if [[ "${DRY_RUN}" == "true" ]]; then
  echo ""
  echo "== --dry-run：以下操作不会执行 =="
  echo "  1. 四个 manifest 的 [package].version: ${current_version} -> ${VERSION}"
  echo "     - ${MAIN_MANIFEST}"
  for app_manifest in "${APP_MANIFESTS[@]}"; do
    echo "     - ${app_manifest}"
  done
  echo "  2. ${LOCKFILE}: 同步 main 与三个 app 包版本到 ${VERSION}"
  echo "  3. CHANGELOG.md: python3 script/changelog.py upsert --tag ${TAG} --date $(date +%F) --notes-file ${NOTES_FILE} --changelog ${CHANGELOG}"
  echo "  4. git add 四个 manifest ${LOCKFILE} ${CHANGELOG}"
  echo "  5. git commit -m \"chore(release): 发布 ${TAG} 测试版\""
  echo "  6. git tag -a ${TAG} -m ${TAG}"
  echo "  7. git push ${REMOTE} dev && git push ${REMOTE} ${TAG}"
  echo ""
  echo "本次 tag：${TAG}"
  echo "通道：beta"
  echo "R2 更新清单将写到：updates/beta/latest.json"
  echo "稳定清单 updates/latest.json 不会受影响。"
  exit 0
fi

# 5. 改写 main 与三个 app crate 的 [package].version（统一版本）。
for bump_manifest in "${MAIN_MANIFEST}" "${APP_MANIFESTS[@]}"; do
python3 - "${bump_manifest}" "${VERSION}" <<'PY'
import re
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
version = sys.argv[2]
lines = manifest_path.read_text(encoding="utf-8").splitlines(keepends=True)

in_package = False
updated = False
for index, line in enumerate(lines):
    stripped = line.strip()
    if stripped == "[package]":
        in_package = True
        continue
    if in_package and stripped.startswith("["):
        in_package = False
    if in_package and re.match(r'^version = "', line):
        ending = "\r\n" if line.endswith("\r\n") else "\n" if line.endswith("\n") else ""
        lines[index] = f'version = "{version}"{ending}'
        updated = True
        break

if not updated:
    raise SystemExit(f"[package].version not found in {manifest_path}")

manifest_path.write_text("".join(lines), encoding="utf-8")
PY
done

# 6. 同步 Cargo.lock：优先让 cargo 自己刷新，离线失败再精确改写。
if cargo metadata --offline --no-deps --format-version 1 >/dev/null 2>&1; then
  echo "Cargo.lock 已通过 cargo metadata 刷新。"
else
  echo "警告：cargo metadata --offline 失败，改为精确改写 ${LOCKFILE} 中 main 与三个 app 包的版本。" >&2
  python3 - "${LOCKFILE}" "${VERSION}" <<'PY'
import re
import sys
from pathlib import Path

lock_path = Path(sys.argv[1])
version = sys.argv[2]
names = {"main", "navop", "navop-shell", "navop-data"}
lines = lock_path.read_text(encoding="utf-8").splitlines(keepends=True)

in_package = False
package_name = ""
updated = set()
for index, line in enumerate(lines):
    stripped = line.strip()
    if stripped == "[[package]]":
        in_package = True
        package_name = ""
        continue
    if stripped.startswith("[") and stripped != "[[package]]":
        in_package = False
    if in_package:
        name_match = re.match(r'^name = "([^"]*)"$', stripped)
        if name_match:
            package_name = name_match.group(1)
        if package_name in names and re.match(r'^version = "', stripped):
            ending = "\r\n" if line.endswith("\r\n") else "\n" if line.endswith("\n") else ""
            lines[index] = f'version = "{version}"{ending}'
            updated.add(package_name)

missing = names - updated
if missing:
    raise SystemExit(f'packages not found in lockfile: {sorted(missing)}')

lock_path.write_text("".join(lines), encoding="utf-8")
PY
fi

for lock_package in main navop navop-shell navop-data; do
  lock_version="$(read_lock_version "${lock_package}")"
  if [[ "${lock_version}" != "${VERSION}" ]]; then
    die "${LOCKFILE} 中 ${lock_package} 包版本为 ${lock_version:-<missing>}，未能同步到 ${VERSION}。"
  fi
done
echo "Cargo.lock 已同步：main / navop / navop-shell / navop-data -> ${VERSION}"

# 7. 写入 CHANGELOG.md 条目并回读校验（release.yml 硬校验的就是这条）。
python3 script/changelog.py upsert \
  --tag "${TAG}" \
  --date "$(date +%F)" \
  --notes-file "${NOTES_FILE}" \
  --changelog "${CHANGELOG}"
python3 script/changelog.py validate \
  --tag "${TAG}" \
  --changelog "${CHANGELOG}"
echo "CHANGELOG.md 已写入并校验：${TAG}"

# 8. 提交、打 tag、推送。
git add -- "${MAIN_MANIFEST}" "${APP_MANIFESTS[@]}" "${LOCKFILE}" "${CHANGELOG}"
git commit -m "chore(release): 发布 ${TAG} 测试版"
COMMITTED="true"

git tag -a "${TAG}" -m "${TAG}"
TAGGED="true"

echo "推送 dev 与 tag 到 ${REMOTE} ..."
git push "${REMOTE}" dev
git push "${REMOTE}" "${TAG}"
PUSHED="true"

echo ""
echo "完成：已推送 ${TAG}，release-trigger.yml -> release.yml 将按 beta 通道执行。"
echo "本次 tag：${TAG}"
echo "通道：beta（GitHub Release 会标记 --prerelease）"
echo "R2 更新清单将写到：updates/beta/latest.json"
echo "稳定清单 updates/latest.json 不会受影响；老客户端仍只读稳定清单。"
echo ""
echo "beta 稳定后：走正常 dev -> main 发布 PR，把版本改成 ${BASE} 后运行 script/release-tag.sh v${BASE}。"
