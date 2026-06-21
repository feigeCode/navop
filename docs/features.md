---
title: 功能
description: 了解 OnetCli 如何用纯 Rust、GPUI 和 GPU 渲染整合数据库、Redis、MongoDB、SSH/SFTP、终端与 AI 工作流
---

# 功能

OnetCli 是一个纯 Rust 构建的高性能一体化运维工作台。它不是 WebView 套壳，而是基于 GPUI 构建的原生桌面应用，用一套工作区收拢数据库、Redis、MongoDB、SSH/SFTP、终端、远程桌面和 AI 辅助能力。

![OnetCli 主应用工作台](/screenshots/app.png)

## 原生桌面性能

OnetCli 走 GPUI 原生界面路线，面向长时间打开、高频切换连接和处理大量工程上下文的使用方式设计。

| 能力 | 说明 |
| --- | --- |
| 纯 Rust 技术栈 | 界面、终端、数据库连接、远程能力和应用逻辑都更贴近原生桌面体验。 |
| GPUI 界面 | 沿用 Zed 生态的高性能 UI 路线，避免 WebView 套壳带来的额外资源开销。 |
| GPU 渲染 | macOS 使用 Metal，Linux 使用 Vulkan，让复杂工作台界面保持低延迟交互。 |
| 跨平台 | 面向 macOS、Windows、Linux 使用场景。 |

## 数据库管理

OnetCli 将多种数据库连接放进同一个桌面工作台，适合日常查询、结构浏览、数据检查和问题排查。

明确支持或在仓库中提到的数据库能力包括：

- PostgreSQL
- MySQL
- SQLite
- SQL Server
- Oracle
- ClickHouse
- DuckDB

通过扩展市场还可按需安装国产与特色数据库驱动：

- 达梦 DM
- 金仓 KingbaseES
- 南大通用 GBase 8s
- OceanBase
- openGauss
- Apache IoTDB
- 纯 Go Oracle（无需 Oracle Instant Client）

- Redis
- MongoDB

![数据库对象浏览与 SQL 助手](/screenshots/database.png)

## Redis 工作台

Redis 不是附属入口，而是独立工作流。OnetCli 提供 Redis Key 浏览、值查看、缓存排查和集群场景支持，适合开发调试、线上缓存确认和运维排查。

![Redis 管理界面](/screenshots/redis.png)

## MongoDB Explorer

MongoDB 能力覆盖集合浏览、文档查看和查询操作。关系型数据库与 NoSQL 数据源可以在同一个客户端内处理，减少工具切换。

![MongoDB 管理界面](/screenshots/mongodb.png)

## SSH、SFTP 与终端

OnetCli 支持 SSH 远程终端、SFTP 文件管理和本地终端。远程连接、文件传输、脚本执行和日志排查可以沿用同一套上下文。

![SSH 远程终端与 AI 助手](/screenshots/ssh.png)

### SFTP 文件管理

SFTP 能力覆盖远程目录浏览、文件传输和终端侧栏联动。终端操作时可以直接打开 SFTP 侧栏，并支持文件拖拽上传。

![SFTP 文件管理](/screenshots/sftp.png)

![终端内置 SFTP 侧栏](/screenshots/sftp_sidebar.png)

## 远程桌面（RDP 与 VNC）

通过可安装的远程桌面 provider，OnetCli 可以在应用内直接打开 RDP 和 VNC 会话：经 RDP 连接 Windows 桌面，或连接任意 VNC 服务端，远程运维操作与数据库、终端、文件共用同一个工作台上下文。

## 远程文件编辑

OnetCli 支持从应用内打开远程文件，并提供语法高亮和自动补全。处理配置文件、脚本和日志片段时，不需要在终端、编辑器和文件传输工具之间来回切换。

![远程文件编辑器](/screenshots/remote_file_editor.png)

## ER 图与服务器监控

OnetCli 也覆盖数据库结构理解和基础服务器监控场景：

- ER Diagram：帮助理解表关系，适合接手新系统、梳理业务数据结构和沟通数据库设计。
- 服务器监控：查看基础服务器状态和趋势图，把远程连接后的常见检查动作产品化。

![ER Diagram](/screenshots/er.png)

![服务器监控](/screenshots/monitor.png)

## AI 助手

AI 在 OnetCli 中是贴近任务上下文的辅助层，而不是单独占据主叙事的聊天入口。

常见用途包括：

- 根据表结构生成 SQL。
- 解释查询结果和字段含义。
- 辅助数据分析并生成图表说明。
- 解释终端命令和排查步骤。
- 帮助把数据库、终端和远程排查信息整理成下一步动作。

![查询结果分析与图表](/screenshots/chatdb.png)

## 适合的使用场景

| 场景 | OnetCli 能做什么 |
| --- | --- |
| 后端开发 | 查看数据库、执行迁移脚本、连接测试服务器、分析接口数据。 |
| 运维排查 | SSH 登录服务器、查看磁盘和日志、传输配置文件、让 AI 辅助整理命令说明。 |
| 缓存排查 | 进入 Redis 查看 Key、值和集群状态，确认缓存数据是否符合预期。 |
| NoSQL 探索 | 在 MongoDB 中浏览集合和文档，结合查询能力定位业务数据。 |
| 数据库管理 | 查看对象结构、执行查询、维护表字段和日常数据检查。 |
| 项目交付 | 跨环境切换时减少工具切换，把连接、命令、文件和数据操作统一到一个桌面入口。 |
