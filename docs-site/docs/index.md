# Navop 使用说明

Navop 是 AI 时代的开发和运维工作台，将数据库、Redis、MongoDB、SSH、SFTP、终端、远程桌面、Notes、AI 和团队同步放在同一个原生工作区。

## 当前版本：v0.19.1

前往[官网下载中心](https://navop.dev/zh-CN/extensions)下载最新稳定版。

- 修复 Windows 下终端频繁卡顿的问题（git bash、PowerShell，操作后切换界面即卡、过会才恢复）：存在高亮/搜索等装饰时只重建发生变化的行，自定义高亮规则只重扫本帧变化的行，本地终端悬停检测目录条目的同步读盘移到后台线程。
- 表格数据支持查找：浏览表数据时按 Cmd/Ctrl+F 打开查找面板，命中单元格高亮描边，Cmd/Ctrl+G / Cmd/Ctrl+Shift+G 在命中间跳转并把所在列横向滚入视口，换页/刷新后按当前词重扫；表数据预览支持字段过滤隐藏列，宽表只看关心的列。
- SQL 编辑器对象详情体验重做：悬停弹详情默认关闭（设置里可开，开启后须停驻 600ms 才显示），改由右键菜单「查看对象详情」打开独立弹窗，内容可选中复制；右键新增「复制 DDL」，选中表名右键也能解析。
- 新增关闭当前页签快捷键 Cmd/Ctrl+Shift+W，可在设置里改绑；连接表单「工作区」字段统一改称「分组」。
- 修复 SSH MFA 登录把保存的密码当作验证码应答导致认证失败的问题；修复老设备（华为 VRP 系等）SSH 连接报 `` `mpint` encoding invalid `` 失败的问题。
- 数据库连接健壮性：复用前 ping 10 秒上限、超时判死并丢弃会话，断开 5 秒上限、超时强制回收，MySQL/PostgreSQL 连接启用 30 秒 TCP keepalive，长时间挂机后的死连接不再永久挂起；修复断连杀死的手工事务卡在死循环。
- 修复 SQLite WITHOUT ROWID 表、视图预览整页空白，PostgreSQL 序列目录一直为空，MCP 客户端配置写入已废弃参数；默认 HTTP 客户端改为跟随系统代理与环境变量代理。
- 打包：Linux 发布包不再链接 WebKitGTK 4.1，渲染依赖独立成 gpu-stack 包按需安装（安装脚本只补宿主缺失的库，支持 --dry-run/--uninstall）；HTML 预览 webview 改为全平台默认关闭，弹窗降级提示「用浏览器打开」「下载 HTML」不受影响。

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
