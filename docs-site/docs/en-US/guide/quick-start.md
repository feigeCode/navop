# Quick start

Navop is a desktop workspace built natively with Rust and GPUI — no browser runtime. You do not need to configure everything at once. Start with one real connection and one read-only task, then grow from there.

![Navop workspace overview](/images/app.png)

The home screen in the screenshot is where connections and workspaces begin. Database, SSH/SFTP, Redis, MongoDB, terminal, and remote desktop connections are all created there; the matching editor, browser, or terminal tab appears after you open a connection.

## Your first task in five minutes

Download Navop from the [download center](https://navop.dev/en-US/extensions). Installation is covered in [Install and update](./install-update).

1. Start Navop, choose **New connection** on the home screen, and create a database or SSH connection. Enter the host, port, and credentials, then run **Test connection**.
2. Open the connection once the test passes. A database connection opens the database workspace, where you can expand schemas and tables and run a read-only query in the SQL editor. An SSH connection opens a terminal workspace; run low-risk commands such as `pwd` and `whoami`, then browse the remote directory over SFTP.
3. For local work, open Terminal or Notes directly. Navop also registers file associations for `.db`, `.duckdb`, `.md`, and `.cast`.

## Know the workspaces

- **Home:** manage database, SSH, Redis, MongoDB, and remote desktop connections in grid or tree view, with recent connections and batch actions.
- **Database workspace:** SQL editor, table data, table design, ER diagrams, import/export, schema comparison, and data transfer.
- **Monitor:** CPU, memory, disk, and network state for SSH hosts with resource monitoring enabled, toggled per connection, refreshing every 5, 15, or 30 seconds.
- **SSH workspace:** terminals, tmux panes, splits, quick commands, broadcast input, SFTP, remote file editing, and terminal notifications.
- **Remote access:** port forwarding, RDP/VNC remote desktops, serial, and Telnet.
- **Notes and AI:** local Markdown preview and editing; AI can generate, explain, and troubleshoot in SQL, result, and terminal context.
- **Extensions and collaboration:** install database drivers, ACP agents, remote desktop providers, and language packs; manage team spaces and shared connections.

Public MCP lets external clients such as Codex and Claude call the running Navop. It uses a dynamic loopback port and a user-only discovery token, and the exposed tools follow Tool Exposure, permission level, and approval policy.

## Recommended onboarding order

Create a personal connection and get comfortable with read-only operations first, then configure terminal and database preferences. After that, try import/export, remote editing, or AI. Enable personal sync or team spaces only when you actually need multi-device or multi-person collaboration; teams, members, and team keys are all managed on the app's **Teams** page. Team Owner, Admin, and Member roles have different permissions, so an administrator should initialize the team key first.

Use separate workspaces for production, staging, and local environments, and give connections clear names. Before write SQL, bulk key deletion, table truncation, overwriting remote files, synchronizing differences, or broadcast input, re-check the current environment, selection scope, and transaction strategy.

## Security and data principles

Passwords, SSH private keys, and other secrets are encrypted locally with a master key. With sync enabled, the cloud stores ciphertext produced locally; a lost master key cannot be recovered by the vendor. Team keys are also stored encrypted, and only on member devices. Do not paste production credentials, master keys, full logs, or unredacted screenshots into public channels.

Configuration, connection ciphertext, and keychain entries stay on the local machine and can be backed up or migrated with the app. Quit the app before copying so you do not capture a half-written file.

Public MCP and AI tools can perform real operations. Start with a conservative permission level, expose only the tools the current task needs, and read each approval request. Before any delete, overwrite, truncate, DDL, bulk sync, or stop-forwarding action, confirm the impact and verify in an environment you can roll back.

## First reproducible task

On first launch, the app seeds a `Demo Orders (SQLite)` demo database in the local data directory. It exists only to help you learn the interface, never connects to the network, and contains no real business data.

1. Find `Demo Orders (SQLite)` on the home screen and open its database workspace.
2. Expand the schema and tables, then open `customers` or `products` to inspect data.
3. Create a SQL query and run this read-only statement:

```sql
SELECT status, COUNT(*) AS order_count
FROM orders
GROUP BY status
ORDER BY order_count DESC;
```

4. Go back to table data, page through a table, and open the large-text or JSON editor for one record.
5. Close the query and data tabs, keep the connection card, and continue with the database connection or SQL editor chapters.

If the demo database does not appear, make sure the app completed its first-launch initialization, or download the current stable build from the [download center](https://navop.dev/en-US/extensions) and relaunch. Do not treat the demo database as a production backup or sync source.

## Completion checklist

- [ ] You can find a connection on the home screen and distinguish its name, type, and environment.
- [ ] You can open database objects or an SSH terminal.
- [ ] You can run a read-only SQL query, or run `pwd` and `whoami` in a terminal.
- [ ] You know that closing a tab does not delete a connection, and deleting a connection does not drop the remote database.
- [ ] You know to re-check the target before write SQL, key deletion, file overwrites, or broadcast input.

## Next steps

- Installing, choosing a package, or upgrading: read [Install and update](./install-update).
- Connecting your own database: read [Database connections](./database-connections).
- Writing and running queries: read [SQL editor and transactions](./sql-editor).
- Connecting to servers: read [SSH and terminal workspace](./ssh-terminal) and [SFTP and remote file editing](./sftp-remote-files).
- Importing existing connections: read [Home, workspaces, and connections](./workspace-connections).
