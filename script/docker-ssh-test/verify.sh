#!/usr/bin/env bash
# 快速验证测试服务器本身（不经过 Navop）：TOTP 认证 + sudo / su 提权。
set -euo pipefail
cd "$(dirname "$0")"

ADDR="${NAVOP_SSH_E2E_ADDR:-127.0.0.1:2222}"
HOST="${ADDR%:*}"
PORT="${ADDR##*:}"
USER_NAME="${NAVOP_SSH_E2E_USER:-tester}"
PASSWORD="${NAVOP_SSH_E2E_PASSWORD:-tester-pass}"
SECRET="${NAVOP_SSH_E2E_OTP:-JBSWY3DPEHPK3PXP}"

CODE="$(python3 totp.py "$SECRET")"
echo "== 用验证码 ${CODE} 登录 ${USER_NAME}@${HOST}:${PORT}"

expect -c "
set timeout 20
log_user 0
spawn ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
  -o PreferredAuthentications=keyboard-interactive -o PubkeyAuthentication=no \
  -p ${PORT} ${USER_NAME}@${HOST}
expect {
  -re {Verification code:\s*$} { send \"${CODE}\r\"; exp_continue }
  -re {Password:\s*$}        { send \"${PASSWORD}\r\"; exp_continue }
  -re {\\\$ $}                { send \"id -un\r\" }
}
expect -re {${USER_NAME}}
puts \">>> 登录成功\"
send \"sudo -n true; echo sudo-nopasswd=\\\$?\r\"
expect -re {sudo-nopasswd=[01]}
puts \">>> sudo 存在密码提示（预期 1）\"
send \"exit\r\"
expect eof
" 2>&1 | sed -e 's/^/    /'

echo "== 提示：Navop 里建主机 ${HOST}:${PORT}，用户 ${USER_NAME}，密码 ${PASSWORD}，"
echo "   高级选项里一次性验证码密钥填 ${SECRET}，即可全自动登录；su/sudo 密码填 ${PASSWORD}。"
