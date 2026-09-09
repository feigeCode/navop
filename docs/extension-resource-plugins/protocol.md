# IPC Protocol

The MVP protocol is headless. Supported method families are:

- lifecycle: `init`, `shutdown`, cancellation;
- resource: open, ping, invoke, close;
- job: start, status, cancel, result, close;
- event stream: open, read, close;
- blob: open, read, close;
- reverse host API: credentials, secrets, notification, storage, logging and host blob upload.

Removed method families:

- `ui/action`;
- `ui/dialog`;
- `ui/window`.

The old declarative UI DTOs and validation limits are no longer part of extension-protocol.
Future gpui-shell APIs are host modules, not additions to this provider wire protocol.
