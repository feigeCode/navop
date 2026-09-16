# Navop 使用说明

Navop 是 AI 时代的开发和运维工作台，将数据库、Redis、MongoDB、SSH、SFTP、终端、远程桌面、Notes、AI 和团队同步放在同一个原生工作区。

## 当前版本：v0.18.0

前往[官网下载中心](https://navop.dev/zh-CN/extensions)下载最新稳定版。

- 新增 FTP / FTPS 独立连接类型，并可在同一条 SSH 连接的远程文件面板中切换 SFTP / FTP / FTPS；SSH 连接可配置双击默认打开终端或双栏文件视图。
- 新增原生资源工作台：扩展声明集合、表格与操作，宿主以原生界面渲染；首个发布 Docker 工作台，提供引擎概览、容器与镜像管理、日志、进程、文件系统变更和 exec 终端。
- 本地终端下拉自动识别并列出 WSL 发行版，可按发行版一键启动。
- TDengine 与 MQTT 改为扩展提供，内置实现移除，旧数据自动迁移并按需安装扩展。
- Redis 统一使用内嵌 redis-rs 并让大键加载保持内存有界；修复键树搜索不到 key、Shell Integration 握手导致输入被暂存、MSTSC 光标抖动等问题。

## 从这里开始

- [快速开始](./guide/quick-start)
- [安装与更新](./guide/install-update)
- [首页、工作区与连接管理](./guide/workspace-connections)

## 按任务查找

- [数据库连接、SQL、导入导出与 Schema 工具](./guide/database-connections)
- [SQL 编辑器、事务与查询结果](./guide/sql-editor)
- [SSH、SFTP、端口转发与 Agent Hub](./guide/ssh-terminal)
- [FTP 与 FTPS 远程文件](./guide/ftp-remote-files)
- [远程桌面、串口与服务器监控](./guide/remote-access)
- [原生资源工作台（Docker 等）](./guide/resource-workbench)
- [Notes Markdown 预览与源码编辑](./guide/notes)
- [AI 工作台、Navop Skill 与 Public MCP](./guide/ai-workbench)
- [团队同步与安全](./guide/teams-sync-security)
- [设置与疑难排解](./guide/settings-shortcuts)

文档内容随 Navop 桌面端持续更新。涉及生产数据库、远程服务器、写入 SQL、文件覆盖和批量同步时，请先确认当前环境和操作范围。
