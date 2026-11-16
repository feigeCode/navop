# SSH port forwarding

Port forwarding uses a saved SSH/SFTP connection as a network path. Navop supports local forwarding, remote forwarding (SSH `-R`), and dynamic SOCKS forwarding, with state and activity logs. A tunnel does not grant application credentials or replace end-to-end TLS.

## Select the base SSH connection

Choose an SSH/SFTP connection that can already log in. It supplies jump-host address, authentication, proxy, and host verification. Stop dependent forwards before editing or deleting it.

Use restricted jump-host accounts and explicit network policies. A running tunnel only proves reachability; the database or application still requires its own authentication.

## Configure local forwarding

Local forwarding maps a local listen address and port to a destination visible from the SSH server. Binding `127.0.0.1` normally limits access to the current computer; binding all interfaces may expose the tunnel to the LAN and needs deliberate firewall control.

The destination is resolved from the jump host's network perspective. If the listen port is occupied, choose another and update the downstream client. Connect that client to the local address while retaining the target service's credentials and certificates.

## Configure remote forwarding (SSH `-R`)

Remote forwarding listens on the SSH server's bind host and port, then carries each incoming connection back through SSH to a target reachable from the computer running Navop. For example, bind `127.0.0.1:18080` with target `127.0.0.1:3000` means that connecting to `127.0.0.1:18080` on the SSH server reaches `127.0.0.1:3000` on the Navop computer. The target is resolved from the Navop computer's network perspective.

The SSH server must permit TCP forwarding and may still reject a remote-listen request. Set the bind port to `0` to let the server allocate an available port; the running tab displays the actual address. Stopping the forward or losing the SSH session removes the server-side listener.

Whether a non-loopback bind is allowed depends on server policy such as `GatewayPorts`. Binding `0.0.0.0`, `::`, or another externally reachable address can expose a service on the Navop computer to the server's LAN or the public Internet. Do this only with deliberate service authentication, firewall rules, and access controls; prefer `127.0.0.1` by default.

## Configure dynamic SOCKS

Dynamic forwarding creates a local SOCKS proxy whose clients select each destination. Configure supporting browsers or tools explicitly. DNS routing depends on the client; verify that sensitive names are not resolved outside the proxy.

Never expose the SOCKS listener to an untrusted network or treat it as anonymous access. Traffic remains subject to jump-host logs and target policy.

## Read state and logs

Use status, retries, and activity logs to distinguish SSH disconnects, listen-port conflicts, rejected remote-listen requests, destination refusal, DNS failure, and access denial. Persistent failure needs a root-cause fix, not endless retrying.

Logs can contain hosts, ports, and timestamps and should be redacted before sharing.

## Stop without surprising clients

Stopping a forward immediately interrupts every database transaction, transfer, browser request, or other client using it. Close downstream work first. App exit, sleep, network changes, and SSH reconnects can also invalidate the tunnel; downstream applications normally need to reconnect and recheck transaction state.
