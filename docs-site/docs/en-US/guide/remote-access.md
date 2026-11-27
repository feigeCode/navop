# Remote desktop, serial, and server monitoring

Navop connects to RDP and VNC through provider extensions, offers serial terminals, and provides a monitoring page for SSH hosts. Platform, provider, and remote-system compatibility determine available behavior.

## Install RDP or VNC providers

Install a compatible remote desktop provider from Extensions, then create an RDP or VNC connection. Enter host, port, user, and password; RDP may also require a Domain. A provider version mismatch should be fixed by updating or reloading the extension rather than randomly changing credentials.

On Windows, opening a dedicated window from a connection's context menu launches the system Remote Desktop client (`mstsc.exe`) in fullscreen, avoiding white-screen, focus, and exit issues that could occur with the embedded native window; hostnames, IPv4/IPv6 addresses, and custom ports are passed to the system client correctly. Embedded native RDP opens in a tab by default, while dedicated VNC windows and remote desktop windows on non-Windows platforms continue to use Navop's built-in window.

Configure SOCKS/HTTP proxy and read-only mode when supported. Read-only reduces input mistakes but does not replace remote account permissions. Verify certificates or host identity and use least-privilege accounts.

## Diagnose display and input

Resolution, clipboard, keyboard layout, and input depend on provider and platform. RDP/VNC providers now stream incremental frame updates, which usually keeps active sessions smoother than full-frame refreshes. For lag, still reduce display load and check latency. Black screens and authentication errors may come from session policy, Domain, certificates, or ports.

Save work in remote applications before closing. Disconnecting does not necessarily log the remote account out; lock or sign out according to security policy.

## Connect serial devices

Select a device and set baud rate, data bits, stop bits, parity, and flow control. Every value must match the hardware. Ensure no other program owns the port and use the correct electrical adapter.

Review device documentation before sending reset, erase, or firmware commands. macOS has limitations around virtual serial pty devices; use a supported real device node and check system privacy permissions.

## Monitor SSH hosts

The Monitor page collects CPU, memory, disk partitions, and network state from saved SSH hosts. Monitoring is off by default and enabled per connection: use the "Resource monitoring" switch on a host card, or start monitoring from that host's entry. Only enabled hosts keep a long-lived sampling session, and the same switch controls whether the home connection list shows resource usage for that host.

The page refreshes every 5, 15, or 30 seconds, supports searching hosts and reconnecting/refreshing all monitors, and shows resource cards with trends. GPU and NPU views appear when the host exposes them. If a host requires interactive credentials such as keyboard-interactive or TOTP, open a terminal once or save the credentials first; the monitor page can also answer these prompts automatically when reconnecting.

Sampling runs inside the current SSH session and writes nothing to the remote host. Older releases deployed a `~/.onetcli-monitor` helper; that directory is now cleaned up on a best-effort basis and is no longer required. The terminal sidebar's "Server monitoring" panel uses the same collector for the current session.

Sampling consumes some resources, and process or network data may contain sensitive operational information. Trends supplement rather than replace production alerting and audit systems.

## Update and remove components

Update providers when versions do not match. Removing a provider makes its saved connections temporarily unavailable but does not delete the remote system. To stop monitoring a host, turn off its resource-monitoring switch, then verify that no stale helper or background session remains under organizational policy.

Proxy, read-only mode, and monitoring never bypass server permissions. Test network, extension state, account rights, and remote services separately before combining the diagnosis.
