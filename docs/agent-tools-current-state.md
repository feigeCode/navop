# Agent / Public MCP 工具现状

本文档记录当前 OnetCli 工具体系的真实代码状态，重点说明哪些工具已经收敛到
`tool_runtime`，以及 Agent / Public MCP / CLI 分别如何暴露这些工具。

当前方向是 **canonical-only**：

- 产品语义只保留 OnetCli Tool Runtime。
- Agent / MCP / CLI / UI 都只是入口适配器。
- 已迁移工具不再保留旧工具名或旧 alias。
- 模型侧 function name 会因为 function calling 限制把点号规范名归一化为下划线，
  例如 `db.exec` -> `db_exec`、`sftp.read` -> `sftp_read`。

## 1. 总体架构

当前有三层入口，但业务工具来源正在收敛到一个 runtime registry：

1. `tool_runtime::ToolRegistry`
   - 通用工具运行时。
   - canonical 工具名使用点号命名，例如 `db.query`、`sftp.read`、`redis.command`。
   - `redis.execute_command`、`ssh.remote_exec`、`ssh.remote_command_*` 等旧 alias
     已不再解析。

2. `public_mcp::tools::PublicMcpToolRegistry`
   - MCP Server 的协议入口。
   - 通过 `ToolRuntimeMcpProvider` 或 runtime-backed provider 暴露 canonical tools。

3. `agent_runtime::ToolRegistry`
   - Agent function calling 入口。
   - 对 `tool_runtime` descriptors 做 function-name 归一化。
   - DB / Redis / SFTP 工具通过
     `agent_runtime::tools::tool_runtime_agent_tool_registry(...)` 从同一个
     `tool_runtime::ToolRegistry` 派生。

核心入口：

- Public MCP registry 构建：
  - `main/src/public_mcp_runtime/tool_registry.rs`
  - `build_tool_registry(cx, &toolsets)`
- Agent registry 构建：
  - `main/src/public_mcp_runtime.rs`
  - `agent_runtime_tool_registry(cx)`
- Runtime -> Agent bridge：
  - `crates/agent_runtime/src/tools/runtime_adapter.rs`

## 2. Public MCP / CLI 工具集

### 2.1 terminal toolset

入口：

- `main/src/public_mcp_runtime/tool_registry.rs`
- `terminal_view::public_mcp::registry(cx)`
- `public_mcp::tools::remote_ops_tool_registry(registry)`
- `public_mcp::tools::terminal_exec_tool_registry(registry)`

工具：

| 工具名 | 说明 | 风险语义 |
|---|---|---|
| `ssh.list_sessions` | 列出当前 App 暴露给 MCP 的活跃 SSH terminal session | 只读 |
| `ssh.session_diagnostics` | 查看一个活跃 SSH session 的诊断信息 | 只读 |
| `ssh.command.poll` | 轮询后台 SSH command 状态 | 只读 |
| `ssh.command.output` | 读取后台 SSH command 输出 | 只读 |
| `ssh.command.cancel` | 取消后台 SSH command | 写/破坏性 |
| `ssh.exec` | 在活跃 SSH terminal session 上执行结构化非交互命令 | 写/开放世界 |
| `terminal.exec` | 把命令写入可见 terminal PTY，形成“像手动输入一样执行”的效果 | 写/开放世界 |

注意：

- `ssh.exec` 是结构化 SSH 执行，不会把命令写入可见终端。
- `terminal.exec` 是可见终端执行，会写入 live terminal input path。
- `ssh.remote_exec`、`ssh.remote_command_poll`、`ssh.remote_command_output`、
  `ssh.remote_command_cancel` 已不再作为 alias 接受。

### 2.2 connections / workspaces / internal functions

入口：

- `onetcli_runtime::connections::connection_tool_registry_with_workspaces_and_session_opener`
- `onetcli_runtime::workspaces::workspace_tool_registry`
- `public_mcp::tools::internal_function_tool_registry(...)`

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
| `internal_functions.list` | 列出 App 内部函数 |
| `internal_functions.call` | 调用指定内部函数 |
| `onetcli.app_info` | 读取 App 元信息 |

### 2.3 database toolset

入口：

- `onetcli_runtime::database_tools::database_tool_registry(repo)`

工具：

| 工具名 | 说明 |
|---|---|
| `db.schema` | 读取数据库 schema 信息 |
| `db.tables` | 列出保存数据库连接中的表 |
| `db.describe_table` | 读取表字段、索引和外键 metadata |
| `db.sample_rows` | 读取单表有限样例行，默认 20 行、最多 100 行 |
| `db.query` | 执行只读 SQL，非查询语句会被拒绝 |
| `db.exec` | 执行写 SQL / SQL 文件 |

### 2.4 sftp toolset

入口：

- `onetcli_runtime::sftp_tools::sftp_tool_registry(repo)`

工具：

| 工具名 | 说明 |
|---|---|
| `sftp.list` | 通过保存的 SSH/SFTP 连接列目录 |
| `sftp.read` | 读取远程文件，返回 base64 内容 |
| `sftp.write` | 写远程文件 |
| `sftp.stat` | 查看远程路径 metadata |
| `sftp.upload` | 上传本地路径到远程 |
| `sftp.download` | 下载远程路径到本地 |

### 2.5 redis toolset

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

CLI / function-calling 入口：

- `onetcli_runtime::redis_tools::redis_tool_registry(repo)`

CLI 工具：

| 工具名 | 说明 |
|---|---|
| `redis.command` | 对保存的 Redis 连接执行命令；当前 CLI 侧主要支持 standalone Redis |
| `redis.keys` | 按 pattern 读取保存 Redis 连接中的 key；只读但可能较重 |
| `redis.get` | 读取保存 Redis 连接中的单个 key；只读 |
| `redis.set` | 写入保存 Redis 连接中的单个 string value；需要写权限 |

`redis.execute_command` 已不再作为 alias 接受。

## 3. Agent Runtime 工具集

Agent registry 构建入口：

- `main/src/public_mcp_runtime.rs`
- `agent_runtime_tool_registry(cx)`

当前策略：

1. 读取 MCP settings 里的 toolsets。
2. 对 Agent 侧单独关闭 `database`、`redis`、`sftp` 的通用 Public MCP adapter。
3. 用剩余 toolsets 构建 Public MCP registry，并通过
   `public_mcp::tools::agent_runtime_tool_registry(...)` 转成 Agent 工具。
4. DB / Redis / SFTP 直接从 `tool_runtime::ToolRegistry` 通过
   `tool_runtime_agent_tool_registry(...)` 桥接到 Agent。
5. 旧 native Agent DB / SSH 工具模块已删除；旧 Redis Agent 工具不再注册。

### 3.1 通用 Public MCP adapter 工具

入口：

- `crates/public_mcp/src/tools/agent_runtime_adapter.rs`

当前仍可能通过 adapter 暴露给 Agent 的工具包括：

| 来源 toolset | Agent 工具名示例 | 说明 |
|---|---|---|
| `internal_functions` | `internal_functions_list` / `internal_functions_call` / `onetcli_app_info` | 内部函数与 app info |
| `connections` | `connections_list` / `connections_show` / `connections_create` 等 | 保存连接管理 |
| `workspaces` | `workspaces_list` / `workspaces_show` | workspace 查询 |
| `terminal` | `ssh_exec` / `terminal_exec` / `ssh_command_poll` 等 | 活跃 SSH terminal session 工具 |

### 3.2 DB Agent 工具

入口：

- `onetcli_runtime::database_tools::database_tool_registry(repo)` 通过
  `agent_runtime::tools::tool_runtime_agent_tool_registry(...)` 桥接。

Agent function tools：

| Agent function 名 | canonical runtime id | 风险 | 说明 |
|---|---|---:|---|
| `db_schema` | `db.schema` | `Read` | 读取 schema-level metadata |
| `db_tables` | `db.tables` | `Read` | 列出数据库表 |
| `db_describe_table` | `db.describe_table` | `Read` | 读取表字段、索引和外键 metadata |
| `db_sample_rows` | `db.sample_rows` | `Read` | 读取单表有限样例行 |
| `db_query` | `db.query` | `Read` | 执行只读 SQL |
| `db_exec` | `db.exec` | `High` | 执行 SQL script 或 SQL file |

不再暴露：

- `db_execute_sql`
- `db_list_databases`
- `db_list_tables`

`db_describe_table` 和 `db_sample_rows` 现在是 canonical runtime id 派生出的
function name，不是旧 native Agent 工具的兼容入口。

### 3.3 Redis Agent 工具

入口：

- `onetcli_runtime::redis_tools::redis_tool_registry(repo)` 通过
  `agent_runtime::tools::tool_runtime_agent_tool_registry(...)` 桥接。

Agent function tools：

| Agent function 名 | canonical runtime id | 风险 | 说明 |
|---|---|---:|---|
| `redis_command` | `redis.command` | `High` | 对保存的 Redis 连接执行一条命令 |
| `redis_keys` | `redis.keys` | `Medium` | 按 pattern 读取 key；只读但可能较重 |
| `redis_get` | `redis.get` | `Low` | 读取单个 key |
| `redis_set` | `redis.set` | `High` | 写入单个 string value |

不再暴露：

- `redis_execute_command`
- `redis.execute_command`

### 3.4 SFTP Agent 工具

入口：

- `onetcli_runtime::sftp_tools::sftp_tool_registry(repo)` 通过
  `agent_runtime::tools::tool_runtime_agent_tool_registry(...)` 桥接。

Agent function tools：

| Agent function 名 | canonical runtime id | 风险 | 说明 |
|---|---|---:|---|
| `sftp_list` | `sftp.list` | `Read` | 列远程目录 |
| `sftp_read` | `sftp.read` | `Read` | 读取远程文件内容 |
| `sftp_write` | `sftp.write` | `High` | 写远程文件 |
| `sftp_stat` | `sftp.stat` | `Read` | 查看远程路径 metadata |
| `sftp_upload` | `sftp.upload` | `High` | 上传本地文件或目录到远程路径 |
| `sftp_download` | `sftp.download` | `High` | 下载远程文件或目录到本地路径 |

不再暴露：

- `ssh_list_dir`
- `ssh_read_file`
- `ssh_file_stat`
- `ssh_write_file`

## 4. 审批机制

审批入口：

- `crates/agent_runtime/src/tasks/agent.rs`
- `requires_tool_approval(...)`

当前规则：

1. `update_plan` 和 `delegate_task` 不走人工确认。
2. `ToolExecutionMode::Manual`：所有业务工具都需要确认。
3. `ToolExecutionMode::Auto`：`spec.risk.requires_confirmation()` 为 true 的工具需要确认。

当前高风险 Agent function tools：

| 工具名 | 风险 |
|---|---:|
| `db_exec` | `High` |
| `redis_command` | `High` |
| `redis_set` | `High` |
| `sftp_write` | `High` |
| `sftp_upload` | `High` |
| `sftp_download` | `High` |
| `ssh_exec` | `High` 或 adapter 映射风险 |
| `terminal_exec` | `High` 或 adapter 映射风险 |

测试覆盖：

- `crates/agent_runtime/tests/high_risk_approval.rs`
- `auto_tool_mode_requires_confirmation_for_high_risk_tools`

## 5. ResourceContext 与资源池

Agent 仍使用 `agent_runtime::ResourceContext`，但产品语义已经按资源池方向推进：

- default resource 是默认目标，不是能力边界。
- 可操作资源来自当前 Agent 会话的 resource pool。
- runtime-backed Agent 工具 schema 已统一暴露 `target`，不再向模型暴露
  `connection` / `connection_id` / `session_id`。
- Agent adapter 会把 `target` 或默认目标映射回当前 runtime handler 仍需要的
  provider 字段；如果模型直接传 provider 字段，Agent adapter 会拒绝。

关键类型：

- `agent_runtime::ResourceContext`
- `agent_runtime::ResourceRef`
- `agent_runtime::ResourceScope`
- `agent_runtime::ResourceKind`

资源来源：

- AI chat 输入上下文由 `ai_chat_view` 构建。
- 连接切换通过侧边栏完成，不通过输入框 `@` 连接 mention 完成。

## 6. 目前实现边界

已完成：

- DB / Redis / SFTP Agent 工具通过 `tool_runtime` bridge 暴露。
- 旧 DB / Redis / SFTP Agent 工具名不再注册。
- `redis.execute_command` 不再作为 `redis.command` alias 解析。
- `ssh.remote_exec` 和 `ssh.remote_command_*` 不再作为 `ssh.*` alias 解析。
- Agent prompt 会在可用时提示使用统一工具命名规则。
- Agent prompt 的资源段使用“资源池 / 默认目标”语义，并要求工具调用使用
  `target` 参数。
- `db.tables`、`db.describe_table`、`db.sample_rows` 已作为 canonical DB metadata
  工具补齐，并通过 Agent bridge 暴露为 `db_tables`、`db_describe_table`、
  `db_sample_rows`。
- 危险 DB / Redis / SFTP 写操作使用 `RiskLevel::High`。

暂未做：

- 底层 runtime handler、CLI 和部分 Public MCP provider 仍使用 `connection`、
  `connection_id` 或 `session_id`，后续需要继续收敛到 runtime-core target
  resolution。
- Public MCP adapter 的风险仍有部分路径不是直接由 runtime annotations 精细映射。
- 旧 `redis_view::agent_tools` 代码仍存在于 `redis_view` crate，但主 Agent registry 不再调用。

## 7. 代码索引

| 模块 | 路径 | 职责 |
|---|---|---|
| Public MCP runtime | `main/src/public_mcp_runtime.rs` | MCP runtime 生命周期、Agent registry 构建入口 |
| Public MCP toolset 拼装 | `main/src/public_mcp_runtime/tool_registry.rs` | 根据 settings 注册 Public MCP providers |
| Runtime -> Agent adapter | `crates/agent_runtime/src/tools/runtime_adapter.rs` | 把 runtime descriptor/call 转成 Agent tool |
| Agent prompt | `crates/agent_runtime/src/tasks/agent_prompt.rs` | 工具命名、资源上下文、终端选择规则 |
| DB runtime tools | `crates/onetcli_runtime/src/database_tools.rs` | `db.schema` / `db.tables` / `db.describe_table` / `db.sample_rows` / `db.query` / `db.exec` |
| SFTP runtime tools | `crates/onetcli_runtime/src/sftp_tools.rs` | `sftp.*` 文件工具 |
| Redis runtime tools | `crates/onetcli_runtime/src/redis_tools.rs` | CLI / function-calling Redis 工具 |
| Public MCP Redis provider | `crates/public_mcp/src/tools/redis.rs` | MCP Redis active-connection 工具 |
| Public MCP remote ops | `crates/public_mcp/src/tools/remote_ops.rs` | `ssh.*` structured SSH 工具 |
| Agent 审批逻辑 | `crates/agent_runtime/src/tasks/agent.rs` | 工具调用审批、执行循环 |
| AI chat resource 构建 | `crates/ai_chat_view/src/resource_builder.rs` | 将连接/侧边栏选择转成 Agent resource context |

## 8. 快速验证命令

常用定向验证：

```bash
rtk cargo test -p agent_runtime system_prompt_prefers_canonical_runtime_tool_names
rtk cargo test -p agent_runtime --test tool_runtime_target_adapter
rtk cargo test -p agent_runtime
rtk cargo test -p main agent_runtime_tool_registry
rtk cargo test -p onetcli_runtime --test database_tools
rtk cargo test -p onetcli_runtime --test redis_tools
rtk cargo test -p public_mcp --test redis_tools
rtk cargo test -p public_mcp --test remote_ops
rtk cargo test -p onetcli_runtime sftp_tools
rtk cargo check -p onetcli_runtime
rtk cargo check -p public_mcp
rtk cargo check -p main
rtk git diff --check
```
