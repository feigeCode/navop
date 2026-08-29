---
name: navop
description: Use terminal commands to discover and operate resources exposed by a running Navop desktop application through its host-authoritative Public MCP runtime.
---

# Navop

Use this Skill when an AI Agent needs to operate Navop resources from a terminal. It keeps a compact workflow in context and loads command help or live schemas only when needed, so the Agent does not carry the complete Navop tool catalog in every turn. This can reduce repeated tool-definition tokens and other context overhead; the amount depends on the client and the number of exposed tools.

The CLI still connects internally to Navop's authenticated loopback Public MCP runtime. Navop remains authoritative for Tool Exposure, permissions, approvals, resource IDs, sessions, results, and audit records. The CLI does not implement SSH, SFTP, terminal, database, Redis, or MongoDB business logic.

The Navop CLI must be installed globally before this Skill can operate resources. Check the command, and install the latest client when it is missing:

```bash
navop --version
npm install -g @navop/cli@latest
navop --version
```

Every Agent-initiated command must include `--json`. Start with:

```bash
navop status --json
```

Read the status result before choosing a tool:

- If Public MCP is unavailable, ask the user to start Navop and enable **Settings > General > MCP > MCP Server**.
- Inspect `permissionMode`, `availableTools`, `toolGroups`, `disabledToolGroups`, and `guidance`.
- Treat the running host's `tools/list` response and `navop.runtime_status` result as authoritative. The npm package and this Skill do not define the host's complete tool set.
- If a tool is unavailable, report its exact name and host-reported tool-group state. Ask the user to enable that group under the returned `settingsPath` and open the required connection or session.
- `deny` blocks mutations. `ask` requires approval in Navop. `allow` runs mutations automatically, but still requires clear user intent for destructive actions.
- Never claim a capability is enabled merely because the CLI has a convenience command; the live `availableTools` list is authoritative.

Discover the current client and available command surface when needed:

```bash
navop --help
navop tool list --json
navop tool schema <tool-name> --json
navop db query --help
navop db exec --help
```

Prefer a domain command whenever `navop --help` exposes one for the requested operation. Domain commands are the normal Agent interface; `tool call` is a low-level fallback, not the default. Use the command's help and validate its flags against the live schema.

For SQL, choose the command by intent:

- Read-only statements use `navop db query`.
- DDL, DML, scripts, and other write-capable SQL use `navop db exec`.
- When `navop db exec` is available, do not replace it with `navop tool call db.exec`.
- Do not pass onetcli-only policy flags such as `--allow-write` to the `navop` CLI. Navop's live `permissionMode` and approval flow govern mutations.

Only when no matching domain command is exposed, or when its mapping cannot represent the live input schema, call the exact host tool:

```bash
navop tool call <tool-name> --arguments '<json-object-matching-live-schema>' --json
```

Never invent a command, tool name, flag, argument, resource ID, or session ID from examples in this Skill. Discover resource and session methods from `tool list`, inspect their schemas, and use only IDs returned by Navop.

## Updating the npm client

Check the registry and installed executable when diagnosing a stale client:

```bash
npm view @navop/cli version
navop --version
```

Update the installed CLI before using features that require a newer client:

```bash
npm install -g @navop/cli@latest
npm update -g @navop/cli
navop --version
navop status --json
```

Prefer reads before writes. Preserve Navop permission and approval decisions. Never work around a denial. Do not retry a mutating operation after timeout or connection loss because its outcome may be unknown. Do not interrupt or control a visible terminal unless the user explicitly requests that action.

Read these references as needed:

- [commands.md](references/commands.md): dynamic command discovery and Bash examples.
- [workflows.md](references/workflows.md): safe discovery and execution sequences.
- [safety.md](references/safety.md): approvals, writes, terminal ownership, and retry rules.
- [errors.md](references/errors.md): JSON envelope, exit codes, and recovery behavior.
