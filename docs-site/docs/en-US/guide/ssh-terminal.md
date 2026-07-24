# SSH and terminal workspaces

Navop combines remote SSH and local Terminal sessions with tabs, splits, broadcast input, history, quick commands, highlighting, and shell integration. Every command acts on the current local machine or remote server, so keep the host and identity visible.

## Create SSH and local profiles

SSH authentication supports passwords, private-key files, inline private-key content, SSH Agent, and default `~/.ssh` configuration. Interactive servers may request MFA. Configure proxy and timeout when necessary, and verify the host fingerprint on first connection.

Local profiles include the system shell, PowerShell, CMD, WSL, Git Bash, or a custom program. Select the profile when opening a terminal; unavailable platform-specific choices are hidden. Use trusted executables and safely parsed arguments, and do not place passwords in command-line parameters.

The terminal AI sidebar works with both SSH and local sessions and uses the active terminal as its default resource context. Verify the shell, operating system, host, and current directory before running generated commands; local PowerShell, custom programs, and remote Linux shells are not interchangeable.

![Agent terminal and workspace explorer](/images/agent_hub.png)

## Use the workspace explorer and Git

Open the workspace explorer beside a terminal to keep the project tree, current changes, and active session visible together. Select a workspace directory explicitly or follow the terminal working directory. Hidden files and Git-ignored files have separate visibility controls. Before moving, deleting, or editing files, confirm whether the explorer represents a local directory or a remote session.

![Git branch management](/images/git_branch.png)

The branch panel supports search, refresh, fetching remote branches, pushing, creating branches, and switching between local or remote branches. Save or stage current work before switching, verify the target and its tracking relationship, and do not assume fetching resolves conflicts.

![Side-by-side Git diff](/images/git_diff.png)

Open a changed file to compare HEAD with the working copy side by side, then use search, reload, and change navigation to review each edit. After an Agent or script changes the project, read the diff, run checks, and only then commit. A read-only diff does not replace formatting, type checking, or tests.

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
