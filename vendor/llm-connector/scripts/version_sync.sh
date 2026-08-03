#!/usr/bin/env bash
set -euo pipefail

# Continuous version sync utility for Cargo.toml
# Modes:
#  - sync: sync local version to crates.io max_version
#  - bump <VERSION>: set local version to provided VERSION
#  - check: show local and remote versions

MODE="${1:-sync}"
CRATE_NAME="${CRATE_NAME:-llm-connector}"
TOML_FILE="${TOML_FILE:-Cargo.toml}"

have_cargo_edit() {
  command -v cargo >/dev/null 2>&1 && cargo set-version --help >/dev/null 2>&1
}

get_local_version() {
  # Find first occurrence of version = "x.y.z" under [package]
  # Portable implementation using awk for macOS/BSD compatibility
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
  # Query crates.io for max_version
  local api
  api="https://crates.io/api/v1/crates/${CRATE_NAME}"
  local json
  if ! json=$(curl -fsSL "$api" 2>/dev/null); then
    echo "" # empty if offline
    return 0
  fi
  # Use extended regex for portability on macOS (BSD sed)
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
    # macOS sed in-place requires an empty suffix argument
    # Escape inner double quotes inside the regex; keep outer double quotes for variable expansion
    sed -i '' -E "s/^([[:space:]]*version[[:space:]]*=[[:space:]]*\")([^\"]+)(\".*)$/\\1${new}\\3/" "$TOML_FILE"
  fi
}

case "$MODE" in
  sync)
    remote_ver=$(get_remote_version)
    if [[ -z "$remote_ver" ]]; then
      echo "Failed to fetch remote version from crates.io (offline?)." >&2
      exit 1
    fi
    echo "Syncing local version to crates.io: $remote_ver"
    set_version "$remote_ver"
    ;;
  bump)
    bump_ver="${2:-}"
    if [[ -z "$bump_ver" ]]; then
      echo "Usage: scripts/version_sync.sh bump <VERSION>" >&2
      exit 2
    fi
    echo "Bumping local version to: $bump_ver"
    set_version "$bump_ver"
    ;;
  check)
    local_ver=$(get_local_version)
    remote_ver=$(get_remote_version)
    echo "Local version:  ${local_ver:-unknown}"
    echo "Remote version: ${remote_ver:-unknown}"
    if [[ -n "$local_ver" && -n "$remote_ver" && "$local_ver" != "$remote_ver" ]]; then
      echo "Version mismatch detected" >&2
      exit 3
    fi
    ;;
  *)
    echo "Unknown mode: $MODE" >&2
    echo "Usage: scripts/version_sync.sh [sync|check|bump <VERSION>]" >&2
    exit 2
    ;;
esac