# Agent / Public MCP 工具现状

本文档说明当前 OnetCli 工具体系的实现状态，重点区分 **Public MCP 工具** 和
**Agent Runtime 工具**。

截至当前实现，Public MCP 仍保留原有面向外部 MCP / CLI 的工具集；Agent Runtime
在 DB、Redis、SSH/SFTP 上改为注册 resource-aware 原生工具，默认使用当前侧边栏
选中的资源，并对危险操作使用高风险审批。

## 1. 总体架构

当前有三层工具概念：

1. `tool_runtime::ToolRegistry`
   - 面向 CLI / Public MCP 的通用工具运行时。
   - 工具名保留点号命名，例如 `db.query`、`sftp.read`、`redis.command`。
   - 迁移期保留旧名 alias，例如 `redis.execute_command -> redis.command`。

2. `public_mcp::tools::PublicMcpToolRegistry`
   - MCP Server 对外暴露的工具注册表。
   - 由 `main/src/public_mcp_runtime/tool_registry.rs` 根据 settings toolset 拼装。

3. `agent_runtime::ToolRegistry`
   - Agent 内部给模型 function calling 使用的工具注册表。
   - 工具名会是模型可调用格式，例如 `db_query`、`redis_execute_command`。
   - 默认携带 `ResourceContext`，可以读取当前侧边栏选中的连接 / database / schema / db。

核心入口：

- Public MCP registry 构建：
  - `main/src/public_mcp_runtime/tool_registry.rs`
  - `build_tool_registry(cx, &toolsets)`
- Agent registry 构建：
  - `main/src/public_mcp_runtime.rs`
  - `agent_runtime_tool_registry(cx)`
- Public MCP 到 Agent 的通用 adapter：
  - `crates/public_mcp/src/tools/agent_runtime_adapter.rs`

## 2. Public MCP 工具集

Public MCP 工具仍然按 `McpToolsetSettings` 开关注册。

### 2.1 terminal toolset

入口：

- `main/src/public_mcp_runtime/tool_registry.rs`
- `terminal_view::public_mcp::registry(cx)`
- `public_mcp::tools::remote_ops_tool_registry(registry)`

工具：

| 工具名 | 说明 | 风险语义 |
|---|---|---|
| `ssh.list_sessions` | 列出当前 App 暴露给 MCP 的活跃 SSH terminal session | 只读 |
| `ssh.session_diagnostics` | 查看一个活跃 SSH session 的诊断信息 | 只读 |
| `ssh.remote_command_poll` | 轮询后台 SSH command 状态 | 只读 |
| `ssh.remote_command_output` | 读取后台 SSH command 输出 | 只读 |
| `ssh.remote_command_cancel` | 取消后台 SSH command | 写/破坏性 |
| `ssh.remote_exec` | 在活跃 SSH terminal session 上执行非交互命令 | 写/开放世界 |

注意：

- 这些工具面向 **活跃 terminal session**，不是保存的 SSH/SFTP connection profile。
- 因此 Agent 侧没有把它包装成“当前侧边栏保存连接”的 `ssh_execute_command`。

### 2.2 internal_functions toolset

入口：

- `main/src/public_mcp_runtime/tool_registry.rs`
- `public_mcp::tools::internal_function_tool_registry(...)`

工具：

| 工具名 | 说明 |
|---|---|
| `internal_functions.list` | 列出 App 内部函数 |
| `internal_functions.call` | 调用指定内部函数 |
| `onetcli.app_info` | 读取 App 元信息，来自 `onetcli_runtime::builtin_tool_registry_with_version` |

### 2.3 connections / workspaces toolset

入口：

- `onetcli_runtime::connections::connection_tool_registry_with_workspaces_and_session_opener`
- `onetcli_runtime::workspaces::workspace_tool_registry`

主要工具：

| 工具名 | 说明 |
|---|---|
| `connections.list` | 列出保存的连接 |
| `connections.show` | 查看单个保存连接 |
| `connections.list_kinds` | 列出可创建的连接类型 |
| `connections.get_schema` | 获取连接创建 schema |
| `connections.validate` | 校验连接创建请求 |
| `connections.create` | 创建保存连接 |
| `connections.find` | 查找保存连接 |
| `connections.update` | 更新保存连接 |
| `connections.delete` | 删除保存连接 |
| `connections.move_workspace` | 移动连接到 workspace |
| `connections.set_sync_enabled` | 设置同步开关 |
| `connections.test` | 测试数据库连接 |
| `connections.open_session` | 打开连接 session |
| `workspaces.list` | 列出 workspace |
| `workspaces.show` | 查看 workspace |

### 2.4 sftp toolset

入口：

- `onetcli_runtime::sftp_tools::sftp_tool_registry(repo)`

Public MCP / CLI 工具：

| 工具名 | 说明 |
|---|---|
| `sftp.list` | 通过保存的 SSH/SFTP 连接列目录 |
| `sftp.read` | 读取远程文件，返回 base64 内容 |
| `sftp.write` | 写远程文件 |
| `sftp.stat` | 查看远程路径 metadata |
| `sftp.upload` | 上传本地路径到远程 |
| `sftp.download` | 下载远程路径到本地 |

### 2.5 database toolset

入口：

- `onetcli_runtime::database_tools::database_tool_registry(repo)`

Public MCP / CLI 工具：

| 工具名 | 说明 |
|---|---|
| `db.schema` | 读取数据库 schema 信息 |
| `db.query` | 执行只读 SQL，非查询语句会被拒绝 |
| `db.exec` | 执行写 SQL / SQL 文件 |

### 2.6 redis toolset

Public MCP 入口：

- `main/src/public_mcp_runtime/redis.rs`
- `public_mcp::tools::RedisToolProvider`

Public MCP 工具：

| 工具名 | 说明 |
|---|---|
| `redis.list_connections` | 列出当前运行中 Redis connection |
| `redis.command` | 对运行中 Redis connection 执行一条 Redis command |
| `redis.keys` | 按 pattern 读取运行中 Redis connection 的 key；只读但可能较重 |
| `redis.get` | 读取运行中 Redis connection 的单个 key；只读 |
| `redis.set` | 写入运行中 Redis connection 的单个 string value；需要审批 |
| `redis.execute_command` | `redis.command` 的兼容 alias |

CLI / function-calling 入口：

- `onetcli_runtime::redis_tools::redis_tool_registry(repo)`

CLI 工具：

| 工具名 | 说明 |
|---|---|
| `redis.command` | 对保存的 Redis 连接执行命令；当前 CLI 侧主要支持 standalone Redis |
| `redis.keys` | 按 pattern 读取保存 Redis 连接中的 key；只读但可能较重 |
| `redis.get` | 读取保存 Redis 连接中的单个 key；只读 |
| `redis.set` | 写入保存 Redis 连接中的单个 string value；需要写权限 |
| `redis.execute_command` | `redis.command` 的兼容 alias |

## 3. Agent Runtime 工具集

Agent registry 构建入口：

- `main/src/public_mcp_runtime.rs`
- `agent_runtime_tool_registry(cx)`

当前策略：

1. 先读取 MCP settings 里的 toolsets。
2. 对 Agent 侧单独关闭：
   - `database`
   - `redis`
   - `sftp`
3. 用剩余 toolsets 构建 Public MCP registry。
4. 通过 `public_mcp::tools::agent_runtime_tool_registry(...)` 转成 Agent 工具。
5. 再额外注册 DB / Redis / SSH-SFTP 原生 Agent 工具。
6. 已迁移到 `tool_runtime` 的工具族，再通过
   `agent_runtime::tools::tool_runtime_agent_tool_registry(...)` 桥接回 Agent。

这样做的原因：

- Public MCP 工具通常要求模型自己传 `connection_id` 或先 list connections。
- Agent 场景应默认使用当前侧边栏资源，不应该把连接列表塞进输入框或让模型自行猜连接。
- DB / Redis / SSH-SFTP 里存在危险写操作，需要更明确的 `RiskLevel::High`。

### 3.1 通用 Public MCP adapter 工具

入口：

- `crates/public_mcp/src/tools/agent_runtime_adapter.rs`

行为：

- 对仍启用的 Public MCP 工具生成 Agent tool。
- 工具名通过 `ToolName::new(...)` 归一化：
  - `connections.list` 变为 `connections_list`
  - `internal_functions.call` 变为 `internal_functions_call`
  - `ssh.remote_exec` 变为 `ssh_remote_exec`
- 当前 adapter 统一标记为 `RiskLevel::Medium`。

当前仍可能通过 adapter 暴露给 Agent 的工具包括：

| 来源 toolset | Agent 工具名示例 | 说明 |
|---|---|---|
| `internal_functions` | `internal_functions_list` / `internal_functions_call` / `onetcli_app_info` | 内部函数与 app info |
| `connections` | `connections_list` / `connections_show` / `connections_create` 等 | 保存连接管理 |
| `workspaces` | `workspaces_list` / `workspaces_show` | workspace 查询 |
| `terminal` | `ssh_list_sessions` / `ssh_remote_exec` 等 | 活跃 SSH terminal session 工具 |

注意：

- `database` / `redis` / `sftp` 在 Agent registry 中被关闭，不走 adapter。
- 因此 Agent 不应再看到旧的 `db_exec`、`redis_execute_command` adapter 版本或 `sftp_write` adapter 版本。
- `redis_execute_command` 这个名字仍存在，但现在来自 Redis 原生 Agent 工具，并且风险为 `High`。
- Redis canonical runtime 工具也会通过 Agent bridge 暴露，模型侧 function 名会归一化成
  `redis_command`、`redis_keys`、`redis_get`、`redis_set`。
- `redis.execute_command` 是 `redis.command` 的 runtime alias，不作为单独 Agent 工具暴露。
- SFTP canonical runtime 工具也会通过 Agent bridge 暴露，模型侧 function 名会归一化成
  `sftp_list`、`sftp_read`、`sftp_write`、`sftp_stat`、`sftp_upload`、`sftp_download`。

### 3.2 DB 原生 Agent 工具

位置：

- `crates/onetcli_runtime/src/agent_db_tools/`

注册入口：

- `onetcli_runtime::agent_db_tools::register_agent_db_tools(repo, registry)`

工具：

| 工具名 | 风险 | 说明 |
|---|---:|---|
| `db_query` | `Read` | 执行只读 SQL。会使用当前 Agent DB resource 的 connection / database / schema。非查询语句会被拒绝。 |
| `db_execute_sql` | `High` | 执行写 SQL / 危险 SQL。Auto 模式也会要求用户审批。 |
| `db_list_databases` | `Read` | 列出当前连接的 database / catalog。 |
| `db_list_tables` | `Read` | 列出当前 database / schema 下的表。 |
| `db_describe_table` | `Read` | 查看表字段、索引、外键。 |
| `db_sample_rows` | `Read` | 读取表的有限样本行。 |

默认资源解析：

- `connection` 参数可显式传。
- 未传 `connection` 时使用 `ToolInvocation::target_resource()`。
- `database` / `schema` 可从参数传入，也可从当前资源 scopes 读取：
  - `database`
  - `schema`

限制：

- `db_query` 通过 DB plugin 的 `is_query_statement` 拦截非查询语句。
- `db_sample_rows` 有行数上限。
- `schema` 目前通过 `config.extra_params["schema"]` 传递，不保证所有数据库驱动都做真实 schema 切换。

### 3.3 Redis 原生 Agent 工具

位置：

- `crates/redis_view/src/agent_tools.rs`
- `crates/onetcli_runtime/src/redis_tools.rs`

注册入口：

- `redis_view::agent_tools::register_agent_redis_tools(cx, registry)`
- `onetcli_runtime::redis_tools::redis_tool_registry(repo)` 通过
  `agent_runtime::tools::tool_runtime_agent_tool_registry(...)` 桥接

工具：

| 工具名 | 风险 | 说明 |
|---|---:|---|
| `redis_execute_command` | `High` | 对当前运行中的 Redis connection 执行一条 Redis command。Auto 模式也会要求用户审批。 |
| `redis_command` | `High` | Agent function-calling 名；canonical runtime id 是 `redis.command`，对保存的 Redis 连接执行一条命令。 |
| `redis_keys` | `Medium` | Agent function-calling 名；canonical runtime id 是 `redis.keys`，按 pattern 读取保存 Redis 连接中的 key，只读但可能较重。 |
| `redis_get` | `Low` | Agent function-calling 名；canonical runtime id 是 `redis.get`，读取保存 Redis 连接中的单个 key。 |
| `redis_set` | `High` | Agent function-calling 名；canonical runtime id 是 `redis.set`，写入保存 Redis 连接中的单个 string value。 |

默认资源解析：

- `connection` 参数可显式传。
- 未传 `connection` 时使用当前 Agent Redis resource id。
- `db` 参数可显式传。
- 未传 `db` 时尝试从当前 Redis resource scope `db` 读取。

实现依赖：

- `redis_view::GlobalRedisState`
- 运行中的 Redis connection。
- `one_core::storage::ConnectionRepository`
- 保存的 `ConnectionType::Redis`

限制：

- `redis_execute_command` 面向当前运行中的 Redis connection；runtime-backed
  `redis_command` / `redis_keys` / `redis_get` / `redis_set` 面向保存连接 repo。
- `redis.command` 可能包含写入、删除、flush、eval 等危险操作，所以标记为 `High`。
- `redis.keys` 只读但可能在大库上较重，所以标记为 `Medium`。后续如需更安全的大库体验，
  应增加 `redis.scan` 或带 limit/cursor 的工具，而不是继续扩大 `KEYS` 语义。

### 3.4 SSH/SFTP 原生 Agent 工具

位置：

- `crates/onetcli_runtime/src/agent_ssh_tools/`
- `crates/onetcli_runtime/src/sftp_tools.rs`

注册入口：

- `onetcli_runtime::agent_ssh_tools::register_agent_ssh_tools(repo, registry)`
- `onetcli_runtime::sftp_tools::sftp_tool_registry(repo)` 通过
  `agent_runtime::tools::tool_runtime_agent_tool_registry(...)` 桥接

工具：

| 工具名 | 风险 | 说明 |
|---|---:|---|
| `ssh_list_dir` | `Read` | 通过当前 SSH/SFTP 保存连接列远程目录。 |
| `ssh_read_file` | `Read` | 读取远程文件，返回 base64 内容。最大读取 1MB。 |
| `ssh_file_stat` | `Read` | 查看远程路径 metadata。 |
| `ssh_write_file` | `High` | 写远程文件。Auto 模式也会要求用户审批。 |
| `sftp_list` | `Read` | Agent function-calling 名；canonical runtime id 是 `sftp.list`，列远程目录。 |
| `sftp_read` | `Read` | Agent function-calling 名；canonical runtime id 是 `sftp.read`，读取远程文件内容。 |
| `sftp_write` | `High` | Agent function-calling 名；canonical runtime id 是 `sftp.write`，写远程文件。 |
| `sftp_stat` | `Read` | Agent function-calling 名；canonical runtime id 是 `sftp.stat`，查看远程路径 metadata。 |
| `sftp_upload` | `High` | Agent function-calling 名；canonical runtime id 是 `sftp.upload`，上传本地文件或目录到远程路径。 |
| `sftp_download` | `High` | Agent function-calling 名；canonical runtime id 是 `sftp.download`，下载远程文件或目录到本地路径。 |

默认资源解析：

- `connection` 参数可显式传。
- 未传 `connection` 时使用当前 Agent SSH resource id。
- 工具要求当前资源 `ResourceKind::Ssh`。

实现依赖：

- `one_core::storage::ConnectionRepository`
- 保存的 `ConnectionType::SshSftp`
- `sftp::RusshSftpClient`
- `ssh::SshConnectConfig`

限制：

- 当前没有实现 `ssh_execute_command` 原生 Agent 工具。
- 原因是现有 `ssh.remote_exec` 面向活跃 terminal session，不是保存连接 profile。
- 当前 SSH/SFTP Agent 工具只覆盖保存连接语义清楚的文件系统操作。
- 迁移期旧 `ssh_*` 文件工具与 runtime-backed `sftp_*` 文件工具并存；后续 prompt
  和工具卡片应优先引导 canonical `sftp.*`，再逐步收敛旧名。

## 4. 审批机制

审批入口：

- `crates/agent_runtime/src/tasks/agent.rs`
- `requires_tool_approval(...)`

当前规则：

1. `update_plan` 和 `delegate_task` 不走人工确认。
2. `ToolExecutionMode::Manual`：
   - 所有业务工具都需要确认。
3. `ToolExecutionMode::Auto`：
   - 只有 `spec.risk.requires_confirmation()` 为 true 的工具需要确认。
   - 当前 `RiskLevel::High` 及以上会要求确认。

因此以下原生 Agent 工具在 Auto 模式也会暂停等待用户审批：

| 工具名 | 风险 |
|---|---:|
| `db_execute_sql` | `High` |
| `redis_execute_command` | `High` |
| `ssh_write_file` | `High` |

测试覆盖：

- `crates/agent_runtime/tests/high_risk_approval.rs`
- `auto_tool_mode_requires_confirmation_for_high_risk_tools`

## 5. ResourceContext 与侧边栏选择

Agent 工具依赖 `ResourceContext` 来获取当前资源。

关键类型：

- `agent_runtime::ResourceContext`
- `agent_runtime::ResourceRef`
- `agent_runtime::ResourceScope`
- `agent_runtime::ResourceKind`

资源来源：

- AI chat 输入上下文由 `ai_chat_view` 构建。
- 连接切换通过侧边栏完成，不通过输入框 `@` 连接 mention 完成。

当前输入框策略：

- `@` 不再列出连接项。
- 如果用户要切换数据库 / Redis / SSH 连接，应在对应侧边栏点击目标连接或目标数据库。

相关实现：

- `crates/ai_chat_view/src/resource_builder.rs`
- `crates/ai_chat_view/src/input/agent_input.rs`

## 6. 目前实现边界

### 已完成

- Agent DB 工具从 Public MCP adapter 改为原生 resource-aware 工具。
- Agent Redis 工具从 adapter 改为原生 resource-aware 工具。
- Agent SSH/SFTP 文件操作从 adapter 改为原生 resource-aware 工具。
- 危险 DB / Redis / SSH 写操作使用 `RiskLevel::High`。
- Auto 模式下高风险工具会要求审批。
- Agent 侧不再暴露旧 `db.exec` / `sftp.write` / `redis.execute_command` adapter 版本。

### 仍保留

- Public MCP / CLI tool runtime 已开始使用 canonical id，并为旧 Redis command 名称保留 alias：
  - `db.query`
  - `db.exec`
  - `sftp.write`
  - `redis.command`
  - `redis.keys`
  - `redis.get`
  - `redis.set`
  - `redis.execute_command` alias
  - `ssh.remote_exec`
- Agent 仍可能通过 adapter 使用 connections / workspaces / internal functions / terminal remote ops。

### 暂未做

- Redis 没拆只读细粒度工具，当前统一通过高风险 `redis_execute_command`。
- SSH 没提供保存连接 profile 语义的 `ssh_execute_command`。
- SFTP upload / download 没作为 Agent 原生工具暴露，目前只暴露 list/read/stat/write。
- Public MCP adapter 的风险仍统一是 `RiskLevel::Medium`，没有按原 MCP annotations 精细映射。

## 7. 代码索引

| 模块 | 路径 | 职责 |
|---|---|---|
| Public MCP runtime | `main/src/public_mcp_runtime.rs` | MCP runtime 生命周期、Agent registry 构建入口 |
| Public MCP toolset 拼装 | `main/src/public_mcp_runtime/tool_registry.rs` | 根据 settings 注册 Public MCP providers |
| Public MCP Redis runtime bridge | `main/src/public_mcp_runtime/redis.rs` | 把 `GlobalRedisState` 暴露给 Public MCP Redis provider |
| Agent DB tools | `crates/onetcli_runtime/src/agent_db_tools/` | DB 原生 Agent 工具 |
| Agent SSH/SFTP tools | `crates/onetcli_runtime/src/agent_ssh_tools/` | SSH/SFTP 原生 Agent 工具 |
| Agent Redis tools | `crates/redis_view/src/agent_tools.rs` | Redis 原生 Agent 工具 |
| Public MCP adapter | `crates/public_mcp/src/tools/agent_runtime_adapter.rs` | 把 Public MCP 工具转成 Agent 工具 |
| Agent 审批逻辑 | `crates/agent_runtime/src/tasks/agent.rs` | 工具调用审批、执行循环 |
| Agent 风险等级 | `crates/agent_runtime/src/risk.rs` | `RiskLevel` 与 `requires_confirmation` |
| ResourceContext | `crates/agent_runtime/src/resource.rs` | 当前资源、资源类型、资源 scopes |
| AI chat resource 构建 | `crates/ai_chat_view/src/resource_builder.rs` | 将连接/侧边栏选择转成 Agent resource context |

## 8. 快速验证命令

常用定向验证：

```bash
rtk cargo test -p onetcli_runtime agent_db_tools -- --nocapture
rtk cargo test -p main public_mcp_runtime::agent_db_registry_tests -- --nocapture
rtk cargo test -p agent_runtime auto_tool_mode_requires_confirmation_for_high_risk_tools -- --nocapture
rtk cargo check -p onetcli_runtime
rtk cargo check -p redis_view
rtk cargo check -p main
rtk cargo fmt --check
```
