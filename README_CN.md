
<div align="center">
  <p><img src="resources/navop-icon.png" alt="Navop" width="120" /></p>
  <h1>Navop</h1>
  <p><strong>数据库、SSH、SFTP、终端、远程桌面、监控与 AI 一体化的原生桌面工作台。</strong></p>
  <p>基于 <a href="https://gpui.rs">GPUI</a> 与 Rust 构建 · GPU 加速渲染</p>

  <p>
    <a href="https://github.com/feigeCode/navop/releases"><img src="https://img.shields.io/github/downloads/feigeCode/navop/total?style=for-the-badge&color=blue" alt="下载量" /></a>
    <a href="https://github.com/feigeCode/navop/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/feigeCode/navop/ci.yml?branch=dev&style=for-the-badge" alt="CI" /></a>
    <a href="#许可证"><img src="https://img.shields.io/badge/license-Apache--2.0%20%2B%20supplementary%20terms-blue?style=for-the-badge" alt="许可证：Apache-2.0 与补充协议" /></a>
    <a href="https://qm.qq.com/cgi-bin/qm/qr?k=&group_code=860670605"><img src="https://img.shields.io/badge/QQ%20Group-860670605-EB1923?style=for-the-badge&logo=tencentqq&logoColor=white" alt="QQ 群 860670605" /></a>
    <a href="https://docs.qq.com/doc/DVEFFd2RnSnJLcFBD"><img src="https://img.shields.io/badge/WeChat%20Group-Join-07C160?style=for-the-badge&logo=wechat&logoColor=white" alt="加入微信群" /></a>
  </p>

  <p>
    <img src="https://img.shields.io/badge/MySQL-4479A1?logo=mysql&logoColor=white" alt="MySQL" />
    <img src="https://img.shields.io/badge/PostgreSQL-4169E1?logo=postgresql&logoColor=white" alt="PostgreSQL" />
    <img src="https://img.shields.io/badge/SQLite-003B57?logo=sqlite&logoColor=white" alt="SQLite" />
    <img src="https://img.shields.io/badge/DuckDB-FFF000?logo=duckdb&logoColor=black" alt="DuckDB" />
    <img src="https://img.shields.io/badge/ClickHouse-FFCC01?logo=clickhouse&logoColor=black" alt="ClickHouse" />
    <img src="https://img.shields.io/badge/SQL%20Server-CC2927?logo=microsoftsqlserver&logoColor=white" alt="SQL Server" />
    <img src="https://img.shields.io/badge/Oracle-F80000?logo=oracle&logoColor=white" alt="Oracle" />
    <img src="https://img.shields.io/badge/Dameng%20DM-C71D23" alt="达梦 DM" />
    <img src="https://img.shields.io/badge/KingbaseES-005BAC" alt="金仓 KingbaseES" />
    <img src="https://img.shields.io/badge/GBase%208s-1E73BE" alt="GBase 8s" />
    <img src="https://img.shields.io/badge/OceanBase-1B9A8C" alt="OceanBase" />
    <img src="https://img.shields.io/badge/openGauss-005EB8" alt="openGauss" />
    <img src="https://img.shields.io/badge/Apache%20IoTDB-1B3A6B?logo=apache&logoColor=white" alt="Apache IoTDB" />
    <img src="https://img.shields.io/badge/Redis-DC382D?logo=redis&logoColor=white" alt="Redis" />
    <img src="https://img.shields.io/badge/MongoDB-47A248?logo=mongodb&logoColor=white" alt="MongoDB" />
    <img src="https://img.shields.io/badge/SSH-111827?logo=gnubash&logoColor=white" alt="SSH" />
    <img src="https://img.shields.io/badge/SFTP-2563EB?logo=filezilla&logoColor=white" alt="SFTP" />
    <img src="https://img.shields.io/badge/Port%20Forwarding-0F766E" alt="端口转发" />
    <img src="https://img.shields.io/badge/RDP-0078D4" alt="RDP" />
    <img src="https://img.shields.io/badge/VNC-5C2D91" alt="VNC" />
  </p>

  <p>
    <a href="README.md">English</a> ·
    <a href="https://docs.navop.dev/">使用文档</a> ·
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

- 内置支持 MySQL、PostgreSQL、SQLite、DuckDB、SQL Server、Oracle 和 ClickHouse；通过扩展安装达梦 DM、金仓 KingbaseES、GBase 8s、OceanBase、openGauss、Apache IoTDB 和神通 Oscar 驱动。
- 浏览数据库对象，编辑和执行 SQL 并查看执行计划，导入导出数据，比较 Schema/Data，并通过 ER 图查看关系。
- 专用的 Redis 与 MongoDB 界面、持久化 SQL 执行历史、存储过程与函数编辑，以及代理和 SSH 隧道路由。

### 远程连接与运维

- 可拖拽分屏的 SSH 与本地终端，支持快捷命令、广播输入、Shell integration、会话锁定、会话录制回放和会话日志；同时支持 Telnet 与串口连接。
- 通过 SFTP 上传下载、搜索、收藏、远程编辑、拖拽传输、ZMODEM 传输和跨服务器复制管理远程文件。
- 可复用的本地、远程（`ssh -R`）与动态 SOCKS 端口转发；X11 转发；主机密钥变更指纹确认；可按连接启用旧版 SSH 算法。
- 导入 SecureCRT 会话、服务器监控，以及通过扩展 provider 提供的 RDP/VNC 远程桌面。

### 编辑、AI 与扩展

- 本地 Markdown 笔记，支持 Mermaid 图、数学公式，并可导出为 HTML、PDF 或 DOCX。
- 使用 AI 生成和解释 SQL、分析数据、生成图表、辅助终端操作、调用工具和运行 Agent 工作流；通过 ACP 接入 Codex、Claude Code 和 OpenCode 等外部 Agent。
- Agent Hub 在同一工作区查看终端 Agent、项目文件、Git 分支、变更列表和并排 Diff；扩展市场提供数据库驱动、远程桌面 provider、文档渲染器、连接导入器和外部编辑器。官方扩展在 [navop-extensions](https://github.com/feigeCode/navop-extensions) 仓库独立构建发布。

### 原生桌面体验

- 基于 GPUI 的原生界面与 GPU 加速渲染；亮色、深色、跟随系统模式，可导入主题，并配置强调色和窗口透明度。
- 支持 English、简体中文和繁体中文界面。
- 加密同步不同设备上的个人连接、凭据与设置。

## Public MCP、Navop CLI 与 Agent Skill

Navop 可将选定的宿主权威工具开放给外部 Codex、Claude、MCP 客户端和自动化程序。请在 **设置 > 通用 > MCP > MCP Server** 中开启服务、选择权限档位（安全 / 确认 / 自动），并在 **Tool Exposure** 中只启用需要的工具组。runtime 只监听动态 loopback 端口，并使用用户专属 discovery token 验证客户端；实时工具、Schema、权限、审批和审计始终由正在运行的 Navop 控制。

终端型 Agent 需要安装 [`@navop/cli`](https://github.com/feigeCode/navop-mcp) 及其内置的 Navop Skill：

```bash
npm install -g @navop/cli@latest

# 为 Codex 安装 Skill；Agents 兼容客户端可改用 --target agents
navop skill install --target codex --scope user
```

独立的 [`@navop/mcp`](https://github.com/feigeCode/navop-mcp) 仅用于为原生 MCP 客户端提供 stdio bridge（`npx -y @navop/mcp@latest`）。完整命令参考和客户端配置请查看 [Navop MCP 与 CLI 仓库](https://github.com/feigeCode/navop-mcp)与 [Public MCP 使用指南](https://docs.navop.dev/guide/public-mcp)。

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

请从 [GitHub Releases](https://github.com/feigeCode/navop/releases/latest) 下载最新版本。每个版本都包含用于校验文件的 `sha256sums.txt`。安装包覆盖 macOS（DMG 与 tar.gz，Apple Silicon / Intel）、Windows（MSI 与 EXE 安装版，以及普通与便携 ZIP）和 Linux（tar.gz、deb、rpm、AppImage），命名遵循 `navop-<version>-<平台>-<架构>.<扩展名>` 规则。

Navop 也已在 [FlatPark](https://flatpark.org/zh-Hans/apps/dev.navop.Navop/) 上架社区 Flatpak 软件包：

```bash
flatpak --user remote-add --if-not-exists flatpark https://dl.flatpark.org/flatpark.flatpakrepo
flatpak --user install flatpark dev.navop.Navop
```

如果 macOS 提示“Apple 无法检查其是否包含恶意软件”，请执行 `sudo xattr -rd com.apple.quarantine /Applications/Navop.app`。

完整的安装包选择表、Windows 便携模式说明、v0.10.1 及更早版本 ZIP 的升级迁移，以及 Oracle Instant Client / 纯 Go 驱动说明，请参阅[安装与更新指南](https://docs.navop.dev/guide/install-update)。

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

## Star History

<a href="https://star-history.dera.page/#feigeCode/navop&type=date&logscale=&legend=top-left">
 <picture>
   <source media="(prefers-color-scheme: dark)" srcset="https://star-history.dera.page/svg?repos=feigeCode/navop&type=date&theme=dark&logscale&legend=top-left" />
   <source media="(prefers-color-scheme: light)" srcset="https://star-history.dera.page/svg?repos=feigeCode/navop&type=date&logscale&legend=top-left" />
   <img alt="Star History Chart" src="https://star-history.dera.page/svg?repos=feigeCode/navop&type=date&logscale&legend=top-left" />
 </picture>
</a>

## 致谢

ER 图渲染基于 [ferrum-flow](https://github.com/tu6ge/ferrum-flow.git)。

## 许可证

Navop 源代码基于 [Apache License 2.0](LICENSE-APACHE) 开源。Navop 自有代码还须遵守 [Navop 补充协议](NAVOP_LICENSE)，该协议允许通过免费分发渠道（如 GitHub Releases、Flatpak 仓库和免费应用商店）免费分发，但禁止商业转售、收取费用、竞争性产品或服务以及付费分发平台。第三方组件继续适用其各自的许可证。

如有许可证与版权相关问题，请联系 xiaofei.hf@gmail.com。
