# Settings, fonts, and shortcuts

Settings controls appearance, databases, terminals, remote editing, Notes, LLMs, MCP, logs, and updates. Test changes outside a critical session. A font or shortcut issue may not damage data, but it can change what you see or which action fires.

## Set language and appearance

Choose language, start page, light/dark theme, or system-following behavior. Appearance settings also include window opacity, theme palettes, a custom accent color, and theme importing. Application and terminal themes provide consistent colors for inputs, popovers, scrollbars, selections, and terminals. Import themes only from trusted sources and verify text, focus, and selection contrast in the workspaces you use.

![Unified themes and appearance settings](/images/theme.png)

Configure application, SQL, table-preview, and terminal fonts separately. Custom font files support `.ttf`, `.otf`, `.ttc`, and `.otc`; verify licensing and integrity.

Missing glyphs trigger fallback or unreadable output. Prefer monospaced fonts for SQL and terminals and recheck charts and highlighting after theme changes.

## Tune database and table behavior

Database preferences include connection opening, SQL auto-save, maximum rows, table row height, and large-text handling. Maximum rows limits preview, not server permission; increasing it costs memory and network. Auto-save does not commit transactions.

Display density never changes stored field values. Use the appropriate editor for long or binary content and inspect SQL Preview before submit.

## Configure terminal and remote editors

Choose a local Terminal profile or trusted custom program. Configure completion, selection copy, right/middle paste, multiline confirmation, dangerous-command warnings, and terminal fonts. Understand bracketed paste and clipboard risk before reducing safeguards.

Select a built-in or extension-provided remote editor, its executable, and automatic upload behavior. Keep remote-conflict checks enabled.

## Configure Notes, LLM, and MCP

Choose the Notes folder without assuming old files move automatically. Manage LLM Provider keys, URLs, models, and defaults; confirm no active Agent depends on a provider before deletion.

MCP settings control Temporary/Persistent mode, permission profile, and Tool Exposure. Reconnect clients and inspect the new tool list after any change.

## Customize shortcuts

Shortcuts are grouped for global actions, terminal, database, table, remote editing, Redis, and Notes. Search existing bindings before changing them, and account for operating-system and input-method interception. Avoid easy single-key bindings for destructive actions.

If a shortcut fails, check focus, platform, input method, and conflicts before resetting or choosing another combination.

## Inspect logs and updates

Settings exposes the log path, automatic checks, and manual update. Logs may include paths, hosts, and error context and must be redacted before sharing. Save work, finish transactions and transfers, and verify extension compatibility around every update.
