# Troubleshooting and issue reports

Change one variable at a time and classify the failure as network, authentication, authorization, driver, extension, client state, or remote service. Record platform, Navop version, connection type, time, and a minimal reproduction. Do not hide configuration errors by switching immediately to an administrator account.

## Diagnose databases, Redis, and MongoDB

For SQL databases, check DNS, host, port, firewall, account, default database/schema, TLS, proxy, SSH tunnel, driver, and server version. Reduce SQL failures to one statement and resolve commit or rollback state. Check permissions and schema filters for missing objects.

For Redis, verify standalone/Sentinel/Cluster mode and node reachability. For MongoDB, verify connection string, authentication database, TLS, topology discovery, and BSON types. Never use FLUSH, collection deletion, or disabled certificate checks as a diagnostic shortcut.

## Diagnose SSH and files

Check fingerprint, user, password/key, key permissions, SSH Agent, MFA, proxy, and server logs. If SSH works but SFTP fails, inspect subsystem, directory rights, and disk. Compare remote size after interrupted transfers and preserve both versions during edit conflicts.

Terminal mojibake may involve remote locale, font, or encoding. For forwarding, distinguish local port conflict, SSH loss, destination refusal, and listen-address errors.

## Diagnose extensions and remote access

Confirm an extension is installed, enabled, platform-compatible, and version-compatible, then reload it and inspect its logs. Drivers, ACP Agents, and remote desktop providers can fail independently. Record both old and new versions after an upgrade regression.

RDP/VNC problems may involve provider, Domain, certificate, session policy, or network. Serial output requires matching device, baud, data bits, stop bits, parity, and flow control.

## Diagnose AI and Public MCP

For models, check Provider state, API key, base URL, model, network, and quota. Missing tools can come from mode, resources, or permissions. ACP authorization does not remove Public MCP approval.

For Public MCP, check running Navop, server mode, Node.js 20+, `npx`, client configuration, discovery permissions, Tool Exposure, and live schema. Reconnect after endpoint restart and never guess resource IDs.

## Diagnose sync and update

Check account, network, master key, team key, role, client version, and unresolved conflicts. The website cannot recover a lost master key. During rotation, avoid concurrent bulk edits.

For updates, verify disk, package architecture, system security policy, and extension compatibility. Back up local data before rollback.

## Submit a redacted report

Use the log path in Settings and include only the smallest time-relevant excerpt, plus expected and actual result and repeatable steps. Substitute fake hosts, schemas, and sample SQL where possible.

Remove passwords, API keys, tokens, master keys, SSH and certificate private keys, full connection strings, internal addresses, personal data, and business results. Redact tab names, history, and paths in screenshots. After destructive behavior, stop retrying and preserve backups, logs, and server audit evidence.
