# Public MCP and external automation

Public MCP lets Codex, Claude Desktop, Claude Code, and compatible clients call the currently running Navop. It is not a fixed cloud API: discovery uses a dynamic loopback port and a user-only token, while live tools and schemas come from the local application.

## Choose a server mode

Temporary mode is appropriate for short-lived use; clients should not expect it to remain available after the task or app ends. Persistent mode supports clients that need continuing local discovery. Both should remain loopback-only and use the protected per-user discovery information.

Do not expose discovery files, tokens, or ports to the public network. Changing mode, restarting Navop, or changing Tool Exposure may restart the endpoint, requiring MCP and ACP clients to reconnect.

## Set permissions and Tool Exposure

Safe, Confirm, and Auto profiles control approval behavior. Begin with Safe or Confirm, and use Auto only when the task, target, and tools are tightly controlled. These profiles do not replace database or server authorization.

Tool Exposure independently controls Terminal, SSH Exec, visible terminal, Connections, SFTP, Redis, MongoDB, Database, and internal functions. Enable only the groups needed for the current client, then turn them off afterward.

## Install client prerequisites

The bridge requires Node.js 20+ and working `npx`. Verify the version, then copy the generated configuration for Codex, Claude Desktop, Claude Code, or a generic MCP JSON client from Navop Settings. Follow each client's path and restart instructions.

Navop can also install or update its Skill for Codex and Agents. The npm package is not a static tool registry; a client must connect to running Navop to obtain current tools and schemas.

## Use the @navop/mcp CLI

The `@navop/mcp` CLI provides `status`, `tools`, `schema`, `call`, and `mcp`. Use status to test discovery, tools to list current exposure, schema to read live parameters, call for an explicit invocation, and mcp as the client bridge.

Run `npx @navop/mcp ...` only after checking package source and version. Always obtain resource IDs and argument definitions from the current tools/schema output. Never guess IDs, reuse IDs from another device, or attempt to bypass approval.

## Approve and troubleshoot

The approval window shows the requesting client, operation, resource, and parameters. Reject unexpected requests and correct the client instead of globally weakening permissions. ACP authorization does not automatically approve Public MCP.

For failures, check that Navop is running, server mode, Node.js, client configuration, discovery permissions, Tool Exposure, and live schema. Reconnect after endpoint changes. Redact tokens, local paths, connection names, and business parameters from logs.
