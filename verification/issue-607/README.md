# PR #607 VNC verification

Environment: `run-navop-docker.sh` on Xvfb `:99`, VNC port `5900`, llvmpipe Vulkan.

- `598-keyboard-interactive.png`: SSH basic info now exposes the keyboard-interactive / two-factor switch.
- `604-json-view.png`: selected table record opens a formatted JSON tree.
- `604-json-raw.png`: JSON view switches to raw mode.
- `594-ctrl-s-edited.png`: table cell edited to `张伟S`; Ctrl+S was pressed and SQLite query returned `1|张伟S`.
- `594-f5-refresh-restored.png`: external DB value restored to `张伟`, then F5 refreshed the table back to `张伟`.
- `594-final-restored.png`: final restored table state.
- `592-ssh-disconnected.png`: persistent SSH session shows Connection Lost after `exit`.
- `592-ssh-reconnected.png`: Enter triggers reconnect and the shell becomes usable again.

`#592` real GateShell/明御 bastion state still requires a live bastion environment.
