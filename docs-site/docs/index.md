# Navop 使用说明

Navop 是 AI 时代的开发和运维工作台，将数据库、Redis、MongoDB、SSH、SFTP、终端、远程桌面、Notes、AI 和团队同步放在同一个原生工作区。

## 当前版本：v0.19.5

前往[官网下载中心](https://navop.dev/zh-CN/extensions)下载最新稳定版。

- 个人同步新增 WebDAV 后端：与「文件夹」「Git」并列，填服务器地址、用户名、密码即可用，密码加密后落盘、不明文保存。协议只用 GET / PUT / DELETE 加 Basic 认证，不做 PROPFIND 探测，以兼容坚果云、群晖、Nextcloud 与自建服务；首次写入用 MKCOL 建目录，服务端返回 409 会提示「目录不可用」而不是误报版本冲突。设置页只显示当前后端的项，密码框可切换明文/掩码，WebDAV 没有本地目录可监听，同步由 60 秒周期扫描驱动。
- SSH Agent 转发：凭据里的私钥可勾选「通过 ssh-agent 转发此密钥」，连接侧也有「SSH Agent 转发」（ForwardAgent）开关。本地 ssh-agent 转发给远端后，远端（例如跳板机）能代表你用本机私钥向更内层主机认证，对新开的终端会话生效。
- 表结构设计页新增「刷新表结构」：重新读取最新的列、索引与表信息；设计器里有未保存改动时会先提示，确认后「放弃并刷新」。
- 终端粘贴确认弹窗补上「打开设置」与「不再提示」：多行粘贴、高危命令、大段粘贴三类提示都能直接跳到对应设置项；大段粘贴是硬阈值，不提供「不再提示」。
- 外部驱动改为按调用类别区分请求超时，并支持连接级覆盖：查询、执行、游标、导入导出等用户操作默认 30 分钟，元数据与结构浏览保持 30 秒；连接高级设置新增「请求超时（秒）」（留空按类别默认，0 表示不限制），慢库上的大查询、大导入不再被统一的 30 秒硬超时打断。
- SQL 编辑器手动事务的提交/回滚按钮、表数据页的「提交更改」在写库期间显示 loading，能看出哪一步还在跑，另一个按钮只禁用、不跟着转。
- 修复 WHERE / ORDER BY 过滤输入框里输入中文导致崩溃（#326），过滤条输入框比迁移前高出一行的问题也已修正（40px → 20px）；修复 AI 对话取消回答后会话被误判为失败、达梦等方言改完列注释仍显示旧注释。
- 修复 Linux（Arch + Hyprland / Wayland）应用内退出以 SIGABRT 收尾并留下 core（#336）；macOS Touch Bar 机型关闭窗口闪退交由上游 zed#65186 修复，撤销「关闭即隐藏」实验开关，macOS 两个架构、Windows、Linux 统一回到「关闭即销毁」。

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
