# Quick start

Navop now ships as three apps built on one native core: the full **Navop**, remote operations app **Navop Shell**, and data toolkit **Navop Data**. You do not need to configure everything at once. Pick the app that matches your work from the table below, finish one real task, and grow from there.

![Navop workspace overview](/images/app.png)

The home screen in the screenshot is where connections and workspaces begin. Database, SSH/SFTP, Redis, MongoDB, terminal, and remote desktop connections are all created there; the matching editor, browser, or terminal tab appears after you open a connection.

## Which app do I need?

| What you mostly do | Recommended app |
| --- | --- |
| Manage both databases and servers, or need ER diagrams, data transfer, monitoring, and file associations | **Navop** (full edition) |
| Mostly SSH into servers, move files over SFTP, forward ports, use RDP/VNC remote desktops, or work with serial/Telnet | **Navop Shell** (remote operations) |
| Mostly write SQL against databases, browse Redis/MongoDB, inspect ER diagrams, and move data | **Navop Data** (data toolkit) |

All three share AI, Notes, team sync, the extension marketplace, themes, keyboard shortcuts, tab restore, tray integration, single-instance behavior, and automatic updates. They are released together at the same version and can be installed side by side, but each keeps its own data directory and keychain. If you are unsure, start with the full Navop; it has every capability, and you can still add the focused apps later.

## Your first task in five minutes

Download the installer for your app from the [download center](https://navop.dev/en-US/extensions). Installation is covered in [Install and update](./install-update).

### With Navop (full edition)

1. Start Navop, choose **New connection** on the home screen, and create a database or SSH connection. Enter the host, port, and credentials, then run **Test connection**.
2. Open the connection once the test passes. A database connection opens the database workspace, where you can expand schemas and tables and run a read-only query in the SQL editor. An SSH connection opens a terminal workspace; run low-risk commands such as `pwd` and `whoami`, then browse the remote directory over SFTP.
3. For local work, open Terminal or Notes directly. The full Navop also registers file associations for `.db`, `.duckdb`, `.md`, and `.cast`.

### With Navop Shell (remote operations)

1. Start Navop Shell and create an SSH/SFTP, terminal, RDP/VNC, serial/Telnet, or port-forwarding connection from **New connection**.
2. Test the connection first, then open the SSH terminal and run commands such as `pwd` and `whoami`, or browse the remote directory over SFTP.
3. For graphical sessions, install a remote desktop provider from the extension marketplace, then create an RDP or VNC connection. For tunnels, start port forwarding from a saved SSH connection.

### With Navop Data (data toolkit)

1. Start Navop Data and create a database, Redis, or MongoDB connection. Fill in the host, port, and credentials, then test it.
2. A database connection opens the SQL workspace, where you can expand schemas and tables and run a read-only query. Redis and MongoDB open their own browsers and diagnostics.
3. To map table relationships or move data, open an ER diagram or data transfer. Both are available only inside the data workspace.

## Knowing the product map

**Full Navop** covers databases, Redis, MongoDB, SSH/SFTP, local and remote terminals, port forwarding, remote desktops, serial/Telnet, the SQL editor, ER diagrams, data transfer, monitoring, Notes, the AI workbench, team sync, the extension marketplace, and Public MCP. The home screen manages connections and workspaces. Inside a database connection you get SQL, table data, table design, ER diagrams, import/export, and comparison/synchronization; the SSH workspace includes terminals, split panes, quick commands, SFTP, remote editing, and server monitoring.

**Navop Shell** keeps the remote-operations workspaces: SSH/SFTP, local and remote terminals, port forwarding, RDP/VNC remote desktops, and serial/Telnet. It shares AI, Notes, teams, and the extension system.

**Navop Data** keeps the data workspaces: the SQL workspace, Redis, MongoDB, ER diagrams, and data transfer. It shares AI, Notes, teams, and the extension system.

All three share themes, keyboard shortcuts, tab restore, tray integration, single-instance behavior, automatic updates, and Public MCP. Public MCP discovery and authorization are per app: `navop`, `navop-shell`, and `navop-data` each expose only the tools available in that app, so do not assume one app's manifest can drive another.

Notes provides local Markdown preview and source editing. The AI workbench is the same across all three apps: configure a model provider and work against real resources in Ask, Plan, or Auto mode. For terminal-capable AI agents such as Codex, you can install the Navop Skill and operate resources through `navop` on demand, which avoids carrying every MCP tool definition in context on each turn. The CLI still requires Public MCP to be enabled for the target app and follows tool exposure, permission level, and approval policy. The extension marketplace is where you install database drivers, ACP agents, connection importers, remote desktop providers, and language packs.

## Recommended onboarding order

Create a personal connection and get comfortable with read-only operations first, then configure terminal and database preferences. After that, try import/export, remote editing, or AI. Enable personal sync or team spaces only when you actually need multi-device or multi-person collaboration; teams, members, and team keys are all managed on the app's **Teams** page. Team Owner, Admin, and Member roles have different permissions, so an administrator should initialize the team key first.

Use separate workspaces for production, staging, and local environments, and give connections clear names. Before write SQL, bulk key deletion, table truncation, overwriting remote files, synchronizing differences, or broadcast input, re-check the current environment, selection scope, and transaction strategy.

## Security and data principles

Passwords, SSH private keys, and other secrets are encrypted locally with a master key. With sync enabled, the cloud stores ciphertext produced locally; a lost master key cannot be recovered by the vendor. Team keys are also stored encrypted, and only on member devices. Do not paste production credentials, master keys, full logs, or unredacted screenshots into public channels.

Each app maintains its own data directory and keychain. Even when installed side by side, they never share the same connection ciphertext. Back up, migrate, and restore each app separately rather than mixing their directories.

Public MCP and AI tools can perform real operations. Start with a conservative permission level, expose only the tools the current task needs, and read each approval request. Before any delete, overwrite, truncate, DDL, bulk sync, or stop-forwarding action, confirm the impact and verify in an environment you can roll back.

## First reproducible task

On first launch, the app seeds a `Demo Orders (SQLite)` demo database in the local data directory. It exists only to help you learn the interface, never connects to the network, and contains no real business data. It can be opened directly in the database workspace of Navop and Navop Data.

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

- [ ] You know whether you are using Navop, Navop Shell, or Navop Data, and how it differs from the other two.
- [ ] You can find a connection on the home screen and distinguish its name, type, and environment.
- [ ] You can open database objects or an SSH terminal.
- [ ] You can run a read-only SQL query, or run `pwd` and `whoami` in a terminal.
- [ ] You know that closing a tab does not delete a connection, and deleting a connection does not drop the remote database.
- [ ] You know to re-check the target before write SQL, key deletion, file overwrites, or broadcast input.

## Next steps

- Choosing or switching apps: read [Install and update](./install-update).
- Connecting your own database: read [Database connections](./database-connections) (Navop / Navop Data).
- Writing and running queries: read [SQL editor and transactions](./sql-editor) (Navop / Navop Data).
- Connecting to servers: read [SSH and terminal workspace](./ssh-terminal) and [SFTP and remote file editing](./sftp-remote-files) (Navop / Navop Shell).
- Importing existing connections: read [Home, workspaces, and connections](./workspace-connections).
