#!/usr/bin/env bash
set -euo pipefail

# 用法：
#   script/release-tag.sh v1.2.3
#   FORCE_RETAG=true script/release-tag.sh v1.2.3   # 覆盖同名 tag
#
# 前置条件（缺一不可）：
#   1. 版本 bump（main/Cargo.toml + Cargo.lock）已随发布 PR 合入 main，
#      即 main/Cargo.toml 的版本已等于 tag；
#   2. CHANGELOG.md 已包含该 tag 的双语条目并已提交；
#   3. 当前在 main 分支且工作区干净。
#
# 作用：校验以上条件后创建并推送 tag，触发 GitHub Actions Release。
#
# 注意：main 是受保护分支（要求 PR + CI gate，且 enforce_admins），
# 因此本脚本不再直接提交/推送版本变更——那会被分支保护拒绝（GH006）。
# 版本 bump 请放进 dev -> main 的发布 PR。

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"
cd "${REPO_ROOT}"

TAG="${1:-}"
REMOTE="${REMOTE:-origin}"
FORCE_RETAG="${FORCE_RETAG:-false}"
MAIN_MANIFEST="${MAIN_MANIFEST:-${REPO_ROOT}/main/Cargo.toml}"
RELEASE_VERSION="${TAG#v}"

if [[ -z "${TAG}" ]]; then
  echo "错误：缺少 tag。用法：script/release-tag.sh v1.2.3" >&2
  exit 1
fi
if [[ ! "${TAG}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "错误：TAG 格式非法。示例：v1.2.3 或 v1.2.3-rc.1" >&2
  exit 1
fi

echo "准备发布：tag=${TAG} remote=${REMOTE}"

BRANCH="$(git rev-parse --abbrev-ref HEAD)"
if [[ "${BRANCH}" != "main" ]]; then
  echo "错误：发布 tag 必须打在 main 上（当前：${BRANCH}）。" >&2
  exit 1
fi

echo "校验 CHANGELOG.md 中的双语发布说明：${TAG}"
python3 script/changelog.py validate \
  --tag "${TAG}" \
  --changelog CHANGELOG.md

if ! git ls-files --error-unmatch CHANGELOG.md script/changelog.py >/dev/null 2>&1; then
  echo "错误：CHANGELOG.md 和 script/changelog.py 必须先加入 Git。" >&2
  exit 1
fi
if ! git diff --quiet HEAD -- CHANGELOG.md script/changelog.py; then
  echo "错误：发布说明或 changelog 工具尚未提交。请先提交 CHANGELOG.md 和 script/changelog.py。" >&2
  exit 1
fi
if [[ -n "$(git status --porcelain)" ]]; then
  echo "错误：工作区不干净，请先提交或暂存变更。" >&2
  exit 1
fi

current_version="$(
  awk -F'"' '
    /^\[package\]$/ { in_package = 1; next }
    /^\[/ { in_package = 0 }
    in_package && /^version = "/ { print $2; exit }
  ' "${MAIN_MANIFEST}"
)"
if [[ "${current_version}" != "${RELEASE_VERSION}" ]]; then
  echo "错误：main/Cargo.toml 版本为 ${current_version:-<missing>}，与 ${TAG} 不一致。" >&2
  echo "请先把版本 bump 放进 dev -> main 的发布 PR 并合入，再运行本脚本。" >&2
  exit 1
fi
echo "main/Cargo.toml 版本已匹配：${RELEASE_VERSION}"

if git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null; then
  if [[ "${FORCE_RETAG}" == "true" ]]; then
    echo "本地存在同名标签，正在删除：${TAG}"
    git tag -d "${TAG}"
  else
    echo "错误：本地已存在标签 ${TAG}。如需覆盖：FORCE_RETAG=true script/release-tag.sh ${TAG}" >&2
    exit 1
  fi
fi

if git ls-remote --exit-code --tags "${REMOTE}" "refs/tags/${TAG}" >/dev/null 2>&1; then
  if [[ "${FORCE_RETAG}" == "true" ]]; then
    echo "远端存在同名标签，正在删除：${TAG}"
    git push "${REMOTE}" ":refs/tags/${TAG}"
  else
    echo "错误：远端已存在标签 ${TAG}。如需覆盖：FORCE_RETAG=true script/release-tag.sh ${TAG}" >&2
    exit 1
  fi
fi

echo "创建并推送标签：${TAG}"
git tag -a "${TAG}" -m "${TAG}"
git push "${REMOTE}" "${TAG}"

echo "完成：已触发 GitHub Actions Release 流程。"
echo "请在 GitHub Actions 查看 release.yml 运行状态。"
