# Navop 使用说明

Navop 是一个**基于 Rust 与 GPUI 原生构建**的桌面工作台：没有 WebView，也没有 Electron，界面、终端、表格和图表都由 GPU 加速渲染。它把数据库、服务器、远程桌面和 AI Agent 放进同一个窗口，覆盖从连接管理、SQL 开发、文件传输到远程排查的日常工作。

## 为什么是 Navop

- **原生性能**：GPUI 直接绘制界面，多连接、终端分屏和大结果集不会因为浏览器布局而变慢。
- **全栈 Rust**：数据库驱动、SSH/SFTP、终端模拟、AI 运行时与界面共享同一套 Rust 代码。
- **一个窗口完成工作**：SQL 工作区、终端、文件管理、远程桌面和 AI 侧边栏可以并排组织，不必在多个工具之间切换。
- **可扩展**：数据库驱动、远程桌面、文档渲染和资源工作台都可以通过 WASM 扩展安装，无需重启应用。
- **跨平台一致**：macOS（arm64/x64）、Windows（x64/win32）和 Linux（x64/arm64）使用同一套界面与能力。

## Navop 支持什么

- **数据库**：MySQL、PostgreSQL、MariaDB、SQLite、DuckDB、SQL Server、Oracle、ClickHouse、TDengine，以及达梦、金仓、GBase、OceanBase、openGauss、IoTDB、Oscar 等扩展驱动。
- **数据工具**：对象浏览、SQL 编辑器与执行计划、导入导出、结构与数据比较、ER 图、表数据批量编辑。
- **Redis / MongoDB / MQTT**：Key 与值管理、集合与文档查询、订阅与发布。
- **SSH 与终端**：本地/远程终端、原生分屏、广播输入、快捷命令、Shell 集成、会话录制回放与日志搜索。
- **文件传输**：SFTP、FTP、FTPS，拖拽上传下载、远程文件编辑、ZMODEM、服务器间复制。
- **远程访问**：本地/远程/SOCKS 端口转发、X11、RDP、VNC、串口、Telnet、主机密钥管理与 SecureCRT 导入。
- **AI 与自动化**：SQL 生成与解释、结果分析、图表、终端辅助、ACP Agent（Codex / Claude Code / OpenCode）、Agent Hub。
- **协作与扩展**：Notes、团队与共享连接、加密同步、扩展市场、Public MCP、Navop CLI 与 Agent Skill。

## 安装与版本

从[官网下载中心](https://navop.dev/zh-CN/extensions)、[GitHub Releases](https://github.com/feigeCode/navop/releases) 或 [CNB](https://cnb.cool/navop-dev/navop/-/releases) 获取最新安装包。安装包命名遵循 `navop-<version>-<label>.<ext>`，支持 macOS、Windows 和 Linux；每次发布都附带校验文件。

同一份源码还会构建两个更小的版本：**Navop Shell**（SSH、终端与远程桌面）和 **Navop Data**（数据库与数据浏览）。它们与 Navop 共享原生外壳、AI、同步与扩展体系，适合只需要部分能力的场景；大多数用户直接使用 Navop 即可。

安装包矩阵、平台差异与升级说明见[安装与更新](./guide/install-update)。

## 从这里开始

- [快速开始：完成第一个连接与任务](./guide/quick-start)
- [安装与更新：安装包、平台差异与数据目录](./guide/install-update)
- [首页、工作区与连接管理](./guide/workspace-connections)

## 按任务查找

- 数据库连接、SQL、导入导出与 Schema 工具：[数据库连接](./guide/database-connections)
- SQL 编辑器、事务与查询结果：[SQL 编辑器](./guide/sql-editor)
- SSH、SFTP、端口转发与 Agent Hub：[SSH 与终端](./guide/ssh-terminal)
- FTP 与 FTPS 远程文件：[FTP 与远程文件](./guide/ftp-remote-files)
- 远程桌面、串口与服务器监控：[远程接入](./guide/remote-access)
- 原生资源工作台（Docker 等）：[资源工作台](./guide/resource-workbench)
- Notes Markdown 预览与源码编辑：[Notes](./guide/notes)
- AI 工作台、Navop Skill 与 Public MCP：[AI 与 MCP](./guide/ai-workbench)
- 团队同步与安全：[团队同步](./guide/teams-sync-security)
- 设置与疑难排解：[设置与疑难排解](./guide/settings-shortcuts)

文档内容随 Navop 持续更新。涉及生产数据库、远程服务器、写入 SQL、文件覆盖和批量同步时，请先确认当前环境和操作范围。
