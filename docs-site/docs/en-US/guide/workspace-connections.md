# Home, workspaces, and connections

The home screen is the entry point for every saved resource. Workspaces group connections, while card and list views, search, and ordering make large sets easier to navigate. Organizing a connection does not change the remote service itself.

## Organize the home screen

Search by connection name and switch between card and list layouts. Create workspaces for production, staging, local development, customers, or projects; edit their names, reorder them, and filter the visible connections. Use unmistakable names and visual cues for production resources.

Before deleting a workspace, review what happens to its saved entries. A workspace is an organizational layer, but removing or moving entries can still affect local navigation and sync state.

## Create, edit, copy, and delete

New Connection includes databases, Redis, MongoDB, SSH/SFTP, local Terminal, RDP, VNC, Serial, Port Forwarding, and additional extension-provided types. Fill in authentication and advanced settings, test the connection, then save it.

Existing connections can be edited, copied, or deleted. A copy is useful for a related environment, but change its name, destination, and credentials immediately. Close tabs, terminals, transfers, and forwards that use a connection before editing or deleting it.

## Search and maintain connections

Search changes only what is displayed. Closing a database or terminal tab does not delete the saved connection. Periodically remove obsolete entries, normalize names, and confirm workspace membership. Never put a password or token in a connection name; hostnames and internal addresses may also require redaction in screenshots.

Deleting a saved entry normally does not delete a remote database or server. Operations performed inside the opened workspace, however, can change real remote data.

## Import from other applications

Connection import requires an importer extension. It can scan supported local applications or read an exported file, then show a preview before anything is saved. Database and SSH records are labeled to indicate whether passwords were imported, missing, unsupported, or blocked by permissions.

Select only reviewed records. Duplicate detection skips obvious repeats, but you must still compare names, hosts, ports, databases, and authentication methods. Some sources cannot export secrets, so re-enter them after import. Port-forward import, saving, and duplicate handling may not be supported and should be rebuilt manually.

## Handle sync consequences

With personal or team sync enabled, deleting or editing a connection can produce a cross-device change or conflict. Compare timestamps and fields before choosing the local or remote version. Do not assume one side is automatically newer or safer.

Keep the distinction between connection metadata and remote resources clear. Syncing or deleting an entry changes access configuration; SQL, key deletion, file overwrite, and other in-workspace actions affect the actual target.
