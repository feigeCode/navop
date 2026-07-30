<div align="center">
  <p><img src="resources/navop-icon.png" alt="Navop" width="120" /></p>
  <h1>Navop</h1>
  <p><strong>数据库、SSH、SFTP、终端、远程桌面、监控与 AI 一体化的原生桌面工作台。</strong></p>
  <p>基于 <a href="https://gpui.rs">GPUI</a> 与 Rust 构建 · GPU 加速渲染</p>

  <p>
    <a href="https://github.com/feigeCode/navop/releases"><img src="https://img.shields.io/github/downloads/feigeCode/navop/total?style=for-the-badge&color=blue" alt="下载量" /></a>
    <a href="https://github.com/feigeCode/navop/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/feigeCode/navop/ci.yml?branch=main&style=for-the-badge" alt="CI" /></a>
    <a href="#许可证"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20supplementary%20terms-blue?style=for-the-badge" alt="许可证：Apache-2.0 与补充协议" /></a>
  </p>

  <p>
    <a href="README.md">English</a> ·
    <a href="#功能特性">功能特性</a> ·
    <a href="#应用截图">应用截图</a> ·
    <a href="#安装">安装</a> ·
    <a href="https://github.com/feigeCode/navop/releases/latest">最新版本</a> ·
    <a href="CONTRIBUTING.md">参与贡献</a>
  </p>

  <p><img src="app1.png" alt="Navop 概览" width="820" /></p>
</div>

## 功能特性

### 数据库与数据工具

- 连接 MySQL、PostgreSQL、SQLite、DuckDB、SQL Server、Oracle 和 ClickHouse。
- 通过扩展安装达梦 DM、金仓 KingbaseES、GBase 8s、OceanBase、openGauss、Apache IoTDB，以及无需 Instant Client 的 Oracle 驱动。
- 浏览数据库对象，编辑和执行 SQL，查看执行计划，导入导出数据，比较 Schema/Data，并通过 ER 图查看关系。
- 使用专用界面管理 Redis 与 MongoDB。
- 通过 SOCKS5、HTTP CONNECT 代理和 SSH 隧道路由受支持的网络连接。

### 远程连接与运维

- 在可拖拽分屏中使用 SSH 与本地终端，并提供快捷命令、历史记录、广播输入、Shell integration 和终端 AI。
- 通过 SFTP 上传下载、搜索、收藏、远程编辑、拖拽传输和跨服务器复制文件。
- 创建可复用的本地端口转发与动态 SOCKS 隧道。
- 使用串口连接、服务器监控，以及通过扩展 provider 提供的 RDP/VNC 远程桌面。

### 编辑、AI 与扩展

- 编辑本地 Markdown 笔记，支持语法高亮、Mermaid、数学公式、相对媒体资源，并可通过沙箱化 WASM 导出器生成 HTML、PDF 或 DOCX。
- 使用 AI 生成和解释 SQL、分析数据、生成图表、辅助终端操作、调用工具和运行 Agent 工作流。
- 通过 ACP 扩展接入 Codex、Claude Code 和 OpenCode 等外部 Agent。
- 使用 Agent Hub 在同一工作区查看终端 Agent、项目文件、Git 分支、变更列表和并排 Diff。
- 通过扩展市场安装数据库驱动、远程桌面 provider、文档渲染器和其他能力。

### 原生桌面体验

- 基于 GPUI 的原生界面与 GPU 加速渲染。
- 支持亮色、深色、跟随系统模式，可导入应用与终端主题，并配置强调色和窗口透明度。
- 支持 English、简体中文和繁体中文界面。
- 加密同步不同设备上的连接与设置。

## Public MCP、Navop CLI 与 Agent Skill

Navop 可将选定的宿主权威工具开放给外部 Codex、Claude、MCP 客户端和自动化程序。请在 **设置 > 通用 > MCP > MCP Server** 中开启服务、选择权限档位，并在 **Tool Exposure** 中只启用需要的工具组。

runtime 只监听动态 loopback 端口，并使用 Navop 写入用户专属 discovery 文件的 token 验证客户端。工具 Schema、权限、审批、连接、会话、结果和审计始终由正在运行的 Navop 应用控制。

终端型 Agent 可使用独立发布的 [`@navop/cli`](https://github.com/feigeCode/navop-mcp)，MCP 客户端可通过 [`@navop/mcp`](https://github.com/feigeCode/navop-mcp) stdio bridge 接入：

```bash
npm install -g @navop/cli@latest
navop status --json
navop tools --json
navop schema <tool-name> --json
navop call <tool-name> --arguments '<json-object>' --json

# 无需全局安装即可启动 MCP stdio bridge
npx -y @navop/mcp@latest
```

CLI 与 Agent Skill 只在需要时发现命令和实时 Schema，减少重复的工具上下文。目前的能力范围包括 runtime 发现、SSH、可见终端、SQL 数据库、Redis、MongoDB、SFTP、连接、工作区与宿主注册函数。实际可用能力始终取决于 Navop 运行状态、Tool Exposure 配置和权限档位：

| 权限档位 | 行为 |
| --- | --- |
| 安全 / `deny` | 允许只读发现，拒绝写操作 |
| 确认 / `ask` | 写操作需要在 Navop UI 中审批 |
| 自动 / `allow` | 自动执行写操作，但破坏性操作仍需明确意图 |

客户端配置与命令文档请查看 [Navop MCP 与 CLI 仓库](https://github.com/feigeCode/navop-mcp)。

## 应用截图

| 数据库 | SSH |
|:-:|:-:|
| [![数据库](database.png)](database.png) | [![SSH](ssh.png)](ssh.png) |

| SFTP | Redis |
|:-:|:-:|
| [![SFTP](sftp.png)](sftp.png) | [![Redis](redis.png)](redis.png) |

| MongoDB | AI 对话 |
|:-:|:-:|
| [![MongoDB](mongodb.png)](mongodb.png) | [![AI 对话](chatdb.png)](chatdb.png) |

| Agent Hub | 扩展市场 |
|:-:|:-:|
| [![Agent Hub](agent_hub.png)](agent_hub.png) | [![扩展市场](extension.png)](extension.png) |

| 远程文件编辑 | ER 图 |
|:-:|:-:|
| [![远程文件编辑](remote_file_editor.png)](remote_file_editor.png) | [![ER 图](er.png)](er.png) |

| 服务器监控 | 主题 |
|:-:|:-:|
| [![服务器监控](monitor.png)](monitor.png) | [![主题](theme.png)](theme.png) |

## 安装

请从 [GitHub Releases](https://github.com/feigeCode/navop/releases/latest) 下载最新版本。每个版本都包含用于校验文件的 `sha256sums.txt`。

| 平台 | 架构 | 产物 |
| --- | --- | --- |
| macOS | Apple Silicon、Intel | `.dmg`、`.tar.gz` |
| Linux | x86_64 | `.tar.gz`、`.deb`、`.rpm`、`.AppImage` |
| Linux | ARM64 | `.tar.gz` |
| Windows | x86_64 | `.msi`、`.zip` |

Windows MSI 是中英双语的当前用户安装程序，使用默认位置时不需要管理员权限；ZIP 压缩包可用于便携运行。

### macOS Gatekeeper

如果安装 DMG 后 macOS 提示“Apple 无法检查其是否包含恶意软件”，请执行：

```bash
sudo xattr -rd com.apple.quarantine /Applications/Navop.app
```

### Oracle

内置 Oracle 驱动需要安装 [Oracle Instant Client](https://www.oracle.com/database/technologies/instant-client/downloads.html)。如需免 Instant Client 使用 Oracle，可从扩展市场安装纯 Go Oracle 驱动。

## 从源码构建

Navop 使用 Rust 2024 edition，并需要安装各平台的系统依赖。

```bash
# 安装 Linux 依赖
./script/bootstrap

# 运行应用
cargo run -p main
```

Windows 请在 PowerShell 中安装依赖：

```powershell
.\script\install-window.ps1
```

常用开发检查：

```bash
cargo build
cargo test --all
cargo clippy --workspace --all-targets
cargo fmt --check
```

完整开发指南请参阅 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 社区与支持

Navop 由个人长期维护。Star、聚焦的小型 PR、Bug 报告和捐赠都有助于项目持续发展。

- [反馈 Bug 或提出功能建议](https://github.com/feigeCode/navop/issues)
- QQ 群：[860670605](https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605)
- 微信群：[加入](https://docs.qq.com/doc/DVEFFd2RnSnJLcFBD)
- 自愿捐赠：[DONATE_CN.md](DONATE_CN.md)
- 旧版 OnetCli 仓库：[feigeCode/onetcli](https://github.com/feigeCode/onetcli)

## 致谢

ER 图渲染基于 [ferrum-flow](https://github.com/tu6ge/ferrum-flow.git)。

## 许可证

Navop 源代码基于 [Apache License 2.0](LICENSE-APACHE) 开源。Navop 自有代码还须遵守 [Navop 补充协议](NAVOP_LICENSE)，其中包含对二次分发、转售、竞争性产品或服务以及未经授权分发平台的限制。第三方组件继续适用其各自的许可证。

如有许可证与版权相关问题，请联系 xiaofei.hf@gmail.com。
