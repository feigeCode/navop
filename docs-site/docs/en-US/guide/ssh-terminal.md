# SSH and terminal workspaces

Navop combines remote SSH and local Terminal sessions with tabs, splits, broadcast input, history, quick commands, highlighting, and shell integration. Every command acts on the current local machine or remote server, so keep the host and identity visible.

## Create SSH and local profiles

SSH authentication supports passwords, private-key files, inline private-key content, SSH Agent, and default `~/.ssh` configuration. Interactive servers may request MFA. Configure proxy and timeout when necessary, and verify the host fingerprint on first connection.

Local profiles include the system shell, PowerShell, CMD, WSL, Git Bash, or a custom program. Select the profile when opening a terminal; unavailable platform-specific choices are hidden. Use trusted executables and safely parsed arguments, and do not place passwords in command-line parameters.

The terminal AI sidebar works with both SSH and local sessions and uses the active terminal as its default resource context. Verify the shell, operating system, host, and current directory before running generated commands; local PowerShell, custom programs, and remote Linux shells are not interchangeable.

## Agent Hub: code, resources, and Git

Agent Hub brings the terminal Agent, project file tree, Git branches, change list, and side-by-side diff into one workspace, keeping coding, resource navigation, and version control in context beside Navop's local terminals, SSH sessions, and connections.

![Agent Hub workspace](/images/agent_hub.png)

- Select a workspace directory explicitly or follow the terminal working directory, with separate controls for hidden and Git-ignored files.
- Search, create, fetch, push, and switch local or remote branches without leaving the workspace.
- After an Agent edits the project, jump to changed files and compare the working copy with HEAD before committing.

Save or stage work before switching branches, and verify the target and its tracking relationship. Before moving, deleting, or editing files, confirm whether the explorer represents a local directory or a remote session. Agent-generated changes still require diff review, formatting, checks, and tests.

## X11 forwarding and XQuartz on macOS

An SSH connection can enable X11 forwarding so remote X11 applications render through a local X server; the setting applies to newly opened terminal sessions. macOS users must install and start XQuartz, and the remote SSH server must allow X11 forwarding.

v0.9.3 improves detection of older XQuartz environments, sockets, and authentication data on Intel Macs. If forwarding still fails, verify that XQuartz is running, reopen Navop and the SSH session, and then check the remote `sshd` configuration and account permissions. When no usable X11 environment is available, Navop disables forwarding without blocking the regular SSH terminal.

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
