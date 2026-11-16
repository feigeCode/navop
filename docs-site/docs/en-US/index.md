# Navop Usage Guide

Navop is a desktop workspace **built natively with Rust and GPUI** — no WebView, no Electron. The interface, terminals, tables, and charts are GPU-rendered. It brings databases, servers, remote desktops, and AI agents into one window, covering connection management, SQL development, file transfer, and remote troubleshooting.

## Why Navop

- **Native performance.** GPUI paints the interface directly, so many live connections, split terminals, and large result sets stay responsive instead of waiting on browser layout.
- **Rust end to end.** Database drivers, SSH/SFTP, terminal emulation, the AI runtime, and the UI share one Rust codebase.
- **One window for the work.** Keep SQL workspaces, terminals, file managers, remote desktops, and the AI sidebar side by side instead of switching tools.
- **Extensible.** Database drivers, remote desktop providers, document renderers, and resource workbenches install as WASM extensions without restarting the app.
- **Consistent across platforms.** macOS (arm64/x64), Windows (x64/win32), and Linux (x64/arm64) share the same interface and capabilities.

## What Navop supports

- **Databases:** MySQL, PostgreSQL, MariaDB, SQLite, DuckDB, SQL Server, Oracle, ClickHouse, TDengine, plus extension drivers for Dameng, KingbaseES, GBase, OceanBase, openGauss, IoTDB, Oscar, and more.
- **Data tools:** object browsing, SQL editor with execution plans, import/export, schema and data comparison, ER diagrams, batch table editing.
- **Redis / MongoDB / MQTT:** key and value management, collection and document queries, subscribe and publish.
- **SSH and terminals:** local and remote terminals, native split panes, broadcast input, quick commands, shell integration, session recording/replay, and searchable logs.
- **File transfer:** SFTP, FTP, and FTPS with drag-and-drop, remote file editing, ZMODEM, and server-to-server copy.
- **Remote access:** local/remote/SOCKS port forwarding, X11, RDP, VNC, serial, Telnet, host-key management, and SecureCRT import.
- **AI and automation:** SQL generation and explanation, result analysis, charts, terminal assistance, ACP agents (Codex / Claude Code / OpenCode), and Agent Hub.
- **Collaboration and extensibility:** Notes, teams and shared connections, encrypted sync, extension marketplace, Public MCP, Navop CLI, and Agent Skill.

## Install and versions

Get the latest builds from the [official Download Center](https://navop.dev/en-US/extensions), [GitHub Releases](https://github.com/feigeCode/navop/releases), or [CNB](https://cnb.cool/navop-dev/navop/-/releases). Packages follow `navop-<version>-<label>.<ext>` and cover macOS, Windows, and Linux; every release includes checksums.

The same source also builds two smaller editions: **Navop Shell** (SSH, terminals, and remote desktop) and **Navop Data** (databases and data browsing). They share Navop's native shell, AI, sync, and extension system and are intended for narrower workflows; most users can start with Navop itself.

See [Installation and updates](./guide/install-update) for the package matrix, platform differences, and upgrade notes.

## Start here

- [Quick start: finish a first connection and task](./guide/quick-start)
- [Installation and updates: packages, platforms, and data directories](./guide/install-update)
- [Home, workspaces, and connections](./guide/workspace-connections)

## Find a workflow

- Database connections, SQL, import/export, and schema tools: [Database connections](./guide/database-connections)
- SQL editor, transactions, and query results: [SQL editor](./guide/sql-editor)
- SSH, SFTP, port forwarding, and Agent Hub: [SSH and terminals](./guide/ssh-terminal)
- FTP and FTPS remote files: [FTP and remote files](./guide/ftp-remote-files)
- Remote desktop, serial, and server monitoring: [Remote access](./guide/remote-access)
- Native resource workbench such as Docker: [Resource workbench](./guide/resource-workbench)
- Notes Markdown preview and source editing: [Notes](./guide/notes)
- AI Workbench, Navop Skill, and Public MCP: [AI and MCP](./guide/ai-workbench)
- Team sync and security: [Team sync](./guide/teams-sync-security)
- Settings and troubleshooting: [Settings and troubleshooting](./guide/settings-shortcuts)

This guide is updated as Navop evolves. Before working with production databases, remote servers, write SQL, file overwrites, or bulk sync, confirm the current environment and scope.
