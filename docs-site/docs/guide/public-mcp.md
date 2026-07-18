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

Navop 还可安装或更新供 Codex 与 Agents 使用的 Navop Skill。使用 Skill 前必须全局安装 `@navop/cli`；AI Agent 通过 `navop ... --json` 发现并操作 Navop 中的数据库、SSH、终端、文件、连接和工作区资源。Skill 不会把所有工具说明静态写进提示词，而是指导 Agent 在需要时查询状态、命令和实时 Schema。

## 为什么使用 Navop Skill

直接把 Navop 配置为原生 MCP Server 时，客户端可能在每轮对话中向模型携带大量已暴露工具的名称、描述和 JSON Schema。随着工具增多，这些重复定义会占用上下文并增加 Token 消耗。Navop Skill 让 AI Agent 平时只保留紧凑工作流，真正执行任务时再通过终端按需运行 `navop` 的状态、领域命令或 `tool schema/call`。

```bash
npm install -g @navop/cli@latest
navop skill install --target codex --scope user
navop status --json
navop db query --help
navop tool schema <tool-name> --json
navop tool call <tool-name> --arguments '<json-object>' --json
```

下面是几类常见的只读操作示例。占位符必须替换为 Navop 实时返回的连接或会话 ID，执行前应先运行对应命令的 `--help` 或读取实时 Schema：

```bash
# 发现当前资源与会话
navop connections list --json
navop connections sessions --json

# SSH：在已打开的 SSH 会话中执行命令
navop ssh exec --target <ssh-session-id> --command 'uname -a' --json

# SFTP：列出远程日志目录
navop sftp list --connection <ssh-connection-id-or-name> --path /var/log --json

# Redis：读取一个 Key
navop redis get --connection-id <redis-connection-id-or-name> --key app:status --json

# MongoDB：查询活动用户
navop mongo find --connection-id <mongo-session-id> --database app --collection users --filter '{"active":true}' --limit 20 --json

# SQL 数据库：执行只读查询
navop db query --connection <database-connection-id-or-name> --sql 'SELECT 1' --json

# 可见终端：读取最近输出
navop terminal read --target <terminal-session-id> --lines 80 --json
```

这种方式特别适合 Codex 等能够执行终端命令的 Agent：无需把完整 Navop 工具目录预先注册到每一轮模型上下文，就能按任务发现当前工具和资源，通常可以减少重复上下文和 Token 开销。实际节省量取决于客户端如何注入 MCP 工具定义以及当前启用的工具数量。

Skill 并不意味着底层完全绕开 MCP。`navop` CLI 内部仍连接 Navop 的本机认证 Public MCP endpoint，Navop 继续负责 Tool Exposure、资源 ID、权限、审批、会话、结果和审计。Skill 只是改变 Agent 侧的使用方式：从“每轮携带整套工具”改成“通过终端按需发现和调用”。

## 使用 @navop/cli

`@navop/cli` 提供 `status`、`tools`、`schema`、`call` 及各资源领域命令。独立的 `@navop/mcp` 只负责为兼容客户端运行 stdio 桥接。

使用 `navop ...` 前确认包来源。工具列表、资源 ID 和参数必须从当前 `tools`/`schema` 结果获取，不允许猜测连接 ID、复用其他设备的 ID 或绕过审批。

## 审批、资源与故障处理

审批窗口会展示外部客户端请求的实际操作。核对客户端、工具、目标连接和参数后再允许；拒绝后应回到客户端修改请求，而不是放宽全部权限。ACP 已授权并不代表 Public MCP 自动放行，二次审批用于保护宿主能力。

连接失败时依次检查 Navop 是否运行、服务模式、Node.js 版本、客户端配置、discovery 文件权限和 Tool Exposure。工具缺失通常是未暴露或当前版本不支持；Schema 不匹配时重新连接并获取实时定义。日志和配置发给他人前删除 Token、路径、连接名称与业务参数。

## 安装前检查

Public MCP 由正在运行的 Navop 提供真实工具；`@navop/client` 是共享连接层，`@navop/cli` 提供终端命令和 Skill，`@navop/mcp` 只提供 stdio 桥接。开始前确认：

1. Navop 正在运行，并已在“设置 → 通用 → MCP”启用 MCP Server。
2. 当前设备已安装 Node.js 20 或更高版本，并能运行 `npx`。
3. 已选择合适的 Permission Profile。
4. Tool Exposure 只开放当前任务需要的能力组。
5. AI Agent 已全局安装 `@navop/cli@latest`；原生 MCP 客户端使用 `@navop/mcp@latest`。

## CLI 自检流程

下面的命令适合确认运行时、工具列表和单个工具 Schema。示例版本以当前 README 为准；实际使用时优先复制 Navop 设置页给出的精确命令。

```bash
navop status --json
navop tools --json
navop schema <tool-name> --json
navop call <tool-name> --arguments '<json-object>' --json
npx -y @navop/mcp@latest
```

需要确认或更新 CLI 时运行：

```bash
npm view @navop/cli version
navop --version
```

使用 Skill 前运行 `npm install -g @navop/cli@latest`；更新已安装的 CLI 可运行 `npm update -g @navop/cli`。

推荐的排查顺序是 `status → tools → schema → call`。不要猜测工具名称、参数或资源 ID；资源 ID 应来自 Navop 实时返回的连接、会话或工作区结果。

## 权限档位

| 档位 | 行为 | 推荐用途 |
| --- | --- | --- |
| Safe / `deny` | 允许只读发现，拒绝修改操作 | 初次配置、审计和只读查询 |
| Confirm / `ask` | 修改操作需要在 Navop 中确认 | 日常交互式使用 |
| Auto / `allow` | 修改操作自动执行 | 受控自动化环境，必须明确限制 Tool Exposure |

即使选择 Auto，调用者也应该明确表达破坏性意图。Tool Exposure 中被关闭的能力组不会被宿主声明为可用，客户端不应尝试绕过。

## 当前能力组

| 能力组 | 当前宿主能力 |
| --- | --- |
| Runtime | 兼容信息、权限提示、Tool Exposure 状态、实时工具和 Schema |
| SSH | 隔离命令执行、会话诊断、后台命令轮询、输出和取消 |
| Visible terminal | 读取有限滚动区、在可见 PTY 执行、明确中断 |
| SQL databases | Schema、表、描述、样例行、只读查询和可写执行 |
| Redis | 活动连接、命令、Keys、Get、Set |
| MongoDB | 数据库、集合、Find、Aggregate、Count、索引、校验、CRUD、Explain |
| SFTP | List、Stat、Read、Write、Upload、Download |
| Connections | List、Find、Show、Kinds、Schema、Validate、Save、Delete、Test、Open、Sessions |
| Workspaces | List 和 Show |
| Internal functions | 列出宿主函数并按实时 Schema 调用 |

能力以运行中的 Navop 返回结果为准。Navop 可以更新宿主工具而不要求 npm 包同步发布，因此文档中的静态列表只能作为导航，不能替代 `tools/list` 和实时 Schema。

## 给外部客户端配置 MCP

Navop 可以管理 Codex、Claude Desktop 和 Claude Code 的配置，也可以复制通用 MCP JSON。使用自动安装或更新前先查看目标配置文件，避免覆盖同名的自定义 Server。

配置完成后重启或重新加载客户端，再执行只读状态检查。若客户端看不到新工具，按顺序检查 Navop 是否运行、MCP Server 开关、Tool Exposure、权限档位、discovery 文件权限和客户端缓存。
