# Navop Usage Guide

Navop is the dev and ops workspace for the AI era, bringing databases, Redis, MongoDB, SSH, SFTP, terminals, remote desktops, Notes, AI, and team sync into one native workspace.

## Current release: v0.19.1

Download the latest stable release from the [official Download Center](https://navop.dev/en-US/extensions).

- Fixed frequent Windows terminal stuttering (git bash, PowerShell freezing after actions until later): rows with highlight/search decorations are rebuilt incrementally, custom highlight rules rescan only changed rows per frame, and the synchronous directory reads behind local-terminal hover detection moved to a background thread.
- Table data find: press Cmd/Ctrl+F while browsing table data to open the find bar; matched cells get an outlined highlight, Cmd/Ctrl+G / Cmd/Ctrl+Shift+G jump between matches and scroll the hit column into view, and hits are re-scanned after paging/refresh. Field filtering for table data preview: a column-header dropdown selects visible fields, so wide tables show only the columns you care about.
- SQL editor object details reworked: hover popups are off by default (enable in settings; when on they require a 600ms dwell), replaced by a right-click "View object details" that opens a standalone dialog with selectable, copyable content. Right-click also gains "Copy DDL"; selecting a table name and right-clicking resolves it too.
- New shortcut Cmd/Ctrl+Shift+W closes the active tab, rebindable in settings; the "Workspace" field in connection forms is now consistently called "Group".
- Fixed SSH MFA logins answering the verification-code prompt with the saved password and failing auth; fixed legacy-device (Huawei VRP etc.) SSH connections dying with `` `mpint` encoding invalid ``.
- Database connection robustness: the pre-reuse ping has a 10-second cap, the disconnect path a 5-second cap, and MySQL/PostgreSQL connections enable 30s TCP keepalive, so connections dropped by NAT/firewalls during long idle periods no longer hang forever; manually-started transactions killed by a disconnect are finalized automatically.
- Fixed SQLite WITHOUT ROWID tables and views showing an empty preview page, the PostgreSQL sequence catalogue always being empty, and MCP client configs being written with a removed positional argument; the default HTTP client now follows system and environment-variable proxies.
- Packaging: Linux packages no longer link WebKitGTK 4.1; rendering dependencies ship as a separate gpu-stack archive installed on demand (the installer only adds libraries the host is missing, with --dry-run/--uninstall support). The HTML preview webview is off by default on all platforms while "Open in browser" and "Download HTML" keep working.

## Start here

- [Quick start](./guide/quick-start)
- [Installation and updates](./guide/install-update)
- [Home, workspaces, and connections](./guide/workspace-connections)

## Find a workflow

- [Database connections, SQL, import/export, and schema tools](./guide/database-connections)
- [SQL editor, transactions, and query results](./guide/sql-editor)
- [SSH, SFTP, port forwarding, and Agent Hub](./guide/ssh-terminal)
- [FTP and FTPS remote files](./guide/ftp-remote-files)
- [Remote desktop, serial, and server monitoring](./guide/remote-access)
- [Native resource workbench (Docker and more)](./guide/resource-workbench)
- [Notes Markdown preview and source editing](./guide/notes)
- [AI Workbench, Navop Skill, and Public MCP](./guide/ai-workbench)
- [Team sync and security](./guide/teams-sync-security)
- [Settings and troubleshooting](./guide/settings-shortcuts)
