#!/usr/bin/env bash
#
# Host side of the Linux GPU dependency stack build. Drives
# script/package-linux-gpu-stack-build.sh inside a RHEL 8 generation container,
# because the stack has to match the glibc baseline Navop is built against and
# only that distribution generation ships a Mesa user space at it.
#
# Docker is used for its package manager, not for cross-architecture emulation:
# the container runs natively on the host runner while the RPMs it resolves
# target --target's architecture. Nothing in the pipeline executes a collected
# library, so no emulator is required.

set -euo pipefail

repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
default_image="rockylinux:8"

binary=""
target=""
output=""
image="$default_image"
source_distribution=""

usage() {
  cat <<'USAGE'
Usage: package-linux-gpu-stack-docker.sh --binary PATH --target TRIPLE --output DIR [options]

Options:
  --binary PATH                release Navop binary for the target
  --target TRIPLE              x86_64-unknown-linux-gnu or aarch64-unknown-linux-gnu
  --output DIR                 directory to receive navop-gpu-stack-linux-<arch>.tar.gz
  --image NAME                 build container (default rockylinux:8)
  --source-distribution TEXT   overrides the distribution string in the manifest
  --help
USAGE
}

fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

parse_arguments() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --binary)
        shift
        [ "$#" -gt 0 ] || fail "--binary requires a path"
        binary="$1"
        ;;
      --target)
        shift
        [ "$#" -gt 0 ] || fail "--target requires a triple"
        target="$1"
        ;;
      --output)
        shift
        [ "$#" -gt 0 ] || fail "--output requires a directory"
        output="$1"
        ;;
      --image)
        shift
        [ "$#" -gt 0 ] || fail "--image requires a name"
        image="$1"
        ;;
      --source-distribution)
        shift
        [ "$#" -gt 0 ] || fail "--source-distribution requires text"
        source_distribution="$1"
        ;;
      -h | --help)
        usage
        exit 0
        ;;
      *)
        fail "unknown argument: $1 (try --help)"
        ;;
    esac
    shift
  done

  [ -n "$binary" ] || fail "--binary is required"
  [ -n "$target" ] || fail "--target is required"
  [ -n "$output" ] || fail "--output is required"
}

main() {
  parse_arguments "$@"
  command -v docker >/dev/null 2>&1 ||
    fail "docker is required to resolve the RHEL 8 dependency tree"

  [ -f "$binary" ] || fail "release binary does not exist: $binary"
  binary="$(cd -- "$(dirname -- "$binary")" && pwd)/$(basename -- "$binary")"

  mkdir -p "$output"
  output="$(cd -- "$output" && pwd)"

  # The container only ever sees three mounts: the repository (read only), the
  # binary's directory, and the output directory.
  local binary_directory
  binary_directory="$(dirname -- "$binary")"

  local container_arguments=(
    --binary "/binary/$(basename -- "$binary")"
    --target "$target"
    --output /out
  )
  if [ -n "$source_distribution" ]; then
    container_arguments+=(--source-distribution "$source_distribution")
  fi

  docker run --rm \
    -v "$repository_root:/workspace:ro" \
    -v "$binary_directory:/binary:ro" \
    -v "$output:/out" \
    --workdir /workspace \
    "$image" \
    bash /workspace/script/package-linux-gpu-stack-build.sh "${container_arguments[@]}"

  # Resolve the newest archive with portable test operators only: the GNU-only
  # `find` time predicates are unavailable on a BSD userland.
  local archive="" candidate
  for candidate in "$output"/navop-gpu-stack-linux-*.tar.gz; do
    [ -f "$candidate" ] || continue
    if [ -z "$archive" ] || [ "$candidate" -nt "$archive" ]; then
      archive="$candidate"
    fi
  done
  [ -n "$archive" ] || fail "the build container produced no dependency archive in $output"
  printf 'Dependency stack written: %s\n' "$archive"
}

main "$@"
