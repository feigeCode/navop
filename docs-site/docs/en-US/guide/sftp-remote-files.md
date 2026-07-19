# SFTP and remote file editing

The SFTP manager provides navigation, favorites, transfer, permissions, extraction, and remote editing. Each side can switch between local storage and a searchable remote endpoint, and files or directories can be copied directly between two remote servers. Overwrites, deletion, permission changes, and automatic upload affect the server immediately.

## Navigate and coordinate with terminal

Browse folders, show hidden files when needed, and favorite common paths. Open a terminal in the current directory; with shell integration, the terminal path can synchronize back to the file manager. Access remains limited by the remote account.

Confirm the current path before creating or renaming. Renaming a live configuration or script can break services and links.

## Copy between remote endpoints

Use the endpoint switcher to search for the target SSH/SFTP server, then confirm the connection, user, and path shown on each side. Server copy transfers the selected files or directories through the remote connections without staging them in a local temporary directory. Refresh the destination and verify size, permissions, and any required checksum; both remote accounts and server policies still apply.

## Transfer files and folders

Upload selected files or directories, drag them from the desktop, or use clipboard input. Choose a local destination and check disk space before downloading. The transfer queue reports progress and errors; closing the app or connection can cancel active tasks.

After interruption, compare size or hash and inspect partial targets before retrying, especially for deployments, archives, and backups.

## Resolve conflicts and extract archives

For same-name conflicts, choose skip, keep both, merge, or overwrite. Directory merge continues through children; it does not intelligently merge file contents. Back up remote configuration before overwrite.

Remote extraction creates files on the server and consumes disk. Inspect untrusted archives in isolation because they may contain unsafe paths or content. Resolve extraction conflicts explicitly.

## Change permissions and delete

Understand owner, group, and other permission bits before changing them. Avoid exposing private keys or removing application execute rights. Test recursive changes on a narrow path first.

Remote deletion normally bypasses the local trash. Verify the full path, count, backup, and impact on running processes.

## Edit internally or externally

The built-in code/text editor supports find, replace, soft wrap, reload, and save, with unsaved-close protection. If the remote file changes or disappears while open, compare both versions before reload or intentional overwrite.

External-editor extensions can define a default editor and executable path. A local save may upload automatically, but Navop still checks for remote changes first. Automatic upload is not version control; preserve backups for critical files.
