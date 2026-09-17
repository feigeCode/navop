# Navop Usage Guide

Navop is the dev and ops workspace for the AI era, bringing databases, Redis, MongoDB, SSH, SFTP, terminals, remote desktops, Notes, AI, and team sync into one native workspace.

## Current release: v0.18.0

Download the latest stable release from the [official Download Center](https://navop.dev/en-US/extensions).

- New standalone FTP / FTPS connection type, plus SFTP / FTP / FTPS switching on the remote-file panel of an SSH connection; SSH connections can default a double-click to the terminal or the dual-pane file view.
- New native resource workbench: extensions declare collections, tables, and operations that the host renders natively. The first release is a Docker workbench with engine overview, container and image management, logs, processes, filesystem changes, and exec terminals.
- The local terminal launcher detects and lists WSL distributions for one-click launch.
- TDengine and MQTT are now extension-provided; the built-in implementations are removed, existing data migrates, and the extensions install on demand.
- Redis uses the embedded redis-rs client with bounded memory for large keys; fixed key-tree search misses, keyboard input buffered by the shell-integration handshake, and MSTSC cursor jitter.
- Closing the main window minimizes it to the system tray and keeps the app running; resource workbench trees support static children, decoupling expansion from navigation.
- AI streaming requests now use an idle read timeout, so long tasks are no longer cut off by a fixed total timeout, and the idle timeout is configurable.

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
