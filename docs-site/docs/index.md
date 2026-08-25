# Navop 使用说明

Navop 是 AI 时代的开发和运维工作台，将数据库、Redis、MongoDB、SSH、SFTP、终端、远程桌面、Notes、AI 和团队同步放在同一个原生工作区。

## 当前版本：v0.11.0

[下载 Navop v0.11.0](https://github.com/feigeCode/navop/releases/tag/v0.11.0)

- 连接列表新增「连接排序」设置（设置 → 通用 → 连接显示），默认按名称自然排序（IP 等数字段按数值比较、忽略大小写），也可切换为「最近使用优先」；首页连接列表、Redis/MongoDB 工作区标签页与持久侧栏连接树统一应用该配置。
- SSH 新增可选兼容支持，可连接仅支持 DSA 主机密钥、SHA-1 密钥交换/MAC 或 1024 位 DH 组协商的旧设备。
- 复制标签页自动追加序号并复用已释放的编号，标签宽度随内容自适应，长标题不再被截断。

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
