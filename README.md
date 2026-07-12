> [!IMPORTANT]
> **This repository has been archived and OnetCli is no longer maintained.**
> OnetCli will not receive new features, bug fixes, or releases. Active development has moved to **Navop**:
> [Official Website](https://navop.dev) · [GitHub Repository](https://github.com/feigeCode/navop)

<div align="center">
  <p>
    <img src="resources/navop-icon.png" alt="Navop" width="120" />
  </p>

  <h1>Navop</h1>

  <p><strong>Native all-in-one workspace for databases, SSH, SFTP, port forwarding, terminals, remote desktop, monitoring, and AI.</strong></p>

  <p>
    Built with <a href="https://gpui.rs">GPUI</a> · Rust native desktop · GPU-accelerated rendering
  </p>

  <p>
    <a href="https://github.com/feigeCode/navop/releases"><img src="https://img.shields.io/github/downloads/feigeCode/navop/total?style=for-the-badge&color=blue" alt="Downloads" /></a>
    <a href="https://github.com/feigeCode/navop/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/feigeCode/navop/ci.yml?branch=main&style=for-the-badge" alt="CI" /></a>
    <a href="LICENSE-APACHE"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20Supplementary-blue?style=for-the-badge" alt="License" /></a>
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
    <a href="#install">Install</a> ·
    <a href="https://github.com/feigeCode/navop/releases/latest">Latest Release</a> ·
    <a href="#features">Features</a> ·
    <a href="#screenshots">Screenshots</a> ·
    <a href="CONTRIBUTING.md">Contributing</a>
  </p>

  <p>
    <img src="app.png" alt="Navop overview" width="820" />
  </p>
</div>

## What's New in v0.8.0

- **AI Agent and Function Calling** — AI agents can call tools to complete structured tasks, load skills through tools, and use improved resource pools, resource mentions, and catalog refreshes.
- **HTML preview flow** — HTML code blocks can be opened in the browser or rendered in an in-app dialog, keeping chat content readable without intrusive inline previews.
- **Database compare and sync** — improved schema/data compare windows, compare target loading, multi-table sync, and database compare sync stability.
- **Terminal productivity** — added a terminal command history panel, SSH broadcast input across windows, and improved remote shell integration install, uninstall, and environment handling.
- **Connection import** — added an entry for importing connections from other applications, with a refined home sidebar layout.
- **Team entry point** — added a team management entry controlled by feature flags.
- **Extensions and language bundles** — added language bundle extension support with detection and installation.
- **Rendering and font fixes** — fixed font fallback/rendering issues that could cause garbled text, and reduced render-process blocking that could make connection lists and data lists stutter while scrolling.
- **Settings and UI polish** — API key fields now support reveal/hide toggling, input components support local theme styling, and window/selector layouts have been refined.


## Why Navop?

<table>
  <tr>
    <td width="50%">
      <h3>Native desktop, not a browser shell</h3>
      <p>Navop is built with Rust and GPUI for a native desktop experience with GPU-accelerated rendering.</p>
    </td>
    <td width="50%">
      <h3>One workspace for daily ops</h3>
      <p>Database management, SSH terminals, SFTP file transfer, port forwarding, serial connections, local terminals, and remote desktop (RDP/VNC) live in one app.</p>
    </td>
  </tr>
  <tr>
    <td>
      <h3>AI next to your data</h3>
      <p>Use the built-in AI assistant for natural language to SQL, query explanation, BI-style analysis, and chart generation.</p>
    </td>
    <td>
      <h3>Remote work without context switching</h3>
      <p>Open a remote terminal, browse files through SFTP, drag files into the sidebar, and edit remote files with syntax highlighting.</p>
    </td>
  </tr>
</table>

## Features

### Database Workspace

Connect to MySQL, PostgreSQL, SQLite, DuckDB, SQL Server, Oracle, and ClickHouse from a single interface. Network database connections can route through per-connection SOCKS5 or HTTP CONNECT proxies, including authenticated proxies and SSH tunnels reached through a proxy. Browse schemas, tables, columns, indexes, foreign keys, procedures, functions, triggers, and sequences where supported.

Beyond the built-in drivers, Navop ships an extension marketplace that adds database drivers for Dameng DM, KingbaseES, GBase 8s, OceanBase, openGauss, Apache IoTDB, and a pure-Go Oracle driver that runs without Oracle Instant Client. Install the ones you need and they appear alongside the built-in connections.

### SQL Editor & Schema Tools

Work with a SQL editor backed by syntax tooling, schema-aware browsing, table structure editing, query execution, explain support, and ER diagrams. Database compare tools support schema/data comparison, target selection, sync planning, and multi-table synchronization workflows.

### Redis & MongoDB

Use the dedicated Redis viewer for key browsing, value inspection, and cluster connections. Explore MongoDB collections, inspect documents, and run queries from the same workspace.

### SSH, SFTP, Port Forwarding, Serial & Terminal

Open integrated SSH sessions, manage SFTP files, start port forwarding tunnels, connect to serial devices, and keep local terminals in multi-tab sessions. Local terminal profiles support the system shell, PowerShell, Command Prompt, WSL, Git Bash, and custom programs with safely parsed arguments. The terminal also includes command history, SSH broadcast input across windows, remote shell integration management, and an SFTP sidebar with drag-and-drop upload support, path favorites, and quick jumps to frequently used directories.

### Port Forwarding

Create reusable SSH port forwarding connections from existing SSH/SFTP servers. Navop supports local forwarding for services such as databases or internal HTTP endpoints, plus dynamic SOCKS tunnels for routing tools through a remote host.

### Remote File Editing

Edit remote files directly inside Navop with syntax highlighting and autocomplete. No need to open another editor or switch back and forth between terminal and file tools.

### Remote Desktop (RDP & VNC)

Open RDP and VNC sessions through installable remote desktop providers. Each connection can use a SOCKS5 or HTTP CONNECT proxy without requiring a provider protocol upgrade. Connect to Windows machines over RDP, or to any VNC server, and drive the remote desktop from the same workspace where your databases, terminals, and files live.

### Monitoring & Charts

Use built-in server monitoring and native rendered charts to inspect remote machine status and data analysis output.

### AI Assistant

Chat with AI inside the app. Navop supports natural language to SQL, query explanation, BI-style data analysis, chart generation, streaming LLM responses, AI Agent workflows, and Function Calling for tool-based task execution. HTML code blocks can be opened in the browser or previewed in an in-app dialog, and generated terminal commands can be quickly pasted into a terminal session and run.

### Performance & Rendering

Navop uses native GPUI rendering and continues to tune heavy UI paths. Recent releases fixed font fallback/rendering issues that could cause garbled text, and reduced render-process blocking that could make connection lists and data lists stutter while scrolling.

### Sync, Security & i18n

Sync connections and settings across devices with encrypted key storage based on AES-GCM and Ed25519. Navop supports light and dark themes, English, Simplified Chinese, and Traditional Chinese.

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

| Monitoring | SFTP Sidebar |
|:-:|:-:|
| [![Monitoring](monitor.png)](monitor.png) | [![SFTP Sidebar](sftp_sidebar.png)](sftp_sidebar.png) |

| Remote File Editor | ER Diagram |
|:-:|:-:|
| [![Remote File Editor](remote_file_editor.png)](remote_file_editor.png) | [![ER Diagram](er.png)](er.png) |

| Extensions |
|:-:|
| [![Extensions](extension.png)](extension.png) |

## Install

Download the latest build from the [Releases](https://github.com/feigeCode/navop/releases/latest) page.

Release artifacts are currently published by platform:

| Platform | Architecture | Artifact |
|----------|--------------|----------|
| macOS | Apple Silicon, Intel | `.dmg`, `.tar.gz` |
| Linux | x86_64 | `.tar.gz` |
| Windows | x86_64 | `.zip` |

Checksums are published as `sha256sums.txt` in each release.

### macOS Gatekeeper

If macOS blocks the app after installing the DMG with "Apple cannot check it for malicious software", run:

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

### Oracle Support

The built-in Oracle driver requires [Oracle Instant Client](https://www.oracle.com/database/technologies/instant-client/downloads.html) (Basic package). Download the version matching your platform and ensure the libraries are in your library search path. Alternatively, install the pure-Go Oracle driver from the extension marketplace, which has no Instant Client dependency.

## Getting Started

1. Open Navop and create your first database connection.
2. Add an SSH host and open a remote terminal.
3. Create a port forwarding connection from that SSH host when you need a local tunnel or SOCKS proxy.
4. Open SFTP file management to browse remote directories or transfer files.
5. Try Redis key browsing or MongoDB document browsing.
6. Use the AI assistant in SQL or data analysis workflows.

## Build From Source

### Prerequisites

- Rust 2024 edition
- Platform-specific system dependencies

### System Dependencies

**macOS / Linux:**

```bash
./script/bootstrap
```

**Windows (PowerShell):**

```powershell
.\script\install-window.ps1
```

### Run

```bash
cargo run -p main
```

### Development Checks

```bash
# Build
cargo build

# Test
cargo test --all

# Lint
cargo clippy --workspace --all-targets

# Format check
cargo fmt --check
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full development guide.

## Tech Stack

| Category | Technologies |
|----------|--------------|
| UI Framework | [GPUI](https://gpui.rs) |
| Language | Rust |
| Databases | tokio-postgres, mysql_async, rusqlite, tiberius, oracle, clickhouse, duckdb |
| Database extensions | Dameng DM, KingbaseES, GBase 8s, OceanBase, openGauss, Apache IoTDB, pure-Go Oracle |
| Redis / MongoDB | redis, mongodb |
| SSH / SFTP / Port Forwarding | russh, russh-sftp, SOCKS5 over SSH direct-tcpip |
| Remote Desktop | RDP & VNC providers via extension runtime |
| Terminal | alacritty_terminal |
| Text Editing | ropey, tree-sitter, sqlparser |
| AI | llm-connector |
| Encryption | aes-gcm, sha2, ed25519 |
| i18n | rust-i18n |

## FAQ

<details>
<summary><strong>Which databases are supported?</strong></summary>

Navop has built-in database support for MySQL, PostgreSQL, SQLite, DuckDB, SQL Server, Oracle, and ClickHouse, plus dedicated Redis and MongoDB views. The extension marketplace adds Dameng DM, KingbaseES, GBase 8s, OceanBase, openGauss, Apache IoTDB, and a pure-Go Oracle driver, so domestic and specialty databases are covered alongside the mainstream ones.
</details>

<details>
<summary><strong>Does Oracle need extra setup?</strong></summary>

Yes. The built-in Oracle driver requires Oracle Instant Client to be installed and available through your system library search path. You can also install the pure-Go Oracle driver from the extension marketplace, which runs without Instant Client.
</details>

<details>
<summary><strong>Where can I download Navop?</strong></summary>

Use the GitHub [Releases](https://github.com/feigeCode/navop/releases/latest) page. The current release workflow publishes macOS, Linux, and Windows artifacts with checksums.
</details>

<details>
<summary><strong>Is Navop free?</strong></summary>

All features are available without sponsorship. The source is licensed under Apache License 2.0, and distribution or product use is also subject to the Navop Supplementary License.
</details>

<details>
<summary><strong>How do I report bugs or request features?</strong></summary>

Open an issue on [GitHub Issues](https://github.com/feigeCode/navop/issues). For code changes, please read [CONTRIBUTING.md](CONTRIBUTING.md) first.
</details>

## Support

Navop is maintained by one person over the long term. If it saves you time, you can support the project through donations, stars, bug reports, or focused pull requests.

### Donation

Donation is optional and does not unlock or restrict any features. See [DONATE.md](DONATE.md) for WeChat Pay, Alipay, and PayPal options.

### Community Contacts

Official community channels:

- QQ Group: [860670605](https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605)
- WeChat Group: [Join](https://docs.qq.com/doc/DVEFFd2RnSnJLcFBD)

## Credits

ER diagram rendering is based on [ferrum-flow](https://github.com/tu6ge/ferrum-flow.git).

## License

Licensed under [Apache License 2.0](LICENSE-APACHE).

The distribution and use of the Navop application are additionally subject to the [Navop Supplementary License](NAVOP_LICENSE), which adds the following restrictions on top of Apache 2.0:

- No redistribution, resale, or repackaging as a standalone product
- No creating competing products or services based on this software
- No hosting on unauthorized distribution platforms

For licensing inquiries, contact xiaofei.hf@gmail.com.

## Star History

<a href="https://www.star-history.com/?repos=feigeCode%2Fnavop&type=date&logscale=&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/chart?repos=feigeCode/navop&type=date&theme=dark&logscale&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/chart?repos=feigeCode/navop&type=date&logscale&legend=top-left" />
   <img alt="Star History Chart" src="https://api.star-history.com/chart?repos=feigeCode/navop&type=date&logscale&legend=top-left" />
 </picture>
</a>
