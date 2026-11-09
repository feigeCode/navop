# Navop Usage Guide

Navop now ships as three apps on one native core: the full **Navop**, **Navop Shell** for remote operations, and **Navop Data** for data work. They share the same version number, can be installed side by side, and share the AI, Notes, team sync, extension market, and theme foundation. What differs is the scope of work each app covers.

## The three apps

| App | Who it is for | What it covers |
| --- | --- | --- |
| **Navop** (full edition) | People who manage both databases and servers and want one app for everything | Databases (PostgreSQL, MySQL, MariaDB, SQLite, SQL Server, Oracle, ClickHouse, Dameng, Kingbase, OceanBase, OpenGauss, and more), Redis, MongoDB, SSH/SFTP, local and remote terminals, port forwarding, RDP/VNC remote desktop, serial/Telnet, SQL editor, ER diagrams, data transfer, monitoring, Notes, AI workbench, team sync, extension market, Public MCP |
| **Navop Shell** (remote operations) | People who mostly sign in to servers, move files, and open remote desktops | SSH/SFTP, local and remote terminals, port forwarding, RDP/VNC remote desktop, serial/Telnet; shares the AI, Notes, team, and extension foundation |
| **Navop Data** (data tools) | People who mostly connect to databases, write SQL, and browse Redis/MongoDB | SQL workspace, Redis, MongoDB, ER diagrams, data transfer; shares the AI, Notes, team, and extension foundation |

## Shared foundation

All three apps run on the same native core and share:

- the AI workbench, Notes, team sync, and personal sync;
- the extension market and the installed extension system;
- themes, keyboard shortcuts, tab restore, tray, single instance, auto-update, and Public MCP.

They are released under the same version number and can be installed side by side. Each app keeps its own data directory and keychain, so installing more than one does not overwrite another app's configuration. Only the full Navop registers `.db`, `.duckdb`, `.md`, and `.cast` file associations; Scoop and Flatpak currently cover the full Navop only.

## Which app should I use?

- **You manage both databases and servers**, or need file associations, monitoring, and data transfer: choose Navop.
- **You do remote operations**, mostly SSH, SFTP, terminals, port forwarding, and remote desktops: choose Navop Shell.
- **You do data work**, mostly SQL, Redis, MongoDB, ER diagrams, and transfer: choose Navop Data.
- **Not sure**: start with Navop, the most complete of the three. You can still install a standalone app later; they coexist without conflict.

## Current release: v0.23.0

All three apps are currently at v0.23.0. Download builds from the [official Download Center](https://navop.dev/en-US/extensions), [GitHub Releases](https://github.com/feigeCode/navop/releases), or [CNB](https://cnb.cool/navop-dev/navop/-/releases). The main product-shape changes:

- Navop grew from a single full app into three apps on one native core, sharing the AI, Notes, team, extension, and update foundation.
- Package names are distinguished by the `navop-`, `navop-shell-`, and `navop-data-` prefixes; all three are released under the same version and can be installed side by side.
- Database and remote-operations capabilities are scoped per app; data directories, keychains, and in-app update channels are independent.
- Only the full Navop provides `.db`, `.duckdb`, `.md`, and `.cast` file associations, plus the Linux `.deb`, `.rpm`, AppImage, and graphics dependency package.

See [Installation and updates](./guide/install-update) for the package matrix and platform differences.

## Start here

- [Quick start: pick an app, then finish a first task](./guide/quick-start)
- [Installation and updates: packages, platforms, and side-by-side installs](./guide/install-update)
- [Home, workspaces, and connections](./guide/workspace-connections) (all three apps)

## Find a workflow

- Database connections, SQL, import/export, and schema tools (Navop / Navop Data): [Database connections](./guide/database-connections)
- SQL editor, transactions, and query results (Navop / Navop Data): [SQL editor](./guide/sql-editor)
- SSH, SFTP, port forwarding, and Agent Hub (Navop / Navop Shell): [SSH and terminals](./guide/ssh-terminal)
- FTP and FTPS remote files (Navop): [FTP and remote files](./guide/ftp-remote-files)
- Remote desktop, serial, and server monitoring (remote desktop and serial: Navop / Navop Shell; monitoring: Navop): [Remote access](./guide/remote-access)
- Native resource workbench such as Docker (Navop): [Resource workbench](./guide/resource-workbench)
- Notes Markdown preview and source editing (all three apps): [Notes](./guide/notes)
- AI Workbench, Navop Skill, and Public MCP (all three apps; Public MCP is discovered per app): [AI and MCP](./guide/ai-workbench)
- Team sync and security (all three apps): [Team sync](./guide/teams-sync-security)
- Settings and troubleshooting (all three apps): [Settings and troubleshooting](./guide/settings-shortcuts)

This guide is updated as the three apps evolve. Before working with production databases, remote servers, write SQL, file overwrites, or bulk sync, confirm the current environment and scope.
