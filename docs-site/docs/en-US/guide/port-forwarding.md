# SSH port forwarding

Port forwarding uses a saved SSH/SFTP connection as a network path. Navop supports local forwarding and dynamic SOCKS forwarding, with state and activity logs. A tunnel does not grant application credentials or replace end-to-end TLS.

## Select the base SSH connection

Choose an SSH/SFTP connection that can already log in. It supplies jump-host address, authentication, proxy, and host verification. Stop dependent forwards before editing or deleting it.

Use restricted jump-host accounts and explicit network policies. A running tunnel only proves reachability; the database or application still requires its own authentication.

## Configure local forwarding

Local forwarding maps a local listen address and port to a destination visible from the SSH server. Binding `127.0.0.1` normally limits access to the current computer; binding all interfaces may expose the tunnel to the LAN and needs deliberate firewall control.

The destination is resolved from the jump host's network perspective. If the listen port is occupied, choose another and update the downstream client. Connect that client to the local address while retaining the target service's credentials and certificates.

## Configure dynamic SOCKS

Dynamic forwarding creates a local SOCKS proxy whose clients select each destination. Configure supporting browsers or tools explicitly. DNS routing depends on the client; verify that sensitive names are not resolved outside the proxy.

Never expose the SOCKS listener to an untrusted network or treat it as anonymous access. Traffic remains subject to jump-host logs and target policy.

## Read state and logs

Use status, connection counts, retries, and activity logs to distinguish SSH disconnects, local port conflicts, destination refusal, DNS failure, and access denial. Persistent failure needs a root-cause fix, not endless retrying.

Logs can contain hosts, ports, and timestamps and should be redacted before sharing.

## Stop without surprising clients

Stopping a forward immediately interrupts every database transaction, transfer, browser request, or other client using it. Close downstream work first. App exit, sleep, network changes, and SSH reconnects can also invalidate the tunnel; downstream applications normally need to reconnect and recheck transaction state.
