# Navop 使用说明

Navop 是 AI 时代的开发和运维工作台，将数据库、Redis、MongoDB、SSH、SFTP、终端、远程桌面、Notes、AI 和团队同步放在同一个原生工作区。

## 当前版本：v0.17.0

前往[官网下载中心](https://navop.dev/zh-CN/extensions)下载最新稳定版。

- 新增「已知主机」页面：集中查看应用信任的 SSH 主机的密钥算法与指纹，支持复制主机标识、删除可信主机，并可扫描系统 `known_hosts` 导入。
- 优化主页连接卡片、连接树和账户入口布局，窗口空间利用更合理。
- 设置页支持清除快捷键并禁用系统快捷键。
- 优化 AI 对话中的运行中活动显示；修复标签栏导航切换槽位与工作区排序恢复问题。
- Windows 现支持通过 Scoop 安装：`scoop bucket add extras && scoop install navop`。

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
