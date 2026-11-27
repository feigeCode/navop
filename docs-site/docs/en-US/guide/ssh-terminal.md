# SSH and terminal workspaces

Navop combines remote SSH and local Terminal sessions with tabs, splits, broadcast input, history, quick commands, highlighting, and shell integration. Every command acts on the current local machine or remote server, so keep the host and identity visible.

## Create SSH and local profiles

SSH authentication supports passwords, private-key files, inline private-key content, SSH Agent, default `~/.ssh` configuration, and no authentication (for trusted internal hosts). Interactive servers may request MFA, and the basic authentication section has a keyboard-interactive switch for servers that need challenge-response prompts. Configure proxy and timeout when necessary, and verify the host fingerprint on first connection. Navop can generate a key pair for you; the public key stays visible after generation so you can copy it into `authorized_keys` or a hosting console.

The SSH connection's Advanced Settings include an "Allow Legacy SSH Algorithms" compatibility option (off by default). Enable it per connection only when a server supports only older algorithms such as DSA host keys, SHA-1 key exchange/MAC, or 1024-bit DH group negotiation; Windows also supports Pageant authentication. Enabling it lowers cryptographic strength, so do not keep it on for servers that can be upgraded.

A key credential can be marked "Forward this key through ssh-agent", and the connection's Advanced Settings also carry an "SSH Agent Forwarding" (ForwardAgent) switch. With the local ssh-agent forwarded to the remote host, that host (a jump host, for example) can authenticate onward to deeper hosts with your local key; the setting applies to newly opened terminal sessions, so reconnect existing ones.

Local profiles include the system shell, PowerShell, CMD, WSL, Git Bash, or a custom program. Select the profile when opening a terminal; unavailable platform-specific choices are hidden. Use trusted executables and safely parsed arguments, and do not place passwords in command-line parameters.

The terminal AI sidebar works with both SSH and local sessions and uses the active terminal as its default resource context. Verify the shell, operating system, host, and current directory before running generated commands; local PowerShell, custom programs, and remote Linux shells are not interchangeable.

## Manage known hosts

The Known Hosts page lists every SSH host trusted by Navop with its host/port, route (direct, proxy, jump), key algorithm, and SHA-256 fingerprint, sorted by most recent use. The toolbar provides:

- **Scan System**: imports plain-text entries from the system OpenSSH `known_hosts` file (hashed names, wildcards, CAs, and revoked entries are skipped); already-trusted hosts are never overwritten.
- **Copy host identity**: copies the full identity (host:port + route) to the clipboard for diagnostics or shared confirmation.
- **Remove trusted host**: deletes the host key from the app trust store; the next connection asks for host-key confirmation again.

After removing a trust record, reconnect and confirm to restore trust if the server key is unchanged. Host-key change warnings continue to rely on the fingerprint comparison shown at connect time.

## Agent Hub: code, resources, and Git

Agent Hub brings the terminal Agent, project file tree, Git branches, change list, and side-by-side diff into one workspace, keeping coding, resource navigation, and version control in context beside Navop's local terminals, SSH sessions, and connections.

![Agent Hub workspace](/images/agent_hub.png)

- Select a workspace directory explicitly or follow the terminal working directory, with separate controls for hidden and Git-ignored files.
- Search, create, fetch, push, and switch local or remote branches without leaving the workspace.
- After an Agent edits the project, jump to changed files and compare the working copy with HEAD before committing.

Save or stage work before switching branches, and verify the target and its tracking relationship. Before moving, deleting, or editing files, confirm whether the explorer represents a local directory or a remote session. Agent-generated changes still require diff review, formatting, checks, and tests.

## X11 forwarding: three prerequisites

An SSH connection can enable X11 forwarding so remote X11 applications render through a local X server. Enabling it requires three elements, all of which are necessary:

1. **The server has X11 enabled** — the remote SSH service must allow X11 Forwarding and support `xauth` (MIT-MAGIC-COOKIE-1); restart `sshd` after changing `sshd_config`.
2. **An X11 server is installed on the client** — your machine needs a running X server: XMing on Windows, XQuartz on macOS.
   - XMing: [https://sourceforge.net/projects/xming](https://sourceforge.net/projects/xming)
   - XQuartz: [https://www.xquartz.org](https://www.xquartz.org), or install it with Homebrew: `brew install --cask xquartz`
   - After installing XQuartz on macOS, run `xhost +local:` to grant local access.
3. **X11 is checked in session details** — enable the "X11 forwarding" option in the SSH connection settings; it applies to newly opened terminal sessions.

Navop auto-detects the local X server: it probes `127.0.0.1:6000` on Windows, and on macOS it resolves DISPLAY, launchctl, and Xauthority in turn. If XQuartz is not detected on macOS, checking the box or testing the connection prompts you to install and start XQuartz; when no usable X11 environment is available, Navop disables forwarding without blocking the regular SSH terminal.

Recent versions improve detection of older XQuartz environments, sockets, and authentication data on Intel Macs. If forwarding still fails, verify that XQuartz is running, reconnect the SSH session, and then check the remote `sshd` configuration and account permissions.

## Arrange tabs and splits

Split a terminal Left, Right, Top, or Bottom, drag tabs between panes, and resize the layout. Splits are useful for logs, commands, and environment comparisons. Before closing a pane, check for foreground jobs, editors, or unfinished processes.

After reconnecting, verify process state and directory. Long-running work should use server-side session management rather than assuming a GUI tab preserves it.

Terminal search accepts case-sensitive, whole-word, and regular-expression options. Whole-word matching also works for non-ASCII text such as Chinese, and a regular expression is interpreted as entered rather than rewritten. Search only locates text in the current buffer; it does not change remote state.

## tmux integration and panes

Terminal settings include "Enable tmux integration", a global option that applies to local and SSH terminals. When enabled, Navop tracks panes and sessions through a tmux control channel; changing the toggle or the custom socket path rebuilds that channel. Leave the socket path empty to use the tmux default.

When the session runs tmux, the terminal sidebar shows a "tmux panes" panel where you can split the window or select a pane. Splits and selections go through the control channel instead of simulated keystrokes. tmux 3.2 or later is required. If tmux is missing or the channel drops, the panel reports it and the ordinary terminal keeps working.

## Terminal notifications

Programs can emit OSC 9 or OSC 777 notifications — common for CLI approval prompts and task-completion messages from agent CLIs. Navop shows a dot on the tab and, depending on your settings, also a system notification.

In "Settings → Terminal → System notifications" choose when to notify (when the window is unfocused or the tab is not active / only when the window is unfocused / always / off), the display duration, and the sound. "Send test notification" verifies that the platform actually delivers notifications: macOS uses Notification Center and asks for permission the first time, Linux goes through the freedesktop D-Bus service, and duration and sound options appear only where the platform supports them.

For Codex CLI, enable OSC 9 notifications in `~/.codex/config.toml` first; otherwise the CLI never emits them.

## Lock sessions and read status

Sessions can be locked with a password kept only in memory, including locking all sessions at once or hiding the output of the current session. A locked terminal rejects keystrokes and cannot be closed with the close button. SecureCRT-style status badges on tabs show connected, disconnected, and connected-and-locked states with tooltips.

Locking is local session protection and does not replace remote account permissions. Decide whether to hide output before stepping away, and follow organizational security policy.

## Use broadcast and paste protection

Broadcast input targets only selected open SSH terminals. Verify every host, user, and current directory; send a harmless `pwd` first. One mistaken command can otherwise affect every selected server.

Multiline paste outside bracketed-paste mode and high-risk commands trigger confirmation. The paste review dialog lets you edit the text or send it line by line. Right- or middle-click paste and selection copy are configurable. Review clipboard text containing newlines, pipes, redirection, or deletion in a text editor first.

## Reuse commands and highlighting

History groups recent, frequent, and favorite commands. Quick commands have names, descriptions, commands, groups, colors, pinning, search, and copy/paste actions; the quick-command editor also offers an "execute on click" option that runs a command immediately on click. The suggestion-popup toggle can be adjusted independently in terminal settings. Deleting a group does not delete its commands, but review organization after changes.

Custom regular-expression highlighting and presets can emphasize errors or identifiers. Highlighting is visual only and does not prove command success.

## Integrate shell and files

Shell integration can synchronize the SSH current directory with the file manager. Runtime SSH shell integration senses prompts, command start/end, and the current directory in real time without writing to the remote host, laying the groundwork for automation and script hints. Compatibility depends on the remote shell and initialization; do not fabricate path state when synchronization fails. The terminal shows connection status and auth prompts inline instead of interrupting with popups. Clipboard images can be uploaded to SSH and their path inserted into the terminal; confirm destination, permissions, and sensitivity before upload.

Server monitoring and privacy are covered in the remote-access chapter.

## Telnet connections

Navop also supports Telnet connections with automatic login scripts, manual credential overrides, and a configurable backspace code for older devices. Telnet transmits credentials in cleartext by default, so prefer SSH in production environments.
