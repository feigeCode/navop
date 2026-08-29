
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
    <a href="https://docs.navop.dev/en-US/">Documentation</a> ·
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

- Built-in support for MySQL, PostgreSQL, SQLite, DuckDB, SQL Server, Oracle, and ClickHouse; extension drivers add Dameng DM, KingbaseES, GBase 8s, OceanBase, openGauss, Apache IoTDB, and Oscar.
- Browse database objects, edit and run SQL with execution plans, import and export data, compare schemas and data, and visualize relationships with ER diagrams.
- Dedicated Redis and MongoDB interfaces, persistent SQL history, stored procedure and function editing, and proxy or SSH-tunnel routing for supported connections.

### Remote access and operations

- SSH and local terminals with draggable split panes, quick commands, broadcast input, shell integration, session lock, recording and replay, and session logs; Telnet and serial connections are also supported.
- Manage remote files with SFTP uploads, downloads, search, favorites, remote editing, drag-and-drop, ZMODEM transfer, and server-to-server copy.
- Reusable local, remote (`ssh -R`), and dynamic SOCKS port forwarding; X11 forwarding; host-key change warnings with explicit fingerprints; optional legacy SSH algorithms.
- Import SecureCRT sessions, monitor servers, and connect to remote desktops through installable RDP and VNC providers.

### Editing, AI, and extensibility

- Local Markdown notes with Mermaid diagrams, math rendering, and export to HTML, PDF, or DOCX.
- AI for SQL generation and explanation, data analysis, charts, terminal assistance, tool calling, and agent workflows; connect external agents through ACP for Codex, Claude Code, and OpenCode.
- Agent Hub keeps a terminal agent, project files, Git branches, changes, and side-by-side diffs in one workspace; the extension marketplace adds database drivers, remote desktop providers, document renderers, connection importers, and external editors. First-party extensions are built and published from the [navop-extensions](https://github.com/feigeCode/navop-extensions) repository.

### Native desktop experience

- Native GPUI interface with GPU-accelerated rendering; light, dark, and system themes, importable themes, accent colors, and window opacity.
- English, Simplified Chinese, and Traditional Chinese interfaces.
- Encrypted synchronization of personal connections, credentials, and settings across devices.

## Public MCP, Navop CLI, and Agent Skill

Navop can expose selected host-authoritative tools to external Codex, Claude, MCP clients, and automation. Enable the server under **Settings > General > MCP > MCP Server**, choose a permission profile (Safe / Confirm / Auto), and select the required groups under **Tool Exposure**. The runtime listens on a dynamic loopback-only port and authenticates clients with a user-only discovery token; Navop remains authoritative for live tools, schemas, permissions, approvals, and audit records.

For terminal-capable agents, install the [`@navop/cli`](https://github.com/feigeCode/navop-mcp) package and the bundled Navop Skill:

```bash
npm install -g @navop/cli@latest

# Install the Skill for Codex, or use --target agents for Agents-compatible clients
navop skill install --target codex --scope user
```

The separate [`@navop/mcp`](https://github.com/feigeCode/navop-mcp) package is only the stdio bridge for native MCP clients (`npx -y @navop/mcp@latest`). See the [Navop MCP and CLI repository](https://github.com/feigeCode/navop-mcp) and the [Public MCP guide](https://docs.navop.dev/en-US/guide/public-mcp) for the full command reference and client configuration.

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

Download the latest build from [GitHub Releases](https://github.com/feigeCode/navop/releases/latest). Each release includes `sha256sums.txt` for checksum verification. Artifacts are available for macOS (DMG and tar.gz, Apple Silicon / Intel), Windows (MSI and EXE installers, plus standard and portable ZIP), and Linux (tar.gz, deb, rpm, and AppImage), following the `navop-<version>-<platform>-<arch>.<ext>` naming convention.

Navop is also available from [FlatPark](https://flatpark.org/apps/dev.navop.Navop/) as a community Flatpak package:

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
```

If macOS Gatekeeper reports that Apple cannot check the app, run `sudo xattr -rd com.apple.quarantine /Applications/Navop.app`.

For the full artifact table, Windows portable-mode notes, upgrade migration from v0.10.1 or earlier ZIPs, and the Oracle Instant Client / pure-Go driver note, see the [Install & update guide](https://docs.navop.dev/en-US/guide/install-update).

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

## Star History

<a href="https://star-history.dera.page/#feigeCode/navop&type=date&logscale=&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=feigeCode/navop&type=date&theme=dark&logscale&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=feigeCode/navop&type=date&logscale&legend=top-left" />
   <img alt="Star History Chart" src="https://star-history.dera.page/svg?repos=feigeCode/navop&type=date&logscale&legend=top-left" />
 </picture>
</a>

## Credits

ER diagram rendering is based on [ferrum-flow](https://github.com/tu6ge/ferrum-flow.git).

## License

Navop source code is licensed under [Apache License 2.0](LICENSE-APACHE). Navop-authored portions are additionally subject to the [Navop Supplementary License](NAVOP_LICENSE), which permits free redistribution through free distribution channels (such as GitHub Releases, Flatpak repositories, and free app stores) while prohibiting commercial resale, charging fees, competing products or services, and paid distribution platforms. Third-party components remain subject to their own licenses.

For licensing inquiries, contact xiaofei.hf@gmail.com.
