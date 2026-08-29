# Safety and ownership

Navop is the only implementation, permission, approval, session, and audit boundary. The CLI must not bypass it.

- Respect approval rejection and permission denial as final for the attempted operation.
- Prefer read-only commands before mutations.
- Do not retry a mutation after timeout or connection loss; the outcome may be unknown.
- Do not send arbitrary bytes to a visible terminal.
- Do not map Agent cancellation to terminal interruption.
- Use `navop terminal interrupt ... --json` only after an explicit user request to interrupt the visible foreground process.
- Do not expose or print the discovery token.
- Do not guess connection or session identifiers.
- Do not silently overwrite a Skill or an SFTP/local file target.
- Do not use the low-level tool interface to bypass a safer domain workflow.
