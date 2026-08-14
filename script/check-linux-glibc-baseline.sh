#!/usr/bin/env bash

set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "Usage: $0 <elf-binary> <maximum-glibc-version>" >&2
  exit 2
fi

binary="$1"
maximum_version="$2"
readelf_command="${READELF:-readelf}"

if [ ! -f "$binary" ]; then
  echo "Error: ELF binary not found: $binary" >&2
  exit 2
fi

if [[ ! "$maximum_version" =~ ^[0-9]+(\.[0-9]+)+$ ]]; then
  echo "Error: invalid maximum GLIBC version: $maximum_version" >&2
  exit 2
fi

if ! version_info="$("$readelf_command" --version-info "$binary")"; then
  echo "Error: failed to inspect GLIBC requirements for $binary" >&2
  exit 1
fi

versions="$(
  printf '%s\n' "$version_info" \
    | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' \
    | sed 's/^GLIBC_//' \
    | sort -Vu \
    || true
)"

if [ -z "$versions" ]; then
  echo "Error: readelf did not report any GLIBC versions for $binary" >&2
  exit 1
fi

highest_version="$(printf '%s\n' "$versions" | tail -n 1)"
echo "highest required GLIBC version: $highest_version (maximum: $maximum_version)"

lowest_version="$(
  printf '%s\n%s\n' "$highest_version" "$maximum_version" \
    | sort -V \
    | head -n 1
)"
if [ "$lowest_version" != "$highest_version" ]; then
  echo "Error: $binary requires GLIBC_$highest_version, above the supported GLIBC_$maximum_version baseline" >&2
  exit 1
fi
