# Navop Usage Guide

Navop is the dev and ops workspace for the AI era, bringing databases, Redis, MongoDB, SSH, SFTP, terminals, remote desktops, Notes, AI, and team sync into one native workspace.

## Current release: v0.19.5

Download the latest stable release from the [official Download Center](https://navop.dev/en-US/extensions).

- Personal sync gains a WebDAV backend next to "Folder" and "Git": enter the server URL, user name and password and it works, with the password sealed to disk rather than stored in the clear. The protocol uses only GET / PUT / DELETE plus Basic auth and never probes with PROPFIND, so it fits Jianguoyun, Synology, Nextcloud and self-hosted servers; a collection is created with MKCOL before the first write, and a 409 is reported as "directory unavailable" instead of a bogus version conflict. The settings page shows only the items of the selected backend, the password field can be revealed, and since WebDAV has no local directory to watch, a 60-second scan drives syncing.
- SSH agent forwarding: a key credential can be marked "Forward this key through ssh-agent", and a connection has its own "SSH Agent Forwarding" (ForwardAgent) switch. With the local ssh-agent forwarded to the remote host, that host (a jump host, for example) can authenticate onward to deeper hosts with your local key; it applies to newly opened terminal sessions.
- The table structure designer gains "Refresh table structure", which reloads the latest columns, indexes and table info; with unsaved edits it warns first and confirms with "Discard and refresh".
- The terminal's paste confirmation dialog now offers "Open settings" and "Don't ask again": multi-line paste, high-risk command and large paste prompts all link to the matching setting. Large paste is a hard threshold and offers no "don't ask again".
- External drivers now time out per call category, with a per-connection override: user operations (query, exec, cursor, import/export) default to 30 minutes while metadata and structure browsing keep 30 seconds, and advanced connection settings gain a "Request timeout (seconds)" field (empty follows the category defaults, 0 means unlimited), so large queries and imports on slow databases are no longer cut off by a single 30-second cap.
- The commit and rollback buttons of the SQL editor's manual transaction and "Commit changes" in the table data page show a loading indicator while the write is in flight, making it clear which step is still running; the other button is only disabled and does not spin.
- Fixed the crash when typing Chinese into the WHERE / ORDER BY filters (#326) and the filter-bar inputs sitting one line taller than before the migration (40px → 20px); fixed an AI chat turn being marked as failed after cancelling a reply, and stale column comments after editing them on dialects such as Dameng.
- Fixed every in-app quit ending in SIGABRT with a core file on Linux (Arch + Hyprland / Wayland, #336); the macOS Touch Bar crash on window close is closed out by upstream zed#65186, so the "hide on close" experiment switch is gone and both macOS architectures, Windows and Linux are back to destroying on close.

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
