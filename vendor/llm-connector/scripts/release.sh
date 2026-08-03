#!/usr/bin/env bash
set -euo pipefail

# Unified release script: sync to GitHub and publish to crates.io
#
# Usage:
#   scripts/release.sh release <VERSION>   # bump version, build, commit, tag, push, publish
#   scripts/release.sh publish             # publish current version (requires cargo login or token)
#   scripts/release.sh check               # show local vs crates.io version
#
# Env:
#   REMOTE=origin                 # Git remote to push to (default: origin)
#   CARGO_REGISTRY_TOKEN=...      # crates.io token if not logged in via `cargo login`

REMOTE="${REMOTE:-origin}"
CRATE_NAME="${CRATE_NAME:-llm-connector}"
TOML_FILE="${TOML_FILE:-Cargo.toml}"

have_cargo_edit() {
  command -v cargo >/dev/null 2>&1 && cargo set-version --help >/dev/null 2>&1
}

get_local_version() {
  # Read version under [package] using portable awk
  awk '
    /^\[package\]/ { in_pkg=1; next }
    in_pkg && /version[ \t]*=/ {
      q1 = index($0, "\"");
      if (q1 > 0) {
        rest = substr($0, q1+1);
        q2 = index(rest, "\"");
        if (q2 > 0) { print substr(rest, 1, q2-1); exit }
      }
    }
    /^\[/ && in_pkg { exit }
  ' "$TOML_FILE"
}

get_remote_version() {
  local api json
  api="https://crates.io/api/v1/crates/${CRATE_NAME}"
  if ! json=$(curl -fsSL "$api" 2>/dev/null); then
    echo ""
    return 0
  fi
  echo "$json" | sed -nE 's/.*"max_version":"([^"]+)".*/\1/p'
}

set_version() {
  local new="$1"
  if [[ -z "$new" ]]; then
    echo "No version provided" >&2
    exit 2
  fi
  if have_cargo_edit; then
    cargo set-version "$new"
  else
    # macOS/BSD sed in-place
    sed -i '' -E "s/^([[:space:]]*version[[:space:]]*=[[:space:]]*\")([^\"]+)(\".*)$/\\1${new}\\3/" "$TOML_FILE"
  fi
}

git_require_clean() {
  if ! git diff --quiet || ! git diff --cached --quiet; then
    echo "Git working tree not clean; please commit or stash changes." >&2
    exit 2
  fi
}

do_build_checks() {
  cargo build
  cargo build --features streaming
}

do_publish() {
  if [[ -n "${CARGO_REGISTRY_TOKEN:-}" ]]; then
    CARGO_REGISTRY_TOKEN="$CARGO_REGISTRY_TOKEN" cargo publish
  else
    cargo publish
  fi
}

cmd_release() {
  local ver="${1:-}"
  if [[ -z "$ver" ]]; then
    echo "Usage: scripts/release.sh release <VERSION>" >&2
    exit 2
  fi

  git_require_clean
  echo "Bumping version to $ver"
  set_version "$ver"

  echo "Running build checks"
  do_build_checks

  echo "Committing version bump"
  git add "$TOML_FILE" Cargo.lock || true
  git commit -m "chore(release): v${ver}" || echo "No changes to commit"

  echo "Tagging v${ver}"
  git tag -a "v${ver}" -m "Release v${ver}" || echo "Tag exists, continuing"

  echo "Pushing to $REMOTE"
  git push "$REMOTE" || true
  git push "$REMOTE" "v${ver}" || true

  echo "Publishing to crates.io"
  do_publish

  echo "Verifying remote version"
  local local_ver remote_ver
  local_ver=$(get_local_version)
  sleep 3
  remote_ver=$(get_remote_version)
  echo "Local version:  ${local_ver:-unknown}"
  echo "Remote version: ${remote_ver:-unknown}"
}

cmd_publish() {
  do_build_checks
  do_publish
}

cmd_check() {
  local local_ver remote_ver
  local_ver=$(get_local_version)
  remote_ver=$(get_remote_version)
  echo "Local version:  ${local_ver:-unknown}"
  echo "Remote version: ${remote_ver:-unknown}"
  if [[ -n "$local_ver" && -n "$remote_ver" && "$local_ver" != "$remote_ver" ]]; then
    echo "Version mismatch detected" >&2
    exit 3
  fi
}

MODE="${1:-}"
case "$MODE" in
  release)
    shift
    cmd_release "${1:-}"
    ;;
  publish)
    cmd_publish
    ;;
  check)
    cmd_check
    ;;
  *)
    echo "Usage: scripts/release.sh [release <VERSION>|publish|check]" >&2
    exit 2
    ;;
esac