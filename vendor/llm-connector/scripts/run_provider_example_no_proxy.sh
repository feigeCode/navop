#!/usr/bin/env bash
set -euo pipefail

EXAMPLE_NAME="${EXAMPLE_NAME:-}"
PROVIDER="${PROVIDER:-}"
REGION="${REGION:-}"
MODEL="${MODEL:-}"

if [[ -z "$EXAMPLE_NAME" ]]; then
  echo "[ERROR] EXAMPLE_NAME 未设置，例如: zhipu / zhipu_tools / moonshot / moonshot_tools / moonshot_thinking" >&2
  exit 1
fi

if [[ -z "$PROVIDER" ]]; then
  echo "[ERROR] PROVIDER 未设置，例如: zhipu / moonshot" >&2
  exit 1
fi

case "$PROVIDER" in
  zhipu)
    API_KEY_VAR="ZHIPU_API_KEY"
    REGION_VAR="ZHIPU_REGION"
    MODEL_VAR="ZHIPU_MODEL"
    ;;
  moonshot)
    API_KEY_VAR="MOONSHOT_API_KEY"
    REGION_VAR="MOONSHOT_REGION"
    MODEL_VAR="MOONSHOT_MODEL"
    ;;
  *)
    echo "[ERROR] 当前脚本仅支持 PROVIDER=zhipu 或 PROVIDER=moonshot，当前: $PROVIDER" >&2
    exit 1
    ;;
esac

API_KEY="${!API_KEY_VAR:-}"
if [[ -z "$API_KEY" ]]; then
  echo "[ERROR] ${API_KEY_VAR} 未设置" >&2
  exit 1
fi

if [[ -n "$REGION" ]]; then
  export "$REGION_VAR=$REGION"
fi

if [[ -n "$MODEL" ]]; then
  export "$MODEL_VAR=$MODEL"
fi

EFFECTIVE_REGION="${!REGION_VAR:-}"
EFFECTIVE_MODEL="${!MODEL_VAR:-}"

echo "=== No-proxy provider example runner ==="
echo "provider: $PROVIDER"
echo "example: $EXAMPLE_NAME"
echo "region: ${EFFECTIVE_REGION:-<default>}"
echo "model: ${EFFECTIVE_MODEL:-<default>}"
echo "proxy: disabled"
echo ""

env \
  -u HTTP_PROXY \
  -u HTTPS_PROXY \
  -u ALL_PROXY \
  -u http_proxy \
  -u https_proxy \
  -u all_proxy \
  -u ZHIPU_PROXY \
  -u MOONSHOT_PROXY \
  cargo run --example "$EXAMPLE_NAME"
