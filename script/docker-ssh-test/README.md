# Docker 测试 SSH 服务器

用来端到端验证 Navop 的 SSH 认证/提权相关改动，覆盖：

- keyboard-interactive 的两步认证：一次性验证码（TOTP，PAM `google-authenticator`）+ 登录密码；
- 会话内 `sudo -i`、`su root` 的密码提示自动填入；
- 验证码密钥填错时认证失败、提权密码被拒后不再重复回填。

服务器配置：OpenSSH + `libpam-google-authenticator`，只允许 `keyboard-interactive`；
PAM 先要验证码、再要密码（与阿里云/堡垒机常见的键盘交互流程一致）。

| 项 | 值 |
| --- | --- |
| 地址 | `127.0.0.1:2222` |
| 用户 / 密码 | `tester` / `tester-pass` |
| root 密码（`su root` 用） | `tester-pass` |
| TOTP 密钥（base32） | `JBSWY3DPEHPK3PXP` |
| TOTP 的 otpauth URI | `otpauth://totp/Navop:tester?secret=JBSWY3DPEHPK3PXP&issuer=Navop` |

## 启动

```bash
cd script/docker-ssh-test
docker compose up -d --build
docker compose logs -f          # 看 sshd 认证日志
```

## 验证服务器本身

```bash
./verify.sh                     # expect + totp.py，检查 TOTP 登录与 sudo/su
python3 totp.py                 # 打印当前验证码
```

## 用 Navop 验证

1. 新建 SSH 主机：主机 `127.0.0.1`，端口 `2222`，用户 `tester`，密码 `tester-pass`。
2. 高级选项里填「一次性验证码密钥」`JBSWY3DPEHPK3PXP`（或上面的 otpauth URI）、
   打开「自动填入 su/sudo 密码」，提权密码留空表示复用登录密码。
3. 连接应完全不需要手工输入：日志里能看到
   `SSH keyboard-interactive prompts were answered locally with the configured OTP secret`。
4. 连上后试 `sudo -i`、`exit`、`su root`，应当直接进入 root，不弹密码。

## 自动化用例

```bash
# 协议层 + 会话提权的端到端用例（默认 #[ignore]）
NAVOP_SSH_E2E=1 cargo test -p terminal --lib docker_ssh -- --ignored --nocapture
```

可用环境变量覆盖：`NAVOP_SSH_E2E_ADDR`、`NAVOP_SSH_E2E_USER`、`NAVOP_SSH_E2E_PASSWORD`、
`NAVOP_SSH_E2E_OTP`。

## 清理

```bash
docker compose down
```
