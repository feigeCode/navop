# Navop 使用说明

Navop 是 AI 时代的开发和运维工作台，将数据库、Redis、MongoDB、SSH、SFTP、终端、远程桌面、Notes、AI 和团队同步放在同一个原生工作区。

## 当前版本：v0.15.1

前往[官网下载中心](https://navop.dev/zh-CN/extensions)下载最新稳定版。

- 终端新增「选中文本后高亮相同内容」：选中一段文本后，可见区域内相同文本会以淡色背景高亮，SSH 与本地终端同时生效，可在终端侧边栏设置中开关（默认开启）。
- 连接列表宽度支持持久化：拖拽调整侧栏连接树宽度后自动保存，重启应用恢复上次宽度；停靠模式侧栏与主窗口背景统一，浮动模式改为浮层卡片样式（圆角 + 阴影）。
- 「自动检查更新」开关与「检查更新」按钮从通用设置页迁移到关于页面，与版本信息同页展示。
- 修复侧边栏与命令栏图标按钮在终端/Agent 自定义主题下颜色不跟随、误显示为黑色的问题。
- 修复 SFTP 覆盖远端文件时恢复旧修改时间（mtime），导致 rsync 部署、Web/应用缓存与增量构建等基于 mtime 的变更检测误判文件未更新的问题。

## 从这里开始

- [快速开始](./guide/quick-start)
- [安装与更新](./guide/install-update)
- [首页、工作区与连接管理](./guide/workspace-connections)

## 按任务查找

- [数据库连接、SQL、导入导出与 Schema 工具](./guide/database-connections)
- [SQL 编辑器、事务与查询结果](./guide/sql-editor)
- [SSH、SFTP、端口转发与 Agent Hub](./guide/ssh-terminal)
- [远程桌面、串口与服务器监控](./guide/remote-access)
- [Notes Markdown 预览与源码编辑](./guide/notes)
- [AI 工作台、Navop Skill 与 Public MCP](./guide/ai-workbench)
- [团队同步与安全](./guide/teams-sync-security)
- [设置与疑难排解](./guide/settings-shortcuts)

文档内容随 Navop 桌面端持续更新。涉及生产数据库、远程服务器、写入 SQL、文件覆盖和批量同步时，请先确认当前环境和操作范围。
