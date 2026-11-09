# Navop 使用说明

Navop 现在是同一套原生内核上的三个应用：全量的 **Navop**、面向远程运维的 **Navop Shell**，以及面向数据工作的 **Navop Data**。三者以同一个版本号发布，可以并排安装，共享 AI、Notes、团队同步、扩展市场和主题等底座，区别在于覆盖的工作范围。

## 三个应用

| 应用 | 适合谁 | 包含的能力 |
| --- | --- | --- |
| **Navop**（全量版） | 数据库和服务器都要管，希望一个应用覆盖全部工作 | 数据库（PostgreSQL、MySQL、MariaDB、SQLite、SQL Server、Oracle、ClickHouse、达梦、人大金仓、OceanBase、OpenGauss 等）、Redis、MongoDB、SSH/SFTP、本地与远程终端、端口转发、远程桌面 RDP/VNC、串口/Telnet、SQL 编辑器、ER 图、数据迁移、监控、Notes、AI 工作台、团队同步、扩展市场、Public MCP |
| **Navop Shell**（远程运维） | 主要工作是登录服务器、传文件、开远程桌面 | SSH/SFTP、本地与远程终端、端口转发、远程桌面 RDP/VNC、串口/Telnet；共享 AI、Notes、团队与扩展体系 |
| **Navop Data**（数据工具） | 主要工作是连数据库、写 SQL、看 Redis/MongoDB | SQL 工作区、Redis、MongoDB、ER 图、数据迁移；共享 AI、Notes、团队与扩展体系 |

## 共享底座

三个应用使用同一套原生内核，并共享以下能力：

- AI 工作台、Notes、团队同步与个人同步；
- 扩展市场，以及已经安装的扩展体系；
- 主题、快捷键、tab 恢复、托盘、单实例、自动更新、Public MCP。

三者以同一版本号发布，可以并排安装；数据目录与钥匙串各自独立，同时安装不会互相覆盖配置。只有全量 Navop 注册 `.db`、`.duckdb`、`.md`、`.cast` 文件关联；Scoop 与 Flatpak 目前也只覆盖全量 Navop。

## 怎么选

- **数据库和服务器都要管**，或者需要文件关联、监控与数据迁移：选 Navop。
- **只做远程运维**，日常是 SSH、SFTP、终端、端口转发和远程桌面：选 Navop Shell。
- **只做数据工作**，日常是 SQL、Redis、MongoDB、ER 图和数据迁移：选 Navop Data。
- **拿不准**：先装 Navop，它的能力是三个里最全的；之后仍可以并排安装独立产品，两者互不冲突。

## 当前版本：v0.23.0

三个应用当前同为 v0.23.0。可以从[官网下载中心](https://navop.dev/zh-CN/extensions)、[GitHub Releases](https://github.com/feigeCode/navop/releases) 或 [CNB](https://cnb.cool/navop-dev/navop/-/releases) 获取安装包。产品形态上的主要变化：

- Navop 从单个全量应用扩展为同一套原生内核上的三个应用，共享 AI、Notes、团队、扩展与更新体系。
- 安装包以 `navop-`、`navop-shell-`、`navop-data-` 前缀区分，三者同版本发布，可以并排安装。
- 数据库与远程运维能力按产品裁剪；数据目录、钥匙串和应用内更新通道各自独立。
- 只有全量 Navop 提供 `.db`、`.duckdb`、`.md`、`.cast` 文件关联，以及 Linux 的 `.deb`、`.rpm`、AppImage 和图形依赖包。

安装包矩阵与平台差异见[安装与更新](./guide/install-update)。

## 从这里开始

- [快速开始：先选应用，再完成第一个任务](./guide/quick-start)
- [安装与更新：安装包、平台差异与并排安装](./guide/install-update)
- [首页、工作区与连接管理](./guide/workspace-connections)（三个应用）

## 按任务查找

- 数据库连接、SQL、导入导出与 Schema 工具（Navop / Navop Data）：[数据库连接](./guide/database-connections)
- SQL 编辑器、事务与查询结果（Navop / Navop Data）：[SQL 编辑器](./guide/sql-editor)
- SSH、SFTP、端口转发与 Agent Hub（Navop / Navop Shell）：[SSH 与终端](./guide/ssh-terminal)
- FTP 与 FTPS 远程文件（Navop）：[FTP 与远程文件](./guide/ftp-remote-files)
- 远程桌面、串口与服务器监控（远程桌面与串口：Navop / Navop Shell；服务器监控：Navop）：[远程接入](./guide/remote-access)
- 原生资源工作台（Docker 等，Navop）：[资源工作台](./guide/resource-workbench)
- Notes Markdown 预览与源码编辑（三个应用）：[Notes](./guide/notes)
- AI 工作台、Navop Skill 与 Public MCP（三个应用，Public MCP 按产品分别发现）：[AI 与 MCP](./guide/ai-workbench)
- 团队同步与安全（三个应用）：[团队同步](./guide/teams-sync-security)
- 设置与疑难排解（三个应用）：[设置与疑难排解](./guide/settings-shortcuts)

文档内容随三个应用持续更新。涉及生产数据库、远程服务器、写入 SQL、文件覆盖和批量同步时，请先确认当前环境和操作范围。
