#!/bin/sh
set -eu

ssh-keygen -A

# 2222: keyboard-interactive TOTP + password, used by OTP/elevation tests.
/usr/sbin/sshd -e

# 2223: password only, used by generic live integration tests.
exec /usr/sbin/sshd -D -e -f /etc/ssh/sshd_config.password
