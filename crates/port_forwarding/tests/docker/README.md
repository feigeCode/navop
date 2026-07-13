# Port Forwarding Docker E2E

该 fixture 启动一个允许 TCP forwarding 的 OpenSSH 服务和一个 nginx 目标服务。

```bash
cd crates/port_forwarding/tests/docker
docker compose up -d --build --wait
cd ../../../..
ONETCLI_DOCKER_E2E=1 cargo test -p port_forwarding --test docker_e2e -- --ignored --nocapture
docker compose -f crates/port_forwarding/tests/docker/docker-compose.yml down
```

默认测试参数：

- SSH：`127.0.0.1:2222`
- 用户：`onetcli`
- 密码：`onetcli-pass`
- 目标：`onetcli-pf-target:80`

E2E 同时验证 Local forwarding、Dynamic SOCKS forwarding、正常停止和监听端口释放。
