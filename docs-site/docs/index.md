# Navop 使用说明

Navop 是 AI 时代的开发和运维工作台，将数据库、Redis、MongoDB、SSH、SFTP、终端、远程桌面、Notes、AI 和团队同步放在同一个原生工作区。

## 当前版本：v0.14.0

前往[官网下载中心](https://navop.dev/zh-CN/extensions)下载最新稳定版。

- SQL 编辑器新增跨数据库/跨 Schema 限定名补全（惰性加载），并优化 FROM 子句的数据库提示、选中数据库限定符建议与限定符元数据作用域隔离。
- SQL 格式化支持保留关键字大小写，新增格式化设置（关键字大小写、缩进）与实时预览，并通过模板掩码避免示例代码/占位符被误格式化。
- 终端将连接状态与认证提示内联显示，不再以弹窗打断操作；后台任务对话框重构为带计数过滤页签，文件操作分组展示更清晰。
- SSH 跳板机配置在禁用后仍保留，便于快速重新启用；SFTP 左侧远端面板遵循配置的 SFTP 初始目录。
- 扩展市场页支持「有更新」过滤，更新通知跳转只显示可更新扩展，并移除 MCP 助手分类。

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
