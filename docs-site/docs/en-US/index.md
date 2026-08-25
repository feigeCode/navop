# Navop Usage Guide

Navop is the dev and ops workspace for the AI era, bringing databases, Redis, MongoDB, SSH, SFTP, terminals, remote desktops, Notes, AI, and team sync into one native workspace.

## Current release: v0.11.0

[Download Navop v0.11.0](https://github.com/feigeCode/navop/releases/tag/v0.11.0)

- Added a configurable "Connection Sorting" option under **Settings → General → Connection Display**, defaulting to natural name order (IP addresses compared by value, case-insensitive) with "Most Recently Used" also available; the Home connection list, Redis/MongoDB workspace tabs, and the persistent sidebar connection tree all honor the setting.
- SSH now offers opt-in compatibility for legacy servers that only support DSA host keys, SHA-1 key exchange/MAC, or 1024-bit DH group negotiation.
- Duplicated tabs are automatically numbered (reusing freed numbers) and tab widths adapt to content so long titles are not truncated.

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
