# Public MCP 与外部自动化

Public MCP 让 Codex、Claude Desktop、Claude Code 和兼容客户端调用当前正在运行的 Navop。它不是云端固定 API：endpoint 使用动态 loopback 端口和仅当前用户可读的 discovery token，真实工具与 Schema 来自本机 Navop。

## 服务模式与发现

Temporary 模式适合临时会话，应用或任务结束后不应依赖它长期存在；Persistent 模式用于需要持续发现的本地客户端。两种模式都只应监听 loopback，并依赖用户级 discovery 信息连接。不要把 discovery token、配置文件或端口转发到公网。

切换模式、重启应用或修改 Tool Exposure 后，MCP endpoint 可能重启。外部 MCP/ACP 客户端失去连接时应重新发现或重连，而不是缓存旧端口和 Token。

## 权限档位与 Tool Exposure

Safe、Confirm 和 Auto 权限档位决定工具调用的审批强度。首次接入选择 Safe 或 Confirm；只有在任务、工具和目标都高度可控时才考虑 Auto。权限档位不替代远端账号和数据库权限。

Tool Exposure 可分别开放 Terminal、SSH Exec、可见终端、Connections、SFTP、Redis、MongoDB、Database 和内部函数。只启用当前客户端真正需要的类别，任务结束后关闭。修改暴露范围可能重启服务，现有客户端需要重连。

## 安装客户端依赖

Public MCP 客户端桥接需要 Node.js 20+ 和可用的 `npx`。先在终端确认版本，再从 Navop 设置页复制 Codex、Claude Desktop、Claude Code 或通用 MCP JSON 配置。不同客户端的配置位置与重启方式不同，应按界面生成的内容操作。

Navop 还可安装或更新供 Codex 与 Agents 使用的 Navop Skill。Skill 提供使用指导，不会把 npm 包变成静态工具 registry；客户端仍需连接运行中的 Navop 才能获取真实工具。

## 使用 @navop/mcp CLI

`@navop/mcp` CLI 提供 `status`、`tools`、`schema`、`call` 和 `mcp` 等命令。`status` 检查发现与服务状态，`tools` 列出当前暴露工具，`schema` 读取实时参数定义，`call` 调用指定工具，`mcp` 为兼容客户端运行桥接。

使用 `npx @navop/mcp ...` 前确认包来源和版本。工具列表、资源 ID 和参数必须从当前 `tools`/`schema` 结果获取，不允许猜测连接 ID、复用其他设备的 ID 或绕过审批。

## 审批、资源与故障处理

审批窗口会展示外部客户端请求的实际操作。核对客户端、工具、目标连接和参数后再允许；拒绝后应回到客户端修改请求，而不是放宽全部权限。ACP 已授权并不代表 Public MCP 自动放行，二次审批用于保护宿主能力。

连接失败时依次检查 Navop 是否运行、服务模式、Node.js 版本、客户端配置、discovery 文件权限和 Tool Exposure。工具缺失通常是未暴露或当前版本不支持；Schema 不匹配时重新连接并获取实时定义。日志和配置发给他人前删除 Token、路径、连接名称与业务参数。
