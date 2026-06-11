---
title: 文档
description: 快速开始使用 OnetCli，连接数据库、Redis、MongoDB、SSH/SFTP、终端与 AI 工作流
---

# 文档

这一页负责把 OnetCli 的常用入口和第一条使用路径收拢到一起。更完整的源码结构、构建细节和版本文件，以 GitHub 仓库为准。

## 快速开始

1. 从 [GitHub Releases](https://github.com/feigeCode/onetcli/releases) 下载对应平台版本。
2. 启动 OnetCli，先创建一个数据库连接，例如 MySQL、PostgreSQL 或 SQLite。
3. 按你的运维或开发场景添加 SSH 主机，进入远程终端。
4. 如果需要文件操作，打开 SFTP 文件管理或终端内置 SFTP 侧栏。
5. 如果你使用 Redis 或 MongoDB，可以进入对应入口浏览 Key、集合和文档。
6. 在 SQL、查询结果或终端排查场景中使用 AI 辅助生成、解释和整理下一步操作。

![OnetCli 主应用工作台](/screenshots/app.png)

## 第一条推荐工作流

### 1. 创建连接

先在工作区中创建数据库或服务器连接。OnetCli 的核心价值是让常用环境能被保存、复用，并保持上下文。

### 2. 定位对象

根据任务进入对应界面：

- 数据库：浏览库、表、字段、视图、函数。
- Redis：浏览 Key，查看值和缓存状态。
- MongoDB：浏览集合和文档。
- SSH：进入远程终端查看系统状态。
- SFTP：浏览远程目录和传输文件。

### 3. 执行操作

在同一应用内执行 SQL、运行命令、传输文件、查看日志或编辑远程文件，减少跨工具切换。

### 4. 使用 AI 辅助

AI 可以辅助：

- 生成 SQL。
- 解释查询结果。
- 总结数据分析结论。
- 解释终端命令。
- 整理排查步骤。

## 常用入口

- [功能总览](/features)
- [下载页面](/download)
- [更新日志](/changelog)
- [GitHub 仓库](https://github.com/feigeCode/onetcli)

## 适合谁使用

- 需要同时处理数据库、服务器与终端任务的后端开发者。
- 需要把 SSH、SFTP、Redis、MongoDB 和数据库排查放到同一工作台的运维同学。
- 希望在桌面端直接结合 AI 做 SQL、命令解释和数据分析的工程团队。
- 需要在多个项目、环境和连接之间频繁切换的重度桌面工具用户。

## 技术定位

OnetCli 是纯 Rust 构建的原生桌面应用，基于 GPUI 和 GPU 渲染路线，不依赖 WebView，也就是官网首页强调的 No WebView。官网上的“高性能一体化运维工作台”指的是：把数据库、远程连接、文件传输、终端和 AI 辅助放到一个可长时间使用的桌面工作台中。
