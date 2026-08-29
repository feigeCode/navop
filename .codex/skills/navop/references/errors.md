# JSON and errors

Successful `--json` output:

```json
{"ok":true,"result":{}}
```

Failed `--json` output:

```json
{"ok":false,"code":"...","message":"...","details":{}}
```

Exit codes:

- `0`: success
- `2`: invalid CLI arguments or an existing Skill target
- `3`: Navop is not running or Public MCP is unavailable
- `4`: discovery or MCP protocol error
- `5`: requested Navop tool is not exposed
- `6`: tool failure, permission denial, or approval rejection
- `8`: timeout
- `9`: connection closed

For exit `3`, ask the user to start Navop and enable **Settings > General > MCP > MCP Server**. For exit `5`, report the exact missing tool and ask the user to enable its group under **Settings > General > Tool Exposure** and open the relevant connection/session. For exit `6`, report the denial or tool error; never bypass it. For exits `8` or `9`, retry read-only discovery when appropriate, but do not retry mutations automatically.
