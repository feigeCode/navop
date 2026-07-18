# Quick start

Navop puts databases, Redis, MongoDB, SSH, SFTP, terminals, remote desktops, serial devices, Notes, AI, and encrypted team sync in one native workspace. Start with one real task, then add the rest of the product as you need it.

## Finish a first task in five minutes

Download the correct build from [GitHub Releases](https://github.com/feigeCode/navop/releases), launch Navop, and choose New Connection. Create a database or SSH connection, enter the address and authentication details, and run Test Connection before saving.

- For a database, expand a schema, open the SQL editor, and run a read-only query.
- For a server, open an SSH terminal, run low-risk commands such as `pwd`, then browse the same host with SFTP.
- For local work, open Terminal or Notes, or use the operating system to open `.db`, `.duckdb`, and `.md` files in Navop.

## Understand the product map

The home screen organizes saved resources into workspaces. Database views cover SQL, table data, table design, ER diagrams, import/export, and schema or data comparison. Redis and MongoDB have dedicated data browsers and diagnostics. SSH workspaces combine terminal splits, command history, quick commands, SFTP, remote editing, and server monitoring. RDP, VNC, Serial, and Port Forwarding are also created from New Connection.

Notes manages local Markdown, rich-text, and whiteboard material. The AI workbench supports configurable providers and Ask, Plan, or Auto modes. Public MCP exposes selected live Navop tools to Codex, Claude, or automation clients. Extensions add database drivers, ACP Agents, importers, remote desktop providers, language packs, and document renderers.

## Follow a safe learning path

Create personal test connections first and learn read-only workflows. Then configure database and terminal preferences, try transfer or remote editing, and finally enable AI. Sign in and use personal sync or a team only when cross-device or shared access is required. Separate production, staging, and local resources into clearly named workspaces.

Before writes, deletes, DDL, bulk synchronization, file overwrites, or terminal broadcast input, recheck the environment and selection. Prefer test data, backups, transactions, and least-privilege accounts.

## Protect credentials and automation

The master key encrypts saved database passwords, SSH keys, and similar secrets locally. Synced services store ciphertext produced on the device; a lost master key cannot be recovered by the website. Team keys are also protected on member devices.

AI and Public MCP tool calls can perform real operations. Begin with conservative permissions, expose only the tool groups required for the current task, and read every approval. Never publish credentials, master keys, private keys, discovery tokens, raw production logs, or unredacted screenshots.
