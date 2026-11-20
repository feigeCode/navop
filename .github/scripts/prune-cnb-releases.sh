#!/usr/bin/env bash
# 清理 navop 在 CNB 上的 Release，只保留最新若干个版本。
#
# 背景：CNB 对象存储免费额度 100 GiB，统计「制品、LFS 对象、图片及附件」，
# 而 navop 每个版本在 CNB Release 上约有 27 个安装包（约 1.75 GiB）。发版节奏
# 一快就会顶满 100 GiB 并开始按 ¥1/GiB/月 计费。GitHub Release 才是分发主线，
# CNB 只是国内镜像，旧版本没有保留价值。
#
# 安全约束（见 crates/app_kit/src/update/mirror.rs）：
#   * 更新器先用 git refs 广告发现 tag，再取「最新 tag」Release 里的 feed-*.json，
#     所以最新版本必须留着，且只删 Release —— 不碰 git tag。
#   * 保留集合固定包含：本次同步的 tag、版本号最大的稳定版、版本号最大的 beta，
#     再叠加版本号最大的 N 个版本。稳定版/beat 保底是为了让两个通道的客户端
#     永远拿得到最新清单，即使 N 个最新版本恰好全是 beta。
#
# 环境变量：
#   CNB_REPOSITORY         navop-dev/navop
#   CNB_TOKEN              具备 repo-release:rw 的 CNB 访问令牌
#   SYNCED_TAG             本次刚同步完的 tag（必须存在于 CNB，否则拒绝清理）
#   CNB_RELEASE_RETENTION  保留的版本数，默认 5
#   CNB_PRUNE_DRY_RUN      true 时只打印计划，不实际删除
set -uo pipefail

: "${CNB_REPOSITORY:?CNB_REPOSITORY is required}"
: "${CNB_TOKEN:?CNB_TOKEN is required}"
: "${SYNCED_TAG:?SYNCED_TAG is required}"

RETENTION="${CNB_RELEASE_RETENTION:-5}"
DRY_RUN="${CNB_PRUNE_DRY_RUN:-false}"
API="https://api.cnb.cool/${CNB_REPOSITORY}"

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

fail() { echo "::error::$*"; exit 1; }
warn() { echo "::warning::$*"; }

human_size() {
  awk -v b="$1" 'BEGIN {
    split("B KiB MiB GiB TiB", u, " "); i = 1
    while (b >= 1024 && i < 5) { b /= 1024; i++ }
    printf "%.2f %s", b, u[i]
  }'
}

# ---------------------------------------------------------------- 拉取 Release
# page_size 默认只有 10，必须显式传；逐页拉到取空为止。
fetch_releases() {
  local page=1 count status
  : >"${work}/releases.ndjson"
  while [ "${page}" -le 20 ]; do
    status="$(curl -sS -o "${work}/page.json" -w '%{http_code}' \
      -H "Authorization: Bearer ${CNB_TOKEN}" \
      -H "Accept: application/json" \
      "${API}/-/releases?page=${page}&page_size=100")" || {
      warn "Cannot list CNB releases (network error on page ${page})"
      return 1
    }
    case "${status}" in
      200) ;;
      401 | 403)
        echo "::error::CNB token cannot list releases (HTTP ${status}): $(head -c 300 "${work}/page.json")"
        return 2
        ;;
      *)
        warn "Cannot list CNB releases (HTTP ${status} on page ${page}): $(head -c 300 "${work}/page.json")"
        return 1
        ;;
    esac
    count="$(jq 'length' "${work}/page.json" 2>/dev/null || echo 0)"
    if [ "${count}" -eq 0 ]; then
      break
    fi
    jq -c '.[] | {id: .id, tag: .tag_name, size: ([.assets[]?.size] | add // 0)}' \
      "${work}/page.json" >>"${work}/releases.ndjson"
    if [ "${count}" -lt 100 ]; then
      break
    fi
    page=$((page + 1))
  done
  [ -s "${work}/releases.ndjson" ] || {
    warn "CNB returned no releases; nothing to prune."
    return 1
  }
}

fetch_releases
case $? in
  0) ;;
  2) exit 1 ;;
  *) exit 0 ;; # 瞬时故障不该让发版流程变红，下次构建会再清一次
esac

total="$(jq -r '.tag' "${work}/releases.ndjson" | wc -l | tr -d ' ')"
echo "CNB has ${total} release(s); retention policy keeps the newest ${RETENTION} plus the newest stable/beta entry point."

# 本次同步的 tag 不在 CNB 上就什么都不删：宁可留下旧版本，也不能在「新版本
# 其实没同步成功」时把镜像清空。
if ! jq -r '.tag' "${work}/releases.ndjson" | grep -Fxq "${SYNCED_TAG}"; then
  fail "Refusing to prune: ${SYNCED_TAG} is not present on CNB"
fi

# ---------------------------------------------------------------- 计算保留集合
tags_sorted="$(jq -r '.tag' "${work}/releases.ndjson" | sort -V)"

keep=" ${SYNCED_TAG} "

newest="$(printf '%s\n' "${tags_sorted}" | tail -n "${RETENTION}")"
for t in ${newest}; do
  keep="${keep}${t} "
done

# 稳定版与 beta 各保底一个：两条通道的客户端都按「最新 tag」取 feed-*.json。
stable_tag="$(printf '%s\n' "${tags_sorted}" | grep -v -- '-' | tail -n 1 || true)"
beta_tag="$(printf '%s\n' "${tags_sorted}" | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+-' | tail -n 1 || true)"
[ -n "${stable_tag}" ] && keep="${keep}${stable_tag} "
[ -n "${beta_tag}" ] && keep="${keep}${beta_tag} "

# 去重并排序，便于阅读日志。
keep_list="$(printf '%s\n' ${keep} | sort -uV | tr '\n' ' ')"
echo "Keeping releases: ${keep_list}"
echo "  ${SYNCED_TAG} = just synced; ${stable_tag:-none} = newest stable; ${beta_tag:-none} = newest beta; newest ${RETENTION} by version"

if [ "$(printf '%s\n' ${keep} | sort -u | wc -l | tr -d ' ')" -ge "${total}" ]; then
  echo "Nothing to prune."
  exit 0
fi

# ---------------------------------------------------------------- 删除旧 Release
deleted=0
freed=0
failed=0
while IFS=$'\t' read -r id tag size; do
  case " ${keep} " in
    *" ${tag} "*)
      echo "keep   ${tag} ($(human_size "${size}"))"
      continue
      ;;
  esac
  if [ "${DRY_RUN}" = "true" ]; then
    echo "delete ${tag} ($(human_size "${size}"), id=${id}) [dry-run]"
    deleted=$((deleted + 1))
    freed=$((freed + size))
    continue
  fi
  status="$(curl -sS -o "${work}/del.json" -w '%{http_code}' -X DELETE \
    -H "Authorization: Bearer ${CNB_TOKEN}" \
    -H "Accept: application/json" \
    "${API}/-/releases/${id}")" || status="000"
  case "${status}" in
    200 | 202 | 204)
      echo "delete ${tag} ($(human_size "${size}"), id=${id})"
      deleted=$((deleted + 1))
      freed=$((freed + size))
      ;;
    404)
      echo "delete ${tag} (already gone)"
      deleted=$((deleted + 1))
      freed=$((freed + size))
      ;;
    401 | 403)
      echo "::error::CNB token lacks release write scope (HTTP ${status}) when deleting ${tag}: $(head -c 300 "${work}/del.json")"
      echo "::error::Create a CNB access token with repo-release:rw at https://cnb.cool/profile/token and update the CNB_TOKEN secret."
      exit 1
      ;;
    *)
      warn "Failed to delete ${tag} (HTTP ${status}): $(head -c 200 "${work}/del.json")"
      failed=$((failed + 1))
      ;;
  esac
done < <(jq -r '[.id, .tag, .size] | @tsv' "${work}/releases.ndjson")

echo "Pruned ${deleted} release(s), freed $(human_size "${freed}") of CNB object storage (kept ${keep_list})."
if [ "${failed}" -gt 0 ]; then
  warn "${failed} release(s) could not be deleted; the next build will retry."
fi
