#!/usr/bin/env bash
set -euo pipefail

MOONSHOT_MODEL="${MOONSHOT_MODEL:-kimi-k2.5}"
MINIMAX_MODEL="${MINIMAX_MODEL:-MiniMax-M2.5}"

if [[ -z "${MOONSHOT_API_KEY:-}" ]]; then
  echo "[ERROR] MOONSHOT_API_KEY 未设置" >&2
  exit 1
fi

if [[ -z "${MINIMAX_API_KEY:-}" ]]; then
  echo "[ERROR] MINIMAX_API_KEY 未设置" >&2
  exit 1
fi

print_result() {
  local vendor="$1"
  local body="$2"

  if command -v jq >/dev/null 2>&1; then
    echo "$body" | jq -r --arg vendor "$vendor" '
      {
        vendor: $vendor,
        id: (.id // ""),
        model: (.model // ""),
        content: (.choices[0].message.content // ""),
        content_len: ((.choices[0].message.content // "") | length),
        usage: (.usage // null)
      }'
  else
    echo "[$vendor] raw response: $body"
  fi
}

echo "=== [1/2] Moonshot direct ==="
MOONSHOT_BODY=$(curl -sS 'https://api.moonshot.ai/v1/chat/completions' \
  -H "Authorization: Bearer ${MOONSHOT_API_KEY}" \
  -H 'Content-Type: application/json' \
  -d "{\"model\":\"${MOONSHOT_MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"请用一句话介绍你自己\"}],\"stream\":false}")
print_result "moonshot-direct" "$MOONSHOT_BODY"

echo ""
echo "=== [2/2] MiniMax direct ==="
MINIMAX_BODY=$(curl -sS 'https://api.minimax.io/v1/chat/completions' \
  -H "Authorization: Bearer ${MINIMAX_API_KEY}" \
  -H 'Content-Type: application/json' \
  -d "{\"model\":\"${MINIMAX_MODEL}\",\"messages\":[{\"role\":\"user\",\"content\":\"请用一句话介绍你自己\"}],\"stream\":false}")
print_result "minimax-direct" "$MINIMAX_BODY"

echo ""
echo "[DONE] 直连验证完成"
