> 老仓库 ：<https://github.com/feigeCode/onetcli> · [![OnetCli Stars](https://img.shields.io/github/stars/feigeCode/onetcli?style=flat-square&logo=github&label=OnetCli%20Stars)](https://github.com/feigeCode/onetcli)

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
- 浏览数据库对象，编辑和执行 SQL，查看执行计划，导入导出数据，比较 Schema/Data，并通过 ER 图查看关系；SQL 查询支持无限结果模式与执行中取消。
- 编辑 MySQL 存储过程与函数、PostgreSQL 函数与过程，支持重载例程导航，并可查看表大小、索引数等表信息。
- 查看持久化保存的 SQL 执行历史，使用改进的跨数据库类型映射和目标表匹配进行 Schema/Data Compare，并选择 Oracle Native 或纯 Go 驱动，同时支持 Oracle 11g 查询分页限制。
- 使用专用界面管理 Redis 与 MongoDB。
- 通过 SOCKS5、HTTP CONNECT 代理和 SSH 隧道路由受支持的网络连接。

### 远程连接与运维

- 在可拖拽分屏中使用 SSH 与本地终端，并提供快捷命令、历史记录、广播输入、Shell integration 和终端 AI；SSH 可配置字符集（UTF-8、GBK、Big5、Shift_JIS 等）与终端类型以适配旧环境。
- 使用密码锁定会话，可一键锁定全部会话，或隐藏当前会话输出。
- 录制终端会话并通过只读时间线回放，回放期间会阻止输入与在线操作。
- 支持 Telnet 连接、自动登录脚本和手动凭据覆盖。
- 通过静态历史查看器查看 SSH、串口和本地终端会话日志，支持滚动、文本选择、搜索与 TXT 导出。
- 通过 SFTP 上传下载、目录上传、搜索、收藏、远程编辑、拖拽传输和跨服务器复制文件；并支持 SSH 下的 ZMODEM 文件传输。
- 支持导入 SecureCRT 会话与快捷命令，并在连接侧边栏批量管理连接。
- 创建可复用的本地、远程（`ssh -R`）端口转发与动态 SOCKS 隧道。
- SSH/SFTP 主机密钥变更时会展示新旧指纹并要求明确确认，且可按连接启用旧版 SSH 算法。
- 使用串口连接、服务器监控，以及通过扩展 provider 提供的 RDP/VNC 远程桌面。

### 编辑、AI 与扩展

- 编辑本地 Markdown 笔记，支持语法高亮、Mermaid、数学公式、相对媒体资源，并可通过沙箱化 WASM 导出器生成 HTML、PDF 或 DOCX。
- 编辑时可放大查看 Mermaid 图和数学公式，并在源码与预览之间切换。
- 使用 AI 生成和解释 SQL、分析数据、生成图表、辅助终端操作、调用工具和运行 Agent 工作流。
- 通过 ACP 扩展接入 Codex、Claude Code 和 OpenCode 等外部 Agent。
- 使用 Agent Hub 在同一工作区查看终端 Agent、项目文件、Git 分支、变更列表和并排 Diff。
- 通过扩展市场安装数据库驱动、远程桌面 provider、文档渲染器和其他能力。

### 原生桌面体验

- 基于 GPUI 的原生界面与 GPU 加速渲染。
- 支持亮色、深色、跟随系统模式，可导入应用与终端主题，并配置强调色和窗口透明度。
- 支持 English、简体中文和繁体中文界面。
- 支持可复用的钥匙串引用，并加密同步不同设备上的个人连接、凭据与设置。

## Public MCP、Navop CLI 与 Agent Skill

Navop 可将选定的宿主权威工具开放给外部 Codex、Claude、MCP 客户端和自动化程序。请在 **设置 > 通用 > MCP > MCP Server** 中开启服务、选择权限档位，并在 **Tool Exposure** 中只启用需要的工具组。

runtime 只监听动态 loopback 端口，并使用 Navop 写入用户专属 discovery 文件的 token 验证客户端。实时工具与 Schema、Tool Exposure、权限、审批、资源 ID、会话、结果和审计始终由正在运行的 Navop 应用控制。CLI 与 Skill 本身不实现 SSH、SFTP、终端、数据库、Redis 或 MongoDB 的业务逻辑。

终端型 Agent 需要安装 Node.js 20+、[`@navop/cli`](https://github.com/feigeCode/navop-mcp) 及其内置的 Navop Skill：

```bash
navop --version
npm install -g @navop/cli@latest
navop --version

# 为 Codex 安装 Skill；Agents 兼容客户端可改用 --target agents
navop skill install --target codex --scope user
```

Skill 只在上下文中保留紧凑工作流，并按需发现命令与实时 Schema。Agent 发起的实际操作命令必须包含 `--json`，`--help` 仅用于发现语法；先检查 runtime 状态，再发现当前命令和工具：

```bash
navop status --json
navop --help
navop tool list --json
navop tool schema <tool-name> --json
navop db query --help
navop db exec --help
```

从 `navop status --json` 中读取 `permissionMode`、`availableTools`、`toolGroups`、`disabledToolGroups` 与 `guidance`。运行中宿主返回的 `tools/list`、`navop.runtime_status` 结果与实时 Schema 才是权威依据；不能因为 CLI 提供了某个便捷命令，就假定对应能力已经开放。

优先使用 `navop --help` 中提供的领域命令。SQL 只读语句使用 `navop db query`；DDL、DML、脚本及其他可能写入的 SQL 使用 `navop db exec`。只有没有领域命令能够表达实时 Schema 时，才使用底层宿主工具调用：

```bash
navop tool call <tool-name> --arguments '<json-object-matching-live-schema>' --json
```

不要猜测工具名称、参数、资源 ID 或会话 ID，只能使用当前 Navop 实时返回的值。必须遵守 Navop 的权限与审批结果；写操作超时或连接中断后，不要自动重试，因为操作结果可能已经发生但状态未知。

实际可用能力取决于正在运行的 Navop、已打开的资源、启用的 Tool Exposure 工具组和权限档位：

| 权限档位 | 行为 |
| --- | --- |
| 安全 / `deny` | 允许只读发现，拒绝写操作 |
| 确认 / `ask` | 写操作需要在 Navop UI 中审批 |
| 自动 / `allow` | 自动执行写操作，但破坏性操作仍需明确意图 |

独立的 [`@navop/mcp`](https://github.com/feigeCode/navop-mcp) 仅用于为原生 MCP 客户端提供 stdio bridge：

```bash
npx -y @navop/mcp@latest
```

安装、更新、命令参考和客户端配置请查看 [Navop MCP 与 CLI 仓库](https://github.com/feigeCode/navop-mcp)。

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
| Linux | x86_64 | `.tar.gz`、`.deb`、`.rpm`、`.AppImage`、`-portable.tar.gz` |
| Linux | ARM64 | `.tar.gz`、`-portable.tar.gz` |
| Windows | x86_64、x86（32 位） | `.msi`、`.exe`、`.zip`、`-portable.zip` |

Windows `.msi` 和 `.exe` 都是中英双语的当前用户安装程序，使用默认位置时不需要管理员权限；EXE 安装包封装的是同一套 MSI 安装流程。普通 `.zip` 是免安装版，仍使用正常的 Windows 用户数据目录，并支持记住主密钥后自动解锁。只有需要把应用数据放在程序同级目录时才应下载 `-portable.zip`。便携版（Linux 与 Windows）默认每次启动都要求输入主密钥。用户也可以在设置中明确选择把可自动恢复的加密主密钥副本保存到 `data/state/key_storage`，但该加密使用程序内置密钥，不具备设备绑定保护；任何同时获得应用程序和完整 `data` 目录的人都可能恢复主密钥。

> **从 v0.10.1 或更早版本的 Windows ZIP 升级：**这些历史 ZIP 已启用便携模式。请下载新的 `-portable.zip`，解压到新目录，并把原目录中的整个 `data` 复制进去。如果把新的普通 `.zip` 解压到另一个目录，或改用 MSI/EXE 安装版，Navop 会使用正常的 Windows 用户数据目录，而且不会自动迁移便携数据。原有连接和设置可能因此看起来消失，但旧便携目录中的数据并未被删除。

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
