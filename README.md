<div align="center">
  <p><img src="resources/navop-icon.png" alt="Navop" width="120" /></p>
  <h1>Navop</h1>
  <p><strong>A native, all-in-one workspace for databases, SSH, SFTP, terminals, remote desktop, monitoring, and AI.</strong></p>
  <p>Built with <a href="https://gpui.rs">GPUI</a> and Rust · GPU-accelerated rendering</p>

  <p>
    <a href="https://github.com/feigeCode/navop/releases"><img src="https://img.shields.io/github/downloads/feigeCode/navop/total?style=for-the-badge&color=blue" alt="Downloads" /></a>
    <a href="https://github.com/feigeCode/navop/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/feigeCode/navop/ci.yml?branch=main&style=for-the-badge" alt="CI" /></a>
    <a href="#license"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20supplementary%20terms-blue?style=for-the-badge" alt="License: Apache-2.0 plus supplementary terms" /></a>
  </p>

  <p>
    <a href="README_CN.md">中文</a> ·
    <a href="#features">Features</a> ·
    <a href="#screenshots">Screenshots</a> ·
    <a href="#install">Install</a> ·
    <a href="https://github.com/feigeCode/navop/releases/latest">Latest Release</a> ·
    <a href="CONTRIBUTING.md">Contributing</a>
  </p>

  <p><img src="app1.png" alt="Navop overview" width="820" /></p>
</div>

## Features

### Databases and data tools

- Connect to MySQL, PostgreSQL, SQLite, DuckDB, SQL Server, Oracle, and ClickHouse.
- Install extension drivers for Dameng DM, KingbaseES, GBase 8s, OceanBase, openGauss, Apache IoTDB, and Oracle without Instant Client.
- Browse database objects, edit and run SQL, inspect execution plans, import or export data, compare schemas and data, and visualize relationships with ER diagrams.
- Work with Redis and MongoDB through dedicated interfaces.
- Route supported network connections through SOCKS5 or HTTP CONNECT proxies and SSH tunnels.

### Remote access and operations

- Use SSH and local terminals with draggable splits, quick commands, history, broadcast input, shell integration, and terminal AI.
- Manage remote files with SFTP uploads, downloads, search, favorites, remote editing, drag-and-drop, and server-to-server copy.
- Create reusable local, remote (`ssh -R`), and dynamic SOCKS port-forwarding connections.
- Open serial connections, monitor servers, and connect to remote desktops through installable RDP and VNC providers.

### Editing, AI, and extensibility

- Edit local Markdown notes with syntax highlighting, Mermaid diagrams, math rendering, relative media, and export to HTML, PDF, or DOCX through a sandboxed WASM exporter.
- Use AI for SQL generation and explanation, data analysis, charts, terminal assistance, tool calling, and agent workflows.
- Connect external agents through ACP extensions for Codex, Claude Code, and OpenCode.
- Use Agent Hub to keep a terminal agent, project files, Git branches, changes, and side-by-side diffs in one workspace.
- Add database drivers, remote desktop providers, document renderers, and other capabilities through the extension marketplace.

### Native desktop experience

- Native GPUI interface with GPU-accelerated rendering.
- Light, dark, and system themes, importable application and terminal themes, accent colors, and window opacity controls.
- English, Simplified Chinese, and Traditional Chinese interfaces.
- Encrypted synchronization of connections and settings across devices.

## Public MCP, Navop CLI, and Agent Skill

Navop can expose selected host-authoritative tools to external Codex, Claude, MCP clients, and automation. Enable the server under **Settings > General > MCP > MCP Server**, choose a permission profile, and select the required groups under **Tool Exposure**.

The runtime listens on a dynamic loopback-only port and authenticates clients with the token in Navop's user-only discovery file. Tool schemas, permissions, approvals, connections, sessions, results, and auditing remain controlled by the running Navop application.

Use the separately published [`@navop/cli`](https://github.com/feigeCode/navop-mcp) package for terminal-capable agents, or [`@navop/mcp`](https://github.com/feigeCode/navop-mcp) as a stdio bridge for MCP clients:

```bash
npm install -g @navop/cli@latest
navop status --json
navop tools --json
navop schema <tool-name> --json
navop call <tool-name> --arguments '<json-object>' --json

# Start the MCP stdio bridge without a global install
npx -y @navop/mcp@latest
```

The CLI and Agent Skill discover commands and live schemas only when needed, reducing recurring tool context. Available capabilities currently include runtime discovery, SSH, visible terminals, SQL databases, Redis, MongoDB, SFTP, connections, workspaces, and registered host functions. Actual availability always depends on the running Navop instance, enabled Tool Exposure groups, and the selected permission profile:

| Profile | Behavior |
| --- | --- |
| Safe / `deny` | Allows read-only discovery and denies mutations |
| Confirm / `ask` | Requires approval in the Navop UI for mutations |
| Auto / `allow` | Runs mutations automatically; destructive intent must still be explicit |

See the [Navop MCP and CLI repository](https://github.com/feigeCode/navop-mcp) for client setup and command documentation.

## Screenshots

| Database | SSH |
|:-:|:-:|
| [![Database](database.png)](database.png) | [![SSH](ssh.png)](ssh.png) |

| SFTP | Redis |
|:-:|:-:|
| [![SFTP](sftp.png)](sftp.png) | [![Redis](redis.png)](redis.png) |

| MongoDB | AI Chat |
|:-:|:-:|
| [![MongoDB](mongodb.png)](mongodb.png) | [![AI Chat](chatdb.png)](chatdb.png) |

| Agent Hub | Extensions |
|:-:|:-:|
| [![Agent Hub](agent_hub.png)](agent_hub.png) | [![Extensions](extension.png)](extension.png) |

| Remote File Editor | ER Diagram |
|:-:|:-:|
| [![Remote File Editor](remote_file_editor.png)](remote_file_editor.png) | [![ER Diagram](er.png)](er.png) |

| Monitoring | Themes |
|:-:|:-:|
| [![Monitoring](monitor.png)](monitor.png) | [![Themes](theme.png)](theme.png) |

## Install

Download the latest build from [GitHub Releases](https://github.com/feigeCode/navop/releases/latest). Each release includes `sha256sums.txt` for checksum verification.

| Platform | Architecture | Artifacts |
| --- | --- | --- |
| macOS | Apple Silicon, Intel | `.dmg`, `.tar.gz` |
| Linux | x86_64 | `.tar.gz`, `.deb`, `.rpm`, `.AppImage` |
| Linux | ARM64 | `.tar.gz` |
| Windows | x86_64 | `.msi`, `.zip` |

The Windows MSI is a bilingual per-user installer and does not require administrator privileges when using the default location. The ZIP archive is available for portable use.

### macOS Gatekeeper

If macOS reports that Apple cannot check the app for malicious software after installing the DMG, run:

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

### Oracle

The built-in Oracle driver requires [Oracle Instant Client](https://www.oracle.com/database/technologies/instant-client/downloads.html). Alternatively, install the pure-Go Oracle driver from the extension marketplace to use Oracle without Instant Client.

## Build from source

Navop uses the Rust 2024 edition and requires platform-specific system dependencies.

```bash
# Linux dependencies
./script/bootstrap

# Run the application
cargo run -p main
```

On Windows, install dependencies from PowerShell with:

```powershell
.\script\install-window.ps1
```

Common development checks:

```bash
cargo build
cargo test --all
cargo clippy --workspace --all-targets
cargo fmt --check
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the complete development guide.

## Community and support

Navop is maintained independently. Stars, focused pull requests, bug reports, and donations all help sustain the project.

- [Report a bug or request a feature](https://github.com/feigeCode/navop/issues)
- QQ Group: [860670605](https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605)
- WeChat Group: [Join](https://docs.qq.com/doc/DVEFFd2RnSnJLcFBD)
- Optional donations: [DONATE.md](DONATE.md)
- Legacy OnetCli repository: [feigeCode/onetcli](https://github.com/feigeCode/onetcli)

## Credits

ER diagram rendering is based on [ferrum-flow](https://github.com/tu6ge/ferrum-flow.git).

## License

Navop source code is licensed under [Apache License 2.0](LICENSE-APACHE). Navop-authored portions are additionally subject to the [Navop Supplementary License](NAVOP_LICENSE), which adds restrictions on redistribution, resale, competing products or services, and unauthorized distribution platforms. Third-party components remain subject to their own licenses.

For licensing inquiries, contact xiaofei.hf@gmail.com.
