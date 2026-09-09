#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
NAVOP_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"
TARGET_BRANCH="navop-gpui-ce"
FETCH_SOURCE="origin"
UPSTREAM_BRANCH="main"
PUSH_TARGET="git@github.com:feigeCode/gpui-component.git"
COMPONENT_DIR="${GPUI_COMPONENT_DIR:-}"
DRY_RUN="false"
NO_FETCH="false"
PUSH="false"
UPDATE_NAVOP="false"
OFFLINE="false"
SKIP_VERIFY="false"
ALLOW_DIRTY_NAVOP="false"

usage() {
  cat <<'EOF'
Usage: script/sync-gpui-component.sh [options]

Options:
  --component-dir PATH  gpui-component checkout on navop-gpui-ce
  --fetch-source VALUE  Git remote name or URL used to fetch main (default: origin)
  --upstream-branch B   Upstream branch to merge (default: main)
  --target-branch B     Compatibility branch (default: navop-gpui-ce)
  --push-target VALUE   Git remote name or URL used to push
  --push                Push the verified compatibility branch
  --update-navop        Update Navop Cargo.toml, versions, docs, and Cargo.lock
  --offline             Run Cargo update and Navop verification offline
  --no-fetch            Use the existing origin/main ref without fetching
  --skip-verify         Skip component and Navop Cargo verification
  --allow-dirty-navop   Permit revision updates on a dirty Navop worktree
  --dry-run             Merge and verify in a temporary worktree only
  -h, --help            Show this help

Recommended full sync:
  script/sync-gpui-component.sh --component-dir PATH \
    --fetch-source git@github.com:feigeCode/gpui-component.git \
    --push --update-navop
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --component-dir) COMPONENT_DIR="$2"; shift 2 ;;
    --fetch-source) FETCH_SOURCE="$2"; shift 2 ;;
    --upstream-branch) UPSTREAM_BRANCH="$2"; shift 2 ;;
    --target-branch) TARGET_BRANCH="$2"; shift 2 ;;
    --push-target) PUSH_TARGET="$2"; shift 2 ;;
    --push) PUSH="true"; shift ;;
    --update-navop) UPDATE_NAVOP="true"; shift ;;
    --offline) OFFLINE="true"; shift ;;
    --no-fetch) NO_FETCH="true"; shift ;;
    --skip-verify) SKIP_VERIFY="true"; shift ;;
    --allow-dirty-navop) ALLOW_DIRTY_NAVOP="true"; shift ;;
    --dry-run) DRY_RUN="true"; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ "${UPDATE_NAVOP}" == "true" && "${ALLOW_DIRTY_NAVOP}" != "true" ]] && \
   [[ -n "$(git -C "${NAVOP_ROOT}" status --porcelain)" ]]; then
  echo "Navop worktree is not clean; commit changes or use --allow-dirty-navop." >&2
  exit 1
fi
if [[ "${DRY_RUN}" == "true" && ("${PUSH}" == "true" || "${UPDATE_NAVOP}" == "true") ]]; then
  echo "--dry-run cannot be combined with --push or --update-navop." >&2
  exit 2
fi

if [[ -z "${COMPONENT_DIR}" ]]; then
  GIT_COMMON_DIR="$(git -C "${NAVOP_ROOT}" rev-parse --path-format=absolute --git-common-dir)"
  WORKSPACE_ROOT="$(dirname "$(dirname "${GIT_COMMON_DIR}")")"
  COMPONENT_DIR="${WORKSPACE_ROOT}/.worktrees/gpui-component-main"
fi
COMPONENT_DIR="$(cd "${COMPONENT_DIR}" && pwd)"

if [[ "$(git -C "${COMPONENT_DIR}" branch --show-current)" != "${TARGET_BRANCH}" ]]; then
  echo "${COMPONENT_DIR} is not on ${TARGET_BRANCH}." >&2
  exit 1
fi
if [[ -n "$(git -C "${COMPONENT_DIR}" status --porcelain)" ]]; then
  echo "gpui-component worktree is not clean: ${COMPONENT_DIR}" >&2
  exit 1
fi

BASE_REVISION="$(git -C "${COMPONENT_DIR}" rev-parse HEAD)"
if [[ "${NO_FETCH}" == "true" ]]; then
  UPSTREAM_REVISION="$(git -C "${COMPONENT_DIR}" rev-parse "origin/${UPSTREAM_BRANCH}^{commit}")"
else
  SYNC_REF="refs/navop-sync/${UPSTREAM_BRANCH}"
  git -C "${COMPONENT_DIR}" fetch "${FETCH_SOURCE}" \
    "+refs/heads/${UPSTREAM_BRANCH}:${SYNC_REF}"
  UPSTREAM_REVISION="$(git -C "${COMPONENT_DIR}" rev-parse "${SYNC_REF}^{commit}")"
fi

TEMP_ROOT="$(mktemp -d)"
TEMP_WORKTREE="${TEMP_ROOT}/worktree"
cleanup() {
  git -C "${COMPONENT_DIR}" worktree remove --force "${TEMP_WORKTREE}" >/dev/null 2>&1 || true
  rm -rf "${TEMP_ROOT}"
}
trap cleanup EXIT

git -C "${COMPONENT_DIR}" worktree add --detach "${TEMP_WORKTREE}" "${BASE_REVISION}"
if ! git -C "${TEMP_WORKTREE}" -c rerere.enabled=true merge --no-edit "${UPSTREAM_REVISION}"; then
  echo "Automatic merge stopped on conflicts:" >&2
  git -C "${TEMP_WORKTREE}" diff --name-only --diff-filter=U >&2
  echo "No compatibility branch or Navop files were changed." >&2
  exit 1
fi

RESULT_REVISION="$(git -C "${TEMP_WORKTREE}" rev-parse HEAD)"
echo "Candidate revision: ${RESULT_REVISION}"

if [[ "${SKIP_VERIFY}" != "true" ]]; then
  cargo check --manifest-path "${TEMP_WORKTREE}/Cargo.toml" --locked \
    -p gpui_ce_components_base \
    -p gpui_ce_components \
    -p gpui_ce_components_shell \
    -p gpui-component-shell
  cargo test --manifest-path "${TEMP_WORKTREE}/Cargo.toml" --locked \
    -p gpui_ce_components_shell --lib --no-fail-fast
fi

if [[ "${DRY_RUN}" == "true" ]]; then
  echo "Dry run completed; ${TARGET_BRANCH} was not changed."
  exit 0
fi

if [[ "$(git -C "${COMPONENT_DIR}" rev-parse HEAD)" != "${BASE_REVISION}" ]] || \
   [[ -n "$(git -C "${COMPONENT_DIR}" status --porcelain)" ]]; then
  echo "gpui-component changed while verification was running; refusing to apply." >&2
  exit 1
fi
git -C "${COMPONENT_DIR}" merge --ff-only "${RESULT_REVISION}"

if [[ "${PUSH}" == "true" ]]; then
  git -C "${COMPONENT_DIR}" push "${PUSH_TARGET}" \
    "${TARGET_BRANCH}:${TARGET_BRANCH}"
fi

if [[ "${UPDATE_NAVOP}" == "true" ]]; then
  UPDATE_ARGS=(
    "${SCRIPT_DIR}/update_gpui_component_revision.py"
    --component-dir "${COMPONENT_DIR}"
    --revision "${RESULT_REVISION}"
  )
  CARGO_ARGS=()
  if [[ "${OFFLINE}" == "true" ]]; then
    UPDATE_ARGS+=(--offline)
    CARGO_ARGS+=(--offline)
  fi
  python3 "${UPDATE_ARGS[@]}"
  if [[ "${SKIP_VERIFY}" != "true" ]]; then
    cargo "${CARGO_ARGS[@]}" check --manifest-path "${NAVOP_ROOT}/Cargo.toml" --locked -p main
    cargo "${CARGO_ARGS[@]}" test --manifest-path "${NAVOP_ROOT}/Cargo.toml" --locked \
      -p main shell_plugin_host::components --no-fail-fast
  fi
fi

echo "gpui-component sync completed: ${RESULT_REVISION}"
