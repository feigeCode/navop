#!/usr/bin/env bash
set -euo pipefail

TARGET_VENDOR="${TARGET_VENDOR:-moonshot}"   # moonshot | minimax
GATEWAY_BASE_URL="${GATEWAY_BASE_URL:-}"
GATEWAY_API_KEY="${GATEWAY_API_KEY:-}"
GATEWAY_MODEL="${GATEWAY_MODEL:-}"

# 可选：给网关透传上游 API key 的 header（如果你们网关要求）
UPSTREAM_API_KEY_HEADER="${UPSTREAM_API_KEY_HEADER:-}"
UPSTREAM_API_KEY="${UPSTREAM_API_KEY:-}"

if [[ -z "$GATEWAY_BASE_URL" ]]; then
  echo "[ERROR] GATEWAY_BASE_URL 未设置，例如: http://127.0.0.1:3000/v1" >&2
  exit 1
fi

if [[ -z "$GATEWAY_API_KEY" ]]; then
  echo "[ERROR] GATEWAY_API_KEY 未设置" >&2
  exit 1
fi

case "$TARGET_VENDOR" in
  moonshot)
    DEFAULT_MODEL="kimi-k2.5"
    ;;
  minimax)
    DEFAULT_MODEL="MiniMax-M2.5"
    ;;
  *)
    echo "[ERROR] TARGET_VENDOR 仅支持 moonshot 或 minimax，当前: $TARGET_VENDOR" >&2
    exit 1
    ;;
esac

MODEL="${GATEWAY_MODEL:-$DEFAULT_MODEL}"
URL="${GATEWAY_BASE_URL%/}/chat/completions"

REQ_BODY=$(cat <<JSON
{"model":"${MODEL}","messages":[{"role":"user","content":"请用一句话介绍你自己"}],"stream":false}
JSON
)

TMP_RESP=$(mktemp)
TMP_CODE=$(mktemp)

CURL_ARGS=(
  -sS
  -o "$TMP_RESP"
  -w "%{http_code}"
  "$URL"
  -H "Authorization: Bearer ${GATEWAY_API_KEY}"
  -H 'Content-Type: application/json'
  -d "$REQ_BODY"
)

if [[ -n "$UPSTREAM_API_KEY_HEADER" && -n "$UPSTREAM_API_KEY" ]]; then
  CURL_ARGS+=( -H "${UPSTREAM_API_KEY_HEADER}: ${UPSTREAM_API_KEY}" )
fi

HTTP_CODE=$(curl "${CURL_ARGS[@]}" || true)
BODY=$(cat "$TMP_RESP")

rm -f "$TMP_RESP" "$TMP_CODE"

echo "=== unigateway request ==="
echo "target_vendor: $TARGET_VENDOR"
echo "url: $URL"
echo "http_code: $HTTP_CODE"

echo ""
if command -v jq >/dev/null 2>&1; then
  echo "$BODY" | jq -r '
    {
      id: (.id // ""),
      model: (.model // ""),
      content: (.choices[0].message.content // ""),
      content_len: ((.choices[0].message.content // "") | length),
      usage: (.usage // null),
      raw_object: (.object // "")
    }'
else
  echo "raw_body: $BODY"
fi

echo ""
echo "[TIP] 若 id/model/content/usage 为空，请让 unigateway 打印上游原始 response 再对照映射链路。"
