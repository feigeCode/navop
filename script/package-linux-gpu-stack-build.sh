#!/usr/bin/env bash
#
# Builds the standalone Linux GPU dependency stack *inside* a RHEL 8 generation
# container. scripts/package-linux-gpu-stack-docker.sh is the host side of this;
# it re-enters here with the repository and the output directory mounted.
#
# Why a container at all: the stack has to satisfy the glibc baseline Navop
# itself is built against (2.28), because the host dynamic loader resolves these
# libraries directly. Only a RHEL 8 generation distribution ships a Mesa user
# space at that baseline, and collecting it means driving dnf.
#
# The RPMs are downloaded with an empty --installroot on purpose. dnf skips
# dependencies that are already installed on the running system, so an
# installroot-less download silently omits the base libraries the closure needs
# (libmount, libblkid, ...). With an empty root dnf resolves the whole tree, and
# --releasever has to be stated explicitly because an empty root has no rpmdb
# to read the release version from.
#
# Nothing here executes the collected libraries; the packager only reads ELF
# headers, which is why a single container can also produce the other
# architecture with --forcearch.

set -euo pipefail

PROGRAM_NAME="navop-gpu-stack build"
DEFAULT_GLIBC_BASELINE="2.28"

# Packages that provide the sonames Navop's own DT_NEEDED closure and the Mesa
# EGL software renderer reach. The packager fails loudly when a needed soname
# has no provider, so this list is verified rather than guesswork: a missing
# entry surfaces as "missing required shared library ... download the RPM".
GPU_STACK_PACKAGES=(
  mesa-dri-drivers mesa-libEGL mesa-libgbm mesa-libglapi
  libglvnd-egl libglvnd
  libwayland-client libwayland-server
  libxkbcommon libxkbcommon-x11 libxcb
  libX11 libX11-xcb libXext libXau libXdmcp
  libdrm libxshmfence freetype libpng
  libstdc++ elfutils-libelf systemd-libs libmount libblkid
  libcap xz-libs lz4-libs libgcrypt libidn2
  libselinux libsepol pcre2 libunistring
)

binary=""
target=""
output=""
source_distribution=""
glibc_baseline="$DEFAULT_GLIBC_BASELINE"

usage() {
  cat <<'USAGE'
Usage: package-linux-gpu-stack-build.sh --binary PATH --target TRIPLE --output DIR [options]

Runs inside the RHEL 8 generation container created by
script/package-linux-gpu-stack-docker.sh and needs no arguments beyond the ones
that wrapper passes.

Options:
  --binary PATH                release Navop binary; its DT_NEEDED closure
                               decides which desktop client libraries ship
  --target TRIPLE              x86_64-unknown-linux-gnu or aarch64-unknown-linux-gnu
  --output DIR                 directory to receive navop-gpu-stack-linux-<arch>.tar.gz
  --source-distribution TEXT   recorded in the manifest
  --glibc-baseline VERSION     maximum GLIBC symbol version allowed (default 2.28)
  --help
USAGE
}

fail() {
  printf 'Error: %s\n' "$*" >&2
  exit 1
}

say() {
  printf '\n==> %s\n' "$*"
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
      --source-distribution)
        shift
        [ "$#" -gt 0 ] || fail "--source-distribution requires text"
        source_distribution="$1"
        ;;
      --glibc-baseline)
        shift
        [ "$#" -gt 0 ] || fail "--glibc-baseline requires a version"
        glibc_baseline="$1"
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
  [ -f "$binary" ] || fail "release binary does not exist: $binary"
}

# Maps a Rust target triple onto the RPM architecture dnf has to resolve for and
# the asset label the archive carries.
target_architecture() {
  case "$target" in
    x86_64-unknown-linux-gnu)
      rpm_arch="x86_64"
      asset_label="x64"
      ;;
    aarch64-unknown-linux-gnu)
      rpm_arch="aarch64"
      asset_label="arm64"
      ;;
    *)
      fail "unsupported target: $target"
      ;;
  esac
}

install_tooling() {
  dnf install -y --setopt=install_weak_deps=False \
    cpio findutils dnf-plugins-core python39 binutils rpm >/dev/null
}

kept_rpm_directory=""

# Scratch space is a global so the EXIT trap can still find it: an EXIT trap runs
# after main returns, by which point main's locals are gone.
work_directory=""

cleanup() {
  if [ -n "$work_directory" ]; then
    rm -rf "$work_directory"
  fi
}

download_rpms() {
  local destination="$1"
  local empty_root="$2"
  local release_version
  release_version="$(rpm -E %{rhel})"
  [ -n "$release_version" ] || fail "cannot determine the distribution release version"

  mkdir -p "$destination" "$empty_root"
  dnf download --resolve --setopt=install_weak_deps=False \
    --releasever="$release_version" --nogpgcheck --forcearch="$rpm_arch" \
    --installroot "$empty_root" --destdir "$destination" \
    "${GPU_STACK_PACKAGES[@]}"
}

# An x86_64 repository also offers i686 multilib builds, and dnf happily resolves
# them. Their files land under /usr/lib (the 64-bit ones under /usr/lib64), but
# feeding them to the packager would only add ambiguity, so keep the target
# architecture and the arch-independent packages.
filter_rpms_to_architecture() {
  local directory="$1"
  local keep="$directory/$rpm_arch"
  local dropped=0
  local rpm package_arch

  mkdir -p "$keep"
  for rpm in "$directory"/*.rpm; do
    [ -e "$rpm" ] || continue
    package_arch="$(rpm -qp --qf '%{ARCH}' "$rpm" 2>/dev/null || echo unknown)"
    case "$package_arch" in
      "$rpm_arch" | noarch)
        mv -- "$rpm" "$keep/"
        ;;
      *)
        rm -f -- "$rpm"
        dropped=$((dropped + 1))
        ;;
    esac
  done
  kept_rpm_directory="$keep"
  printf 'kept %s RPMs for %s, dropped %s foreign-architecture RPMs\n' \
    "$(find "$keep" -name '*.rpm' | wc -l)" "$rpm_arch" "$dropped"
}

unpack_rpms() {
  local rpm_directory="$1"
  local destination="$2"
  local rpm

  mkdir -p "$destination"
  for rpm in "$rpm_directory"/*.rpm; do
    [ -e "$rpm" ] || continue
    # cpio writes relative to its working directory. --no-absolute-filenames
    # keeps a malformed archive from writing outside the tree; the RPM payloads
    # only ever carry standard FHS paths.
    (cd "$destination" && rpm2cpio "$rpm" | cpio -idm --no-absolute-filenames --quiet)
  done
}

default_source_distribution() {
  local pretty_name=""
  if [ -r /etc/os-release ]; then
    # shellcheck disable=SC1091
    pretty_name="$(. /etc/os-release && printf '%s' "${PRETTY_NAME:-}")"
  fi
  [ -n "$pretty_name" ] || pretty_name="RHEL 8 generation distribution"
  printf '%s (AppStream %s)' "$pretty_name" "$rpm_arch"
}

main() {
  parse_arguments "$@"
  target_architecture
  if [ -z "$source_distribution" ]; then
    source_distribution="$(default_source_distribution)"
  fi
  # dnf and rpm come from the base image; everything else is installed below.
  for command in dnf rpm; do
    command -v "$command" >/dev/null 2>&1 || fail "required command is missing: $command"
  done

  work_directory="$(mktemp -d)"
  trap cleanup EXIT
  local work="$work_directory"

  say "Installing packaging tooling"
  install_tooling
  for command in rpm2cpio cpio python3; do
    command -v "$command" >/dev/null 2>&1 || fail "required command is missing: $command"
  done

  say "Resolving the ${rpm_arch} dependency tree"
  download_rpms "$work/rpms" "$work/empty-root"
  filter_rpms_to_architecture "$work/rpms"

  say "Unpacking RPMs into a file tree"
  unpack_rpms "$kept_rpm_directory" "$work/root"

  say "Collecting the dependency stack"
  local repository_root
  repository_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
  mkdir -p "$output"
  python3 "$repository_root/script/package-linux-gpu-stack.py" \
    --binary "$binary" \
    --library-root "$work/root" \
    --rpm-directory "$kept_rpm_directory" \
    --target "$target" \
    --output "$work/gpu-stack" \
    --installer-source "$repository_root/script/linux-gpu-stack-install.sh" \
    --glibc-baseline "$glibc_baseline" \
    --source-distribution "$source_distribution"

  local archive="navop-gpu-stack-linux-${asset_label}.tar.gz"
  say "Archiving $archive"
  tar \
    --sort=name \
    --mtime='UTC 1970-01-01' \
    --owner=0 \
    --group=0 \
    --numeric-owner \
    -czf "$output/$archive" \
    -C "$work/gpu-stack" .
  ls -l "$output/$archive"
}

main "$@"
