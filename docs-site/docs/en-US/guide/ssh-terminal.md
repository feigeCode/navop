# SSH and terminal workspaces

Navop combines remote SSH and local Terminal sessions with tabs, splits, broadcast input, history, quick commands, highlighting, and shell integration. Every command acts on the current local machine or remote server, so keep the host and identity visible.

## Create SSH and local profiles

SSH authentication supports passwords, private-key files, inline private-key content, SSH Agent, and default `~/.ssh` configuration. Interactive servers may request MFA. Configure proxy and timeout when necessary, and verify the host fingerprint on first connection.

Local profiles include the system shell, PowerShell, CMD, WSL, Git Bash, or a custom program. Use trusted executables and safely parsed arguments; do not place passwords in command-line parameters.

## Arrange tabs and splits

Split a terminal Left, Right, Top, or Bottom, drag tabs between panes, and resize the layout. Splits are useful for logs, commands, and environment comparisons. Before closing a pane, check for foreground jobs, editors, or unfinished processes.

After reconnecting, verify process state and directory. Long-running work should use server-side session management rather than assuming a GUI tab preserves it.

## Use broadcast and paste protection

Broadcast input targets only selected open SSH terminals. Verify every host, user, and current directory; send a harmless `pwd` first. One mistaken command can otherwise affect every selected server.

Multiline paste outside bracketed-paste mode and high-risk commands trigger confirmation. Right- or middle-click paste and selection copy are configurable. Review clipboard text containing newlines, pipes, redirection, or deletion in a text editor first.

## Reuse commands and highlighting

History groups recent, frequent, and favorite commands. Quick commands have names, descriptions, commands, groups, colors, pinning, search, and copy/paste actions. Deleting a group does not delete its commands, but review organization after changes.

Custom regular-expression highlighting and presets can emphasize errors or identifiers. Highlighting is visual only and does not prove command success.

## Integrate shell and files

Shell integration can synchronize the SSH current directory with the file manager. Compatibility depends on the remote shell and initialization. Clipboard images can be uploaded to SSH and their path inserted into the terminal; confirm destination, permissions, and sensitivity before upload.

Server monitoring deployment and privacy are covered in the remote-access chapter.
