# Navop Usage Guide

Navop is the dev and ops workspace for the AI era, bringing databases, Redis, MongoDB, SSH, SFTP, terminals, remote desktops, Notes, AI, and team sync into one native workspace.

## Current release: v0.15.1

Download the latest stable release from the [official Download Center](https://navop.dev/en-US/extensions).

- Terminal gains "highlight identical text on selection": after selecting text, matching text in the visible area is highlighted with a subtle background, working in both SSH and local terminals; toggleable in the terminal sidebar settings (on by default).
- Connection list width is now persisted: resizing the sidebar connection tree is saved automatically and restored on next launch; the docked sidebar shares the main window background, and the floating mode adopts a card-style look (rounded corners + shadow).
- The "check for updates automatically" toggle and "Check for Updates" button move from general settings to the About page, alongside the version information.
- Fixed sidebar and command bar icon buttons rendering black instead of following the terminal/Agent custom theme colors.
- Fixed SFTP restoring the old mtime when overwriting remote files, which made mtime-based change detection (rsync deploys, web/app caches, incremental builds) treat the overwritten file as unchanged.

## Start here

- [Quick start](./guide/quick-start)
- [Installation and updates](./guide/install-update)
- [Home, workspaces, and connections](./guide/workspace-connections)

## Find a workflow

- [Database connections, SQL, import/export, and schema tools](./guide/database-connections)
- [SQL editor, transactions, and query results](./guide/sql-editor)
- [SSH, SFTP, port forwarding, and Agent Hub](./guide/ssh-terminal)
- [Remote desktop, serial, and server monitoring](./guide/remote-access)
- [Notes Markdown preview and source editing](./guide/notes)
- [AI Workbench, Navop Skill, and Public MCP](./guide/ai-workbench)
- [Team sync and security](./guide/teams-sync-security)
- [Settings and troubleshooting](./guide/settings-shortcuts)
