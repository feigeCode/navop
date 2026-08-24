> 老仓库 / Legacy repository：<https://github.com/feigeCode/onetcli> · [![OnetCli Stars](https://img.shields.io/github/stars/feigeCode/onetcli?style=flat-square&logo=github&label=OnetCli%20Stars)](https://github.com/feigeCode/onetcli)

<div align="center">
  <p><img src="resources/navop-icon.png" alt="Navop" width="120" /></p>
  <h1>Navop</h1>
  <p><strong>A native, all-in-one workspace for databases, SSH, SFTP, terminals, remote desktop, monitoring, and AI.</strong></p>
  <p>Built with <a href="https://gpui.rs">GPUI</a> and Rust · GPU-accelerated rendering</p>

  <p>
    <a href="https://github.com/feigeCode/navop/releases"><img src="https://img.shields.io/github/downloads/feigeCode/navop/total?style=for-the-badge&color=blue" alt="Downloads" /></a>
    <a href="https://github.com/feigeCode/navop/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/feigeCode/navop/ci.yml?branch=dev&style=for-the-badge" alt="CI" /></a>
    <a href="#license"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20supplementary%20terms-blue?style=for-the-badge" alt="License: Apache-2.0 plus supplementary terms" /></a>
    <a href="https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605"><img src="https://img.shields.io/badge/QQ%20Group-860670605-EB1923?style=for-the-badge&logo=tencentqq&logoColor=white" alt="QQ Group 860670605" /></a>
    <a href="https://docs.qq.com/doc/DVEFFd2RnSnJLcFBD"><img src="https://img.shields.io/badge/WeChat%20Group-Join-07C160?style=for-the-badge&logo=wechat&logoColor=white" alt="Join WeChat Group" /></a>
  </p>

  <p>
    <img src="https://img.shields.io/badge/MySQL-4479A1?logo=mysql&logoColor=white" alt="MySQL" />
    <img src="https://img.shields.io/badge/PostgreSQL-4169E1?logo=postgresql&logoColor=white" alt="PostgreSQL" />
    <img src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white" alt="SQLite" />
    <img src="https://img.shields.io/badge/DuckDB-FFF000?logo=duckdb&logoColor=black" alt="DuckDB" />
    <img src="https://img.shields.io/badge/ClickHouse-FFCC01?logo=clickhouse&logoColor=black" alt="ClickHouse" />
    <img src="https://img.shields.io/badge/SQL%20Server-CC2927?logo=microsoftsqlserver&logoColor=white" alt="SQL Server" />
    <img src="https://img.shields.io/badge/Oracle-F80000?logo=oracle&logoColor=white" alt="Oracle" />
    <img src="https://img.shields.io/badge/Dameng%20DM-C71D23" alt="Dameng DM" />
    <img src="https://img.shields.io/badge/KingbaseES-005BAC" alt="KingbaseES" />
    <img src="https://img.shields.io/badge/GBase%208s-1E73BE" alt="GBase 8s" />
    <img src="https://img.shields.io/badge/OceanBase-1B9A8C" alt="OceanBase" />
    <img src="https://img.shields.io/badge/openGauss-005EB8" alt="openGauss" />
    <img src="https://img.shields.io/badge/Apache%20IoTDB-1B3A6B?logo=apache&logoColor=white" alt="Apache IoTDB" />
    <img src="https://img.shields.io/badge/Redis-DC382D?logo=redis&logoColor=white" alt="Redis" />
    <img src="https://img.shields.io/badge/MongoDB-47A248?logo=mongodb&logoColor=white" alt="MongoDB" />
    <img src="https://img.shields.io/badge/SSH-111827?logo=gnubash&logoColor=white" alt="SSH" />
    <img src="https://img.shields.io/badge/SFTP-2563EB?logo=filezilla&logoColor=white" alt="SFTP" />
    <img src="https://img.shields.io/badge/Port%20Forwarding-0F766E" alt="Port Forwarding" />
    <img src="https://img.shields.io/badge/RDP-0078D4" alt="RDP" />
    <img src="https://img.shields.io/badge/VNC-5C2D91" alt="VNC" />
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
- Browse database objects, edit and run SQL, inspect execution plans, import or export data, compare schemas and data, and visualize relationships with ER diagrams. Run SQL queries in unlimited-results mode and cancel in-flight execution when needed.
- Edit MySQL stored procedures and functions and PostgreSQL functions and procedures, with overload-aware routine navigation and table information such as sizes and index counts.
- Review persistent SQL execution history, compare schemas and data across databases with improved type mapping and target matching, and choose Native or pure-Go Oracle drivers, including Oracle 11g query-limit support.
- Work with Redis and MongoDB through dedicated interfaces.
- Route supported network connections through SOCKS5 or HTTP CONNECT proxies and SSH tunnels.

### Remote access and operations

- Use SSH and local terminals with draggable split panes in any direction, quick commands, history, broadcast input, shell integration, and terminal AI. Configure SSH encodings (UTF-8, GBK, Big5, Shift_JIS, and more) and terminal types to match legacy environments.
- Lock sessions with a password, lock all active sessions at once, or hide the output of the current session.
- Record sessions and replay them in a read-only timeline viewer that blocks input and online operations.
- Connect over Telnet with automatic login scripts and manual credential overrides.
- Review SSH, serial, and local terminal session logs in a static history viewer with scrollback, selection, search, and TXT export.
- Manage remote files with SFTP uploads, directory uploads, downloads, search, favorites, remote editing, drag-and-drop, and server-to-server copy; transfer files over SSH with ZMODEM.
- Import SecureCRT sessions and quick commands, and batch-manage connections from the sidebar.
- Create reusable local, remote (`ssh -R`), and dynamic SOCKS port-forwarding connections.
- Confirm SSH and SFTP host-key changes with explicit fingerprint warnings, and enable legacy SSH algorithms only when a server requires them.
- Open serial connections, monitor servers, and connect to remote desktops through installable RDP and VNC providers.

### Editing, AI, and extensibility

- Edit local Markdown notes with syntax highlighting, Mermaid diagrams, math rendering, relative media, and export to HTML, PDF, or DOCX through a sandboxed WASM exporter.
- Enlarge Mermaid diagrams and math formulas, and switch between their source and preview views while editing.
- Use AI for SQL generation and explanation, data analysis, charts, terminal assistance, tool calling, and agent workflows.
- Connect external agents through ACP extensions for Codex, Claude Code, and OpenCode.
- Use Agent Hub to keep a terminal agent, project files, Git branches, changes, and side-by-side diffs in one workspace.
- Add database drivers, remote desktop providers, document renderers, and other capabilities through the extension marketplace.

### Native desktop experience

- Native GPUI interface with GPU-accelerated rendering.
- Light, dark, and system themes, importable application and terminal themes, accent colors, and window opacity controls.
- English, Simplified Chinese, and Traditional Chinese interfaces.
- Reusable keychain references and encrypted synchronization of personal connections, credentials, and settings across devices.

## Public MCP, Navop CLI, and Agent Skill

Navop can expose selected host-authoritative tools to external Codex, Claude, MCP clients, and automation. Enable the server under **Settings > General > MCP > MCP Server**, choose a permission profile, and select the required groups under **Tool Exposure**.

The runtime listens on a dynamic loopback-only port and authenticates clients with the token in Navop's user-only discovery file. Navop remains authoritative for live tools and schemas, Tool Exposure, permissions, approvals, resource IDs, sessions, results, and audit records. The CLI and Skill do not implement SSH, SFTP, terminal, database, Redis, or MongoDB business logic.

For terminal-capable Agents, install Node.js 20+, the [`@navop/cli`](https://github.com/feigeCode/navop-mcp) package, and the bundled Navop Skill:

```bash
navop --version
npm install -g @navop/cli@latest
navop --version

# Install the Skill for Codex, or use --target agents for Agents-compatible clients
navop skill install --target codex --scope user
```

The Skill keeps a compact workflow in context and discovers commands and live schemas only when needed. Every Agent-initiated operational command must include `--json`; use `--help` only to discover syntax. Start with runtime status, then inspect the live command and tool surface:

```bash
navop status --json
navop --help
navop tool list --json
navop tool schema <tool-name> --json
navop db query --help
navop db exec --help
```

Read `permissionMode`, `availableTools`, `toolGroups`, `disabledToolGroups`, and `guidance` from `navop status --json`. The running host's `tools/list` response, `navop.runtime_status` result, and live schemas are authoritative; never assume a capability exists because the CLI exposes a convenience command.

Prefer a domain command shown by `navop --help`. For SQL, use `navop db query` for read-only statements and `navop db exec` for DDL, DML, scripts, and other write-capable SQL. Use the low-level host tool fallback only when no domain command can represent the live schema:

```bash
navop tool call <tool-name> --arguments '<json-object-matching-live-schema>' --json
```

Never guess tool names, arguments, resource IDs, or session IDs. Use only values returned by the running Navop instance, preserve its approval decisions, and do not retry a mutation after a timeout or connection loss because its outcome may be unknown.

Actual capabilities depend on the running application, open resources, enabled Tool Exposure groups, and permission profile:

| Profile | Behavior |
| --- | --- |
| Safe / `deny` | Allows read-only discovery and denies mutations |
| Confirm / `ask` | Requires approval in the Navop UI for mutations |
| Auto / `allow` | Runs mutations automatically; destructive intent must still be explicit |

The separate [`@navop/mcp`](https://github.com/feigeCode/navop-mcp) package is only the stdio bridge for native MCP clients:

```bash
npx -y @navop/mcp@latest
```

See the [Navop MCP and CLI repository](https://github.com/feigeCode/navop-mcp) for installation, updates, command reference, and client configuration.

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
| Linux | x86_64 | `.tar.gz`, `.deb`, `.rpm`, `.AppImage`, `-portable.tar.gz` |
| Linux | ARM64 | `.tar.gz`, `-portable.tar.gz` |
| Windows | x86_64, x86 (32-bit) | `.msi`, `.exe`, `.zip`, `-portable.zip` |

The Windows `.msi` and `.exe` are bilingual per-user installers and do not require administrator privileges when using the default location. The EXE installer wraps the same MSI installation. The standard `.zip` requires no installation but still uses the normal per-user data directories and supports remembered master-key unlock. Use `-portable.zip` only when the application data must stay beside the executable. Portable archives (Linux and Windows) ask for the master key on every start by default. You may explicitly choose in Settings to store an encrypted, automatically recoverable copy under `data/state/key_storage`, but this uses a key embedded in the application rather than device-bound protection; anyone who obtains both the application and the complete `data` directory may be able to recover the master key.

> **Upgrading from the Windows ZIP in v0.10.1 or earlier:** those archives enabled portable mode. Download the new `-portable.zip`, extract it to a new directory, and copy the complete existing `data` directory into it. Extracting the new standard `.zip` to a different directory, or switching to the MSI/EXE installer, uses the normal Windows user data location and does not automatically migrate portable data. The old connections and settings may therefore appear missing even though the original portable data has not been deleted.

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
