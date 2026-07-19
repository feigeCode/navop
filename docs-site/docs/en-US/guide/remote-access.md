# Remote desktop, serial, and server monitoring

Navop connects to RDP and VNC through provider extensions, offers Serial terminals, and can deploy an SSH monitoring helper. Platform, provider, and remote-system compatibility determine available behavior.

## Install RDP or VNC providers

Install a compatible remote desktop provider from Extensions, then create an RDP or VNC connection. Enter host, port, user, and password; RDP may also require a Domain. A provider version mismatch should be fixed by updating or reloading the extension rather than randomly changing credentials.

Configure SOCKS/HTTP proxy and read-only mode when supported. Read-only reduces input mistakes but does not replace remote account permissions. Verify certificates or host identity and use least-privilege accounts.

## Diagnose display and input

Resolution, clipboard, keyboard layout, and input depend on provider and platform. RDP/VNC providers now stream incremental frame updates, which usually keeps active sessions smoother than full-frame refreshes. For lag, still reduce display load and check latency. Black screens and authentication errors may come from session policy, Domain, certificates, or ports.

Save work in remote applications before closing. Disconnecting does not necessarily log the remote account out; lock or sign out according to security policy.

## Connect serial devices

Select a device and set baud rate, data bits, stop bits, parity, and flow control. Every value must match the hardware. Ensure no other program owns the port and use the correct electrical adapter.

Review device documentation before sending reset, erase, or firmware commands. macOS has limitations around virtual serial pty devices; use a supported real device node and check system privacy permissions.

## Enable SSH server monitoring

Monitoring shows CPU, memory, disk, network, and process trends. After explicit confirmation, Navop deploys `~/.onetcli-monitor` to the remote account. Managed servers may require administrator approval and appropriate directory/execute permissions.

Sampling consumes some resources, and process or network views may contain sensitive operational data. Trends supplement rather than replace production alerting and audit systems.

## Update and remove components

Update providers or the helper when versions do not match. Removing a provider makes its saved connections temporarily unavailable but does not delete the remote system. When disabling monitoring, verify whether the remote helper is still running and clean it up under organizational policy.

Proxy, read-only mode, and monitoring never bypass server permissions. Test network, extension state, account rights, and remote services separately before combining the diagnosis.
