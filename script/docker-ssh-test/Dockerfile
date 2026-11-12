# Navop 端到端测试用 SSH 服务器：密码 + TOTP(PAM google-authenticator) + su/sudo 提权
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        openssh-server libpam-google-authenticator sudo \
    && rm -rf /var/lib/apt/lists/*

# 登录用户与 root 用同一个密码，便于用一条连接分别验证 sudo 和 su root。
RUN useradd -m -s /bin/bash tester \
    && echo 'tester:tester-pass' | chpasswd \
    && echo 'root:tester-pass' | chpasswd \
    && echo 'tester ALL=(ALL) ALL' > /etc/sudoers.d/tester \
    && chmod 0440 /etc/sudoers.d/tester

# TOTP 种子 JBSWY3DPEHPK3PXP（纯 base32，无 otpauth:// 前缀，也可用 URI 形式测试）。
RUN printf 'JBSWY3DPEHPK3PXP\n" TOTP_AUTH\n' > /home/tester/.google_authenticator \
    && chown tester:tester /home/tester/.google_authenticator \
    && chmod 600 /home/tester/.google_authenticator

# 只走 keyboard-interactive：PAM 先要验证码，再走 common-auth 要密码。
RUN printf '%s\n' \
        'Port 2222' \
        'ListenAddress 0.0.0.0' \
        'PasswordAuthentication yes' \
        'KbdInteractiveAuthentication yes' \
        'UsePAM yes' \
        'PermitRootLogin yes' \
        'AuthenticationMethods keyboard-interactive' \
        'PrintMotd no' \
        > /etc/ssh/sshd_config.d/99-navop-test.conf \
    && sed -i 's|^@include common-auth|auth required pam_google_authenticator.so\n@include common-auth|' /etc/pam.d/sshd \
    && grep -n "pam_google_authenticator" /etc/pam.d/sshd

RUN mkdir -p /run/sshd

EXPOSE 2222
CMD ["/bin/sh", "-c", "ssh-keygen -A && exec /usr/sbin/sshd -D -e -p 2222"]
