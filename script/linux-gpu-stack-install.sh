#!/bin/bash
#
# Navop Linux GPU and desktop dependency stack installer.
#
# Navop links against a small set of system libraries and reaches EGL through
# dlopen. On a host that ships neither a Mesa user space nor a DRI driver the
# window cannot be created and Navop reports "Failed to create surface". This
# archive carries the missing pieces: the Mesa software renderer (llvmpipe)
# plus the X11 / Wayland / input-method client libraries the binary needs.
#
# Everything is copied onto the paths the dynamic loader already searches, so
# every Navop package form (release tarball, .deb, .rpm, AppImage) picks the
# stack up with no extra configuration and no environment variables.
#
# The installer is additive. A library is only copied when the host does not
# already resolve that SONAME, and the whole Mesa stack is skipped when the
# host already exposes a usable EGL plus a DRI driver. Use --force to
# overwrite, --dry-run to preview, --uninstall to remove.
#
# Usage:
#   sudo ./install.sh             install everything the host is missing
#   ./install.sh --dry-run        print the plan without touching the system
#   sudo ./install.sh --force     overwrite libraries that already exist
#   sudo ./install.sh --prefix DIR   stage under DIR instead of / (testing)
#   sudo ./install.sh --uninstall remove the files this installer wrote
#   ./install.sh --help

set -euo pipefail

PACKAGE_NAME="navop-gpu-stack"

DRY_RUN=0
FORCE=0
UNINSTALL=0
PREFIX=""

# Mesa loads a DRI driver by file name from its compile-time search directory.
# Any driver found in these locations means the host already has a working Mesa
# and must be left alone.
HOST_DRI_DRIVER_DIRECTORIES=(
  "/usr/lib/x86_64-linux-gnu/dri"
  "/usr/lib/aarch64-linux-gnu/dri"
  "/usr/lib64/dri"
  "/usr/lib/dri"
)

# Records the loader configuration we may add, so --uninstall can take it back.
LD_CONF_FILENAME="navop-gpu-stack.conf"
LD_CONF_PATH="/etc/ld.so.conf.d/${LD_CONF_FILENAME}"

ARCH=""
TARGET=""
LIBDIR=""
DRI_DIR=""
EGL_VENDOR_DIR=""
GLIBC_BASELINE=""
SOURCE_DISTRIBUTION=""

INSTALLED_FILES=()
SKIPPED_FILES=()
REMOVED_FILES=()
KEPT_FILES=()

# Uninstall must not touch anything the archive merely could have installed.
# The manifest lists every candidate, including the entries that were skipped
# because the host already resolved them, so the set that was actually written
# is recorded here and --uninstall replays only that.
RECORD_PATH=""
INSTALL_RECORDS=()

info() { printf '%s\n' "$*"; }
warn() { printf '%s\n' "$*" >&2; }
fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

usage() {
  sed -n '3,26p' "$0" | sed 's/^# \{0,1\}//'
}

parse_arguments() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --dry-run) DRY_RUN=1 ;;
      --force) FORCE=1 ;;
      --uninstall) UNINSTALL=1 ;;
      --prefix)
        shift
        [ "$#" -gt 0 ] || fail "--prefix requires a directory"
        PREFIX="$1"
        ;;
      --prefix=*) PREFIX="${1#--prefix=}" ;;
      -h|--help)
        usage
        exit 0
        ;;
      *) fail "unknown argument: $1 (try --help)" ;;
    esac
    shift
  done

  if [ -n "$PREFIX" ]; then
    case "$PREFIX" in
      */) PREFIX="${PREFIX%/}" ;;
    esac
    [ "$PREFIX" != "/" ] || PREFIX=""
  fi
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command is not available: $1"
}

locate_ldconfig() {
  if command -v ldconfig >/dev/null 2>&1; then
    command -v ldconfig
    return 0
  fi
  local candidate
  for candidate in /sbin/ldconfig /usr/sbin/ldconfig /usr/bin/ldconfig; do
    if [ -x "$candidate" ]; then
      printf '%s\n' "$candidate"
      return 0
    fi
  done
  fail "ldconfig was not found; this host has no glibc dynamic loader"
}

resolve_package_root() {
  local source="${BASH_SOURCE[0]}"
  while [ -L "$source" ]; do
    local directory
    directory="$(cd -P -- "$(dirname -- "$source")" && pwd)"
    source="$(readlink -- "$source")"
    case "$source" in
      /*) ;;
      *) source="$directory/$source" ;;
    esac
  done
  cd -P -- "$(dirname -- "$source")" && pwd
}

load_metadata() {
  local root="$1"
  local environment_file="$root/install.env"
  local manifest_file="$root/manifest.tsv"

  [ -f "$environment_file" ] || fail "missing ${environment_file}"
  [ -f "$manifest_file" ] || fail "missing ${manifest_file}"

  # shellcheck disable=SC1090
  . "$environment_file"

  ARCH="${NAVOP_GPU_STACK_ARCH:-}"
  TARGET="${NAVOP_GPU_STACK_TARGET:-}"
  LIBDIR="${NAVOP_GPU_STACK_LIBDIR:-}"
  DRI_DIR="${NAVOP_GPU_STACK_DRI_DIR:-}"
  EGL_VENDOR_DIR="${NAVOP_GPU_STACK_EGL_VENDOR_DIR:-}"
  GLIBC_BASELINE="${NAVOP_GPU_STACK_GLIBC_BASELINE:-}"
  SOURCE_DISTRIBUTION="${NAVOP_GPU_STACK_SOURCE_DISTRIBUTION:-}"

  [ -n "$ARCH" ] || fail "install.env does not declare NAVOP_GPU_STACK_ARCH"
  [ -n "$LIBDIR" ] || fail "install.env does not declare NAVOP_GPU_STACK_LIBDIR"
}

host_architecture() {
  case "$(uname -m)" in
    x86_64|amd64) printf 'x86_64\n' ;;
    aarch64|arm64) printf 'aarch64\n' ;;
    *) uname -m ;;
  esac
}

verify_host_architecture() {
  local host
  host="$(host_architecture)"
  if [ "$host" != "$ARCH" ]; then
    fail "this archive targets ${ARCH} but the host reports ${host}; download the matching ${PACKAGE_NAME} archive"
  fi
}

soname_is_resolved() {
  local soname="$1"
  "$LD_CONFIG" -p 2>/dev/null |
    awk -v soname="$soname" '$1 == soname { found = 1 } END { exit found ? 0 : 1 }'
}

host_has_dri_driver() {
  local directory entry
  for directory in "${HOST_DRI_DRIVER_DIRECTORIES[@]}"; do
    [ -d "$directory" ] || continue
    for entry in "$directory"/*_dri.so; do
      [ -e "$entry" ] && return 0
    done
  done
  return 1
}

host_has_usable_gl() {
  # Both halves are required: glvnd needs libEGL.so.1 to dispatch, and Mesa
  # needs a DRI driver behind it. Either one alone leaves Navop unable to open
  # a window, which is exactly the state this archive exists to repair.
  soname_is_resolved "libEGL.so.1" || return 1
  host_has_dri_driver
}

warn_about_glibc() {
  [ -n "$GLIBC_BASELINE" ] || return 0
  local host_version
  host_version="$("$LD_CONFIG" --version 2>/dev/null | head -n 1 | awk '{print $NF}')"
  [ -n "$host_version" ] || return 0
  if [ "$(printf '%s\n%s\n' "$GLIBC_BASELINE" "$host_version" | sort -V | head -n 1)" != "$GLIBC_BASELINE" ]; then
    warn "warning: this host ships glibc ${host_version}, older than the ${GLIBC_BASELINE} baseline the bundled stack was built against; the libraries will be installed but may refuse to load."
  fi
}

sha256_of() {
  sha256sum -- "$1" | awk '{print $1}'
}

install_file() {
  local source="$1"
  local destination="$2"
  local expected_digest="$3"

  if [ -e "$destination" ] && [ "$FORCE" -eq 0 ]; then
    if [ "$(sha256_of "$destination")" = "$expected_digest" ]; then
      SKIPPED_FILES+=("$destination (already installed)")
    else
      SKIPPED_FILES+=("$destination (host copy kept)")
    fi
    return 0
  fi

  if [ "$DRY_RUN" -eq 1 ]; then
    INSTALLED_FILES+=("$destination (dry run)")
    return 0
  fi

  mkdir -p -- "$(dirname -- "$destination")"
  cp -- "$source" "$destination"
  chmod 0644 -- "$destination"
  INSTALLED_FILES+=("$destination")
  local record
  printf -v record '%s\t%s' "$expected_digest" "$destination"
  INSTALL_RECORDS+=("$record")
}

write_install_record() {
  if [ "$DRY_RUN" -eq 1 ] || [ "$#" -eq 0 ]; then
    return 0
  fi
  mkdir -p -- "$(dirname -- "$RECORD_PATH")"
  printf '%s\n' "$@" > "$RECORD_PATH"
}

remove_file() {
  local destination="$1"
  local expected_digest="$2"

  [ -e "$destination" ] || return 0
  if [ "$(sha256_of "$destination")" != "$expected_digest" ]; then
    KEPT_FILES+=("$destination (modified since installation)")
    return 0
  fi
  if [ "$DRY_RUN" -eq 0 ]; then
    rm -f -- "$destination"
  fi
  REMOVED_FILES+=("$destination")
}

ensure_loader_configuration() {
  local directory="$PREFIX$LIBDIR"

  if [ "$DRY_RUN" -eq 0 ]; then
    mkdir -p -- "$directory"
  fi
  [ -z "$PREFIX" ] || return 0

  # RHEL-style hosts already list /usr/lib64 in ld.so.conf; Debian-style hosts
  # do not, so the directory has to be registered explicitly.
  local existing
  for existing in /etc/ld.so.conf /etc/ld.so.conf.d/*.conf; do
    [ -f "$existing" ] || continue
    if grep -qE "^[[:space:]]*${directory}/?[[:space:]]*$" "$existing" 2>/dev/null; then
      return 0
    fi
  done

  info "Registering ${directory} in the dynamic loader configuration (${LD_CONF_PATH})"
  if [ "$DRY_RUN" -eq 0 ]; then
    mkdir -p -- /etc/ld.so.conf.d
    printf '%s\n' "$directory" > "$LD_CONF_PATH"
    INSTALLED_FILES+=("$LD_CONF_PATH")
  fi
}

drop_loader_configuration() {
  [ -f "$LD_CONF_PATH" ] || return 0
  if [ "$DRY_RUN" -eq 0 ]; then
    rm -f -- "$LD_CONF_PATH"
  fi
  REMOVED_FILES+=("$LD_CONF_PATH")
}

refresh_loader_cache() {
  [ -z "$PREFIX" ] || return 0
  [ "$DRY_RUN" -eq 0 ] || return 0
  "$LD_CONFIG" >/dev/null 2>&1 ||
    warn "warning: ldconfig failed; the loader cache may be stale until the next boot."
}

install_stack() {
  local root="$1"
  local kind relative group digest destination source

  local install_gl=1
  if [ "$FORCE" -eq 0 ] && host_has_usable_gl; then
    install_gl=0
    info "Host already provides a usable EGL and DRI driver; leaving the existing Mesa stack alone."
  fi

  while IFS=$'\t' read -r kind relative group digest; do
    [ -n "$kind" ] || continue
    [ -n "$relative" ] || fail "manifest.tsv contains an entry without a path"

    if [ "$group" = "gl" ] && [ "$install_gl" -eq 0 ]; then
      continue
    fi

    source="$root/payload/$relative"
    destination="$PREFIX/$relative"
    [ -f "$source" ] || fail "archive payload is incomplete: ${source} is missing"

    if [ "$kind" = "library" ] && [ "$FORCE" -eq 0 ] &&
      soname_is_resolved "$(basename -- "$relative")"; then
      SKIPPED_FILES+=("$(basename -- "$relative") (already provided by the host)")
      continue
    fi

    install_file "$source" "$destination" "$digest"
  done < "$root/manifest.tsv"

  if [ "$install_gl" -eq 1 ]; then
    ensure_loader_configuration
  fi

  if [ "$DRY_RUN" -eq 0 ] && [ "${#INSTALL_RECORDS[@]}" -gt 0 ]; then
    write_install_record "${INSTALL_RECORDS[@]}"
    INSTALLED_FILES+=("$RECORD_PATH")
  fi
}

uninstall_stack() {
  local digest destination

  [ -f "$RECORD_PATH" ] || fail \
    "no installation record at ${RECORD_PATH}; nothing to replay. Remove the stack by hand if it was installed by an older version."

  while IFS=$'\t' read -r digest destination; do
    [ -n "$destination" ] || continue
    remove_file "$destination" "$digest"
  done < "$RECORD_PATH"

  if [ "$DRY_RUN" -eq 0 ]; then
    rm -f -- "$RECORD_PATH"
    rmdir -- "$(dirname -- "$RECORD_PATH")" 2>/dev/null || true
  fi

  drop_loader_configuration
  refresh_loader_cache
}

report() {
  local title="$1"
  shift
  [ "$#" -gt 0 ] || return 0
  info ""
  info "$title"
  local entry
  for entry in "$@"; do
    info "  - $entry"
  done
}

main() {
  parse_arguments "$@"

  local root
  root="$(resolve_package_root)"
  load_metadata "$root"
  RECORD_PATH="$PREFIX/usr/lib/${PACKAGE_NAME}/installed.tsv"

  require_command uname
  require_command awk
  require_command sha256sum
  LD_CONFIG="$(locate_ldconfig)"

  if [ -z "$PREFIX" ] && [ "$(id -u)" -ne 0 ]; then
    fail "installing into /usr requires root; re-run with sudo, or use --prefix for a staging directory"
  fi

  verify_host_architecture

  if [ "$DRY_RUN" -eq 1 ]; then
    info "Dry run: no files will be written."
  fi

  info "${PACKAGE_NAME} for ${TARGET} (built on ${SOURCE_DISTRIBUTION})"
  info "Loader directory: $PREFIX$LIBDIR"
  info "DRI driver directory: $PREFIX$DRI_DIR"

  if [ "$UNINSTALL" -eq 1 ]; then
    uninstall_stack
    report "Removed:" "${REMOVED_FILES[@]}"
    report "Kept (modified since installation):" "${KEPT_FILES[@]}"
    info ""
    info "Uninstall complete."
    return 0
  fi

  warn_about_glibc
  install_stack "$root"
  refresh_loader_cache

  report "Installed:" "${INSTALLED_FILES[@]}"
  report "Skipped:" "${SKIPPED_FILES[@]}"

  info ""
  if [ "$DRY_RUN" -eq 1 ]; then
    info "Dry run complete. Re-run without --dry-run to apply."
  else
    info "Done. Start Navop again; no environment variables are required."
    info "Remove this stack later with: $0 --uninstall"
  fi
}

main "$@"
