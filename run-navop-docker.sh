#!/usr/bin/env bash
#
# Run Navop on a headless Linux host through Xvfb + x11vnc + Docker.
#
# The Docker runtime supplies a modern Mesa/lavapipe Vulkan stack for hosts
# whose system Mesa is too old for GPUI/wgpu. The application binary itself is
# still built on the host and mounted from this repository.

set -euo pipefail

SCRIPT_DIR="$(cd -P -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
CONTAINER_NAME="${NAVOP_CONTAINER_NAME:-navop-gui}"
IMAGE_NAME="${NAVOP_DOCKER_IMAGE:-navop-runtime:navop-local}"
DISPLAY_NAME="${NAVOP_DISPLAY:-:99}"
VNC_PORT="${NAVOP_VNC_PORT:-5900}"
VNC_PASSFILE="${NAVOP_VNC_PASSFILE:-/root/.vncpasswd}"
BINARY="${NAVOP_BINARY:-${SCRIPT_DIR}/target/debug/navop}"
RUNTIME_HOME="${NAVOP_RUNTIME_HOME:-$(cd -P -- "${SCRIPT_DIR}/.." && pwd)/.navop-runtime-home}"
LOG_PATH="/root/navop.log"

ACTION="start"
REBUILD_IMAGE=0
BUILD_APP=0

info() { printf '[navop] %s\n' "$*"; }
warn() { printf '[navop] warning: %s\n' "$*" >&2; }
fail() {
  printf '[navop] error: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: ./run-navop-docker.sh [command]

Commands:
  (default)        Start Navop, or report the existing running instance
  --restart        Restart the Navop process inside the container
  --recreate       Recreate the Docker container from the runtime image
  --status         Show display, VNC, container, and application status
  --logs           Follow the Navop application log
  --stop           Stop Navop and the runtime container
  --rebuild-image  Rebuild the Ubuntu/Mesa runtime image before starting
  --build          Run `cargo build -p navop` before starting
  -h, --help       Show this help

Environment overrides:
  NAVOP_CONTAINER_NAME  Docker container name (default: navop-gui)
  NAVOP_DOCKER_IMAGE    Runtime image name (default: navop-runtime:navop-local)
  NAVOP_DISPLAY         X display (default: :99)
  NAVOP_VNC_PORT        VNC port (default: 5900)
  NAVOP_VNC_PASSFILE    x11vnc password file (default: /root/.vncpasswd)
  NAVOP_BINARY          Navop binary (default: <repo>/target/debug/navop)
  NAVOP_RUNTIME_HOME    Persistent container HOME (default: <repo>/../.navop-runtime-home)
  NAVOP_LANG            Application locale (default: zh_CN.UTF-8)
EOF
}

parse_arguments() {
  while [ "$#" -gt 0 ]; do
    case "$1" in
      --restart) ACTION="restart" ;;
      --recreate) ACTION="recreate" ;;
      --status) ACTION="status" ;;
      --logs) ACTION="logs" ;;
      --stop) ACTION="stop" ;;
      --rebuild-image) REBUILD_IMAGE=1 ;;
      --build) BUILD_APP=1 ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        fail "unknown argument: $1 (try --help)"
        ;;
    esac
    shift
  done
}

require_command() {
  command -v "$1" >/dev/null 2>&1 || fail "required command not found: $1"
}

display_is_ready() {
  DISPLAY="$DISPLAY_NAME" xdpyinfo >/dev/null 2>&1
}

ensure_display() {
  if display_is_ready; then
    info "X display ${DISPLAY_NAME} is ready"
    return
  fi

  require_command Xvfb
  require_command xdpyinfo
  mkdir -p "$RUNTIME_HOME"

  info "starting Xvfb on ${DISPLAY_NAME}"
  nohup Xvfb "$DISPLAY_NAME" \
    -screen 0 1440x900x24 \
    +extension GLX \
    +render \
    -noreset \
    >"${RUNTIME_HOME}/xvfb.log" 2>&1 &
  printf '%s\n' "$!" >"${RUNTIME_HOME}/xvfb.pid"

  for _ in $(seq 1 50); do
    if display_is_ready; then
      info "X display ${DISPLAY_NAME} started"
      return
    fi
    sleep 0.1
  done

  tail -n 40 "${RUNTIME_HOME}/xvfb.log" >&2 || true
  fail "failed to start Xvfb on ${DISPLAY_NAME}"
}

vnc_port_is_listening() {
  ss -ltn 2>/dev/null |
    awk -v port=":${VNC_PORT}" '$4 ~ port "$" { found = 1 } END { exit found ? 0 : 1 }'
}

ensure_vnc() {
  if vnc_port_is_listening; then
    info "VNC port ${VNC_PORT} is already listening"
    return
  fi

  require_command x11vnc
  mkdir -p "$RUNTIME_HOME"

  local arguments=(
    -display "$DISPLAY_NAME"
    -forever
    -shared
    -rfbport "$VNC_PORT"
    -o "${RUNTIME_HOME}/x11vnc.log"
  )

  if [ -f "$VNC_PASSFILE" ]; then
    arguments+=(-rfbauth "$VNC_PASSFILE")
  else
    warn "VNC password file not found: ${VNC_PASSFILE}; listening on localhost without a password"
    arguments+=(-localhost -nopw)
  fi

  info "starting x11vnc on port ${VNC_PORT}"
  nohup x11vnc "${arguments[@]}" >/dev/null 2>&1 &
  printf '%s\n' "$!" >"${RUNTIME_HOME}/x11vnc.pid"

  for _ in $(seq 1 50); do
    if vnc_port_is_listening; then
      info "x11vnc started"
      return
    fi
    sleep 0.1
  done

  tail -n 40 "${RUNTIME_HOME}/x11vnc.log" >&2 || true
  fail "failed to start x11vnc on port ${VNC_PORT}"
}

build_runtime_image() {
  require_command docker

  local build_dir
  build_dir="$(mktemp -d "${TMPDIR:-/tmp}/navop-runtime-image.XXXXXX")"

  cat >"${build_dir}/Dockerfile" <<'EOF'
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        dbus-x11 \
        fonts-noto-cjk \
        fonts-noto-core \
        libegl1 \
        libfontconfig1 \
        libgl1 \
        libglx-mesa0 \
        libvulkan1 \
        libx11-xcb1 \
        libxcb-xkb1 \
        libxcb1 \
        libxcursor1 \
        libxi6 \
        libxinerama1 \
        libxkbcommon-x11-0 \
        libxrandr2 \
        libxrender1 \
        libxtst6 \
        mesa-vulkan-drivers \
        procps \
    && rm -rf /var/lib/apt/lists/*

CMD ["sleep", "infinity"]
EOF

  info "building Docker runtime image ${IMAGE_NAME}"
  if ! docker build --tag "$IMAGE_NAME" "$build_dir"; then
    rm -rf -- "$build_dir"
    fail "failed to build Docker runtime image ${IMAGE_NAME}"
  fi
  rm -rf -- "$build_dir"
}

ensure_runtime_image() {
  if [ "$REBUILD_IMAGE" -eq 1 ] ||
    ! docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
    build_runtime_image
  fi
}

container_exists() {
  docker container inspect "$CONTAINER_NAME" >/dev/null 2>&1
}

container_running() {
  [ "$(docker inspect --format '{{.State.Running}}' "$CONTAINER_NAME" 2>/dev/null)" = "true" ]
}

app_running() {
  container_running &&
    docker exec "$CONTAINER_NAME" pgrep -x navop >/dev/null 2>&1
}

create_container() {
  ensure_runtime_image
  mkdir -p "$RUNTIME_HOME"

  local docker_arguments=(
    -d
    --name "$CONTAINER_NAME"
    --network host
    -v "${SCRIPT_DIR}:${SCRIPT_DIR}:ro"
    -v "/tmp/.X11-unix:/tmp/.X11-unix"
    -v "${RUNTIME_HOME}:/root"
    -e "DISPLAY=${DISPLAY_NAME}"
    -e "XDG_RUNTIME_DIR=/root/.xdg-runtime"
    -e "LIBGL_ALWAYS_SOFTWARE=1"
    -e "WGPU_BACKEND=vulkan"
    -e "LANG=${NAVOP_LANG:-zh_CN.UTF-8}"
  )

  local cargo_checkouts="${CARGO_HOME:-${HOME}/.cargo}/git/checkouts"
  if [ -d "$cargo_checkouts" ]; then
    # Debug builds of rust-embed read assets from compile-time absolute paths.
    docker_arguments+=(-v "${cargo_checkouts}:/root/.cargo/git/checkouts:ro")
  fi

  info "creating Docker container ${CONTAINER_NAME}"
  docker run "${docker_arguments[@]}" "$IMAGE_NAME" sleep infinity
}

ensure_container() {
  if [ "$ACTION" = "recreate" ] && container_exists; then
    info "removing existing container ${CONTAINER_NAME}"
    docker rm -f "$CONTAINER_NAME" >/dev/null
  fi

  if container_exists; then
    if ! container_running; then
      info "starting existing container ${CONTAINER_NAME}"
      docker start "$CONTAINER_NAME" >/dev/null
    fi
  else
    create_container
  fi
}

stop_app() {
  if app_running; then
    info "stopping Navop process"
    docker exec "$CONTAINER_NAME" pkill -TERM -x navop >/dev/null 2>&1 || true
    for _ in $(seq 1 50); do
      if ! app_running; then
        return
      fi
      sleep 0.1
    done
    docker exec "$CONTAINER_NAME" pkill -KILL -x navop >/dev/null 2>&1 || true
  fi
}

launch_app() {
  local quoted_script_dir quoted_binary
  quoted_script_dir="$(printf '%q' "$SCRIPT_DIR")"
  quoted_binary="$(printf '%q' "$BINARY")"

  docker exec "$CONTAINER_NAME" mkdir -p /root/.xdg-runtime
  docker exec "$CONTAINER_NAME" chmod 700 /root/.xdg-runtime
  : >"${RUNTIME_HOME}/navop.log"

  info "starting Navop"
  docker exec -d "$CONTAINER_NAME" bash -lc \
    "cd ${quoted_script_dir} && exec dbus-run-session -- env DISPLAY=${DISPLAY_NAME} XDG_RUNTIME_DIR=/root/.xdg-runtime LIBGL_ALWAYS_SOFTWARE=1 WGPU_BACKEND=vulkan LANG=${NAVOP_LANG:-zh_CN.UTF-8} ${quoted_binary} >> ${LOG_PATH} 2>&1"

  for _ in $(seq 1 100); do
    if app_running; then
      info "Navop process is running"
      return
    fi
    sleep 0.1
  done

  tail -n 80 "${RUNTIME_HOME}/navop.log" >&2 || true
  fail "Navop exited during startup"
}

print_connection_info() {
  local container_image
  container_image="$(
    docker inspect --format '{{.Config.Image}}' "$CONTAINER_NAME" 2>/dev/null ||
      printf '%s' "$IMAGE_NAME"
  )"

  cat <<EOF

Navop is running.

VNC (recommended through an SSH tunnel):
  ssh -N -L ${VNC_PORT}:127.0.0.1:${VNC_PORT} root@<this-host>
  Connect VNC Viewer to 127.0.0.1:${VNC_PORT}

Container: ${CONTAINER_NAME}
Image:     ${container_image}
Log:       ${RUNTIME_HOME}/navop.log
Data:      ${RUNTIME_HOME}/.navop-dev
EOF
}

show_status() {
  if display_is_ready; then
    printf 'display:   %s ready\n' "$DISPLAY_NAME"
  else
    printf 'display:   %s not ready\n' "$DISPLAY_NAME"
  fi

  if vnc_port_is_listening; then
    printf 'vnc:       port %s listening\n' "$VNC_PORT"
  else
    printf 'vnc:       port %s not listening\n' "$VNC_PORT"
  fi

  if container_exists; then
    printf 'container: %s (%s, image %s)\n' \
      "$CONTAINER_NAME" \
      "$(docker inspect --format '{{.State.Status}}' "$CONTAINER_NAME")" \
      "$(docker inspect --format '{{.Config.Image}}' "$CONTAINER_NAME")"
  else
    printf 'container: %s missing\n' "$CONTAINER_NAME"
  fi

  if app_running; then
    printf 'app:       running\n'
    docker exec "$CONTAINER_NAME" ps -o pid=,stat=,etime=,cmd= -C navop
  else
    printf 'app:       not running\n'
  fi
}

main() {
  parse_arguments "$@"
  require_command docker

  case "$ACTION" in
    status)
      show_status
      exit 0
      ;;
    logs)
      container_exists || fail "container does not exist: ${CONTAINER_NAME}"
      docker exec "$CONTAINER_NAME" tail -f "$LOG_PATH"
      exit 0
      ;;
    stop)
      if container_exists; then
        stop_app
        info "stopping container ${CONTAINER_NAME}"
        docker stop "$CONTAINER_NAME" >/dev/null
      fi
      exit 0
      ;;
  esac

  ensure_display
  ensure_vnc

  if [ "$BUILD_APP" -eq 1 ]; then
    require_command cargo
    info "building Navop"
    (cd "$SCRIPT_DIR" && cargo build -p navop)
  fi

  [ -x "$BINARY" ] || fail "Navop binary is missing or not executable: ${BINARY} (run with --build)"

  ensure_container

  case "$ACTION" in
    restart)
      stop_app
      launch_app
      print_connection_info
      ;;
    recreate)
      launch_app
      print_connection_info
      ;;
    start)
      if app_running; then
        info "Navop is already running; use --restart to relaunch it"
      else
        launch_app
      fi
      print_connection_info
      ;;
  esac
}

main "$@"
