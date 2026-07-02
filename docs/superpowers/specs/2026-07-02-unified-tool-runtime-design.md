# Unified Tool Runtime Design

## Goal

将 OnetCli 当前分散的 Agent 工具、Public MCP 工具、CLI 工具和 Tool Runtime 工具收敛为一个产品语义：

```text
OnetCli Tool Runtime
```

Agent / ACP、Public MCP、CLI、UI 只作为入口适配器存在，不再拥有各自独立的业务工具语义、目标参数体系、权限策略或审计模型。

最终用户和 Agent 只需要理解：

1. 当前会话有哪些可操作资源。
2. 每个工具能操作哪些资源。
3. 本次调用的目标资源是什么。
4. 操作风险多高。
5. 是否需要确认。

## Current State

当前仓库已经具备部分统一基础，但仍存在三套工具边界：

1. `crates/tool_runtime`
   - 已有 `ToolDescriptor`、`ToolRegistry`、`ToolHandler`、`ToolAdapter`、`ToolContext`、`ToolResult`。
   - `public_mcp` 和 `onetcli_runtime` 已经有部分工具使用它，例如 `db.query`、`db.exec`、`sftp.list`、`sftp.read`。
   - 目前缺少资源池、统一目标解析、统一权限、审批、审计、alias 和 invocation contract。

2. `crates/agent_runtime`
   - 仍有自己的 `ResourceContext`、`ToolSpec`、`ToolRegistry`、`ToolInvocation`、`ToolRouter`。
   - Agent 工具名仍有 `db_query`、`db_execute_sql`、`ssh_read_file` 等旧 Agent 语义。
   - Agent 权限模式是 `ToolExecutionMode::Auto / ReadOnly / Manual`。

3. `crates/public_mcp`
   - 有自己的 `PublicMcpToolRegistry`、`PublicMcpToolProvider`、`PermissionMode::Deny / Ask / Allow`。
   - 已有 `ToolRuntimeMcpProvider` 可以将 `tool_runtime::ToolRegistry` 暴露为 MCP tools。
   - 仍存在 MCP 侧权限判断和工具 provider 聚合语义。

UI 侧的 `ResourceContext.current` 已接近默认目标语义，但产品文案仍是“上下文 / 当前资源”，不是“默认目标 / 资源池”。

## Design Principles

1. `tool_runtime` 是唯一真实执行层。
2. `agent_runtime` 负责会话、turn、prompt、计划、审批事件、transcript，不再长期定义业务 Tool trait。
3. `public_mcp` 负责 MCP 协议、transport、discovery、stdio bridge，不再长期拥有业务工具目录。
4. CLI 和 UI 只做入口适配、参数展示、结果展示和用户操作。
5. 所有业务能力统一注册到 `tool_runtime::ToolRegistry`。
6. Agent、MCP、CLI、UI 都从同一个 registry 派生工具列表。
7. 权限、资源、审批、审计、风险等级在 runtime core 统一处理。
8. `default_target` 只是默认目标，不是资源池边界。
9. 模型只看到 canonical tool id；旧工具名通过 alias 兼容。
10. 每个阶段都朝最终架构移动，不引入会长期保留的临时产品语义。

## Target Architecture

```text
                 AI Agent / ACP
                       |
                Agent Tool Adapter
                       |
Public MCP Adapter -> Tool Runtime Core <- UI / Command Adapter
                       |
                Resource Providers
             DB / SSH / Redis / SFTP / Terminal
```

模块职责：

```text
crates/tool_runtime
  Core models:
    ToolId, ResourceId
    ToolDescriptor, ToolAnnotations, ToolOrigin
    ResourcePool, ResourceRef, ResourceKind, ResourceScope
    ToolInvocation, ToolCaller, AuditContext
    PermissionPolicy, OperationPolicy, PermissionDecision
    ApprovalRequest, AuditEvent
    ToolRegistry, ToolRouter

crates/agent_runtime
  Agent workflow:
    session, turn, prompt, planning, transcript, subagent
  Adapter:
    converts tool_runtime descriptors to model function tools
    converts model tool calls to ToolInvocation

crates/public_mcp
  Protocol adapter:
    converts ToolDescriptor to rmcp Tool
    converts tools/call to ToolInvocation
    maps MCP compatibility settings to PermissionPolicy

crates/onetcli_runtime
  Business tools:
    SSH / SFTP / DB / Redis / Terminal / Connections / Workspaces
    all implement or adapt to tool_runtime handlers

crates/ai_chat_view
  Product UI:
    resource pool management
    default target picker
    approval cards
    tool result cards

main/src/public_mcp_runtime
  App integration:
    builds unified catalog from app state
    maps settings to PermissionPolicy
```

## Core Models

### Tool Identity

`ToolId` is the canonical internal id. It preserves dotted names such as `db.query`, `sftp.read`, and `ssh.exec`.

Agent-facing model APIs that require OpenAI-safe function names can derive transport names from `ToolId`, but the runtime stores and audits the canonical id.

```rust
pub struct ToolId(String);
```

Canonical names:

```text
ssh.exec
ssh.list_sessions
ssh.command.poll
ssh.command.output
ssh.command.cancel

sftp.list
sftp.read
sftp.write
sftp.stat
sftp.upload
sftp.download

db.query
db.exec
db.schema
db.tables
db.describe_table

redis.command
redis.keys
redis.get
redis.set

connections.list
connections.show
connections.open_session
workspaces.list
workspaces.show
```

Compatibility aliases:

```text
ssh_remote_exec       -> ssh.exec
ssh.remote_exec       -> ssh.exec
ssh.remote_command_poll -> ssh.command.poll
ssh.remote_command_output -> ssh.command.output
ssh.remote_command_cancel -> ssh.command.cancel

db_query              -> db.query
db.query              -> db.query
db_execute_sql        -> db.exec

ssh_list_dir          -> sftp.list
ssh_read_file         -> sftp.read
ssh_write_file        -> sftp.write
ssh_file_stat         -> sftp.stat
```

### SSH Command Surface

`ssh.exec` should match the way users type commands in the terminal. The Agent-facing
schema keeps the command as one shell line and uses `target` only to select the
resource:

```json
{
  "target": "ssh-prod-a",
  "command": "df -h && echo \"===INODE===\" && df -i",
  "cwd": "/root",
  "timeout_ms": 60000
}
```

The command string is displayed unchanged in approval cards and tool result cards.
Adapter-specific details such as `session_id`, PTY management, command polling, and
output collection stay behind the adapter boundary. Historical fields are still
accepted by compatibility adapters in this precedence order:

```text
target > connection > connection_id > session_id > default_target
```

This keeps Agent behavior consistent with terminal input: if a user can paste the
same command into the terminal, the model should be able to call `ssh.exec` with that
command text and an explicit resource target.

### ToolDescriptor

`ToolDescriptor` describes one canonical tool.

```rust
pub struct ToolDescriptor {
    pub id: ToolId,
    pub title: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub annotations: ToolAnnotations,
    pub target: ToolTargetSpec,
    pub origin: ToolOrigin,
    pub aliases: Vec<ToolAlias>,
}

pub struct ToolAnnotations {
    pub read_only: bool,
    pub destructive: bool,
    pub open_world: bool,
    pub idempotent: bool,
    pub supports_parallel: bool,
    pub risk: RiskLevel,
}

pub enum ToolOrigin {
    Builtin,
    Database,
    Ssh,
    Sftp,
    Redis,
    Terminal,
    PublicMcp,
    ExternalMcp,
    Acp,
    Cli,
}

pub struct ToolTargetSpec {
    pub supported_kinds: Vec<ResourceKind>,
    pub required: bool,
}
```

`origin` is for debugging and audit only. It must not be shown as the primary user-facing tool category.

### ResourcePool

`ResourcePool` replaces the product meaning of “current context”.

```rust
pub struct ResourcePool {
    pub default_target: Option<ResourceId>,
    pub resources: Vec<ResourceRef>,
}

pub struct ResourceRef {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub label: String,
    pub aliases: Vec<String>,
    pub scopes: Vec<ResourceScope>,
    pub capabilities: Vec<ResourceCapability>,
    pub origin: ResourceOrigin,
}
```

Semantics:

1. `default_target` is the target used when the user says “这台机器 / 当前连接 / 这个库”.
2. `resources` is the full set of resources the task may operate.
3. `default_target` does not restrict access to the rest of the pool.
4. Explicit target selection must match `id`, `label`, or `aliases`.
5. If the requested target is ambiguous, Agent must ask the user.
6. If the requested target is not in the resource pool, Agent must not guess or use hidden resources.

Resource matching behavior:

```text
用户说“这台机器”:
  use default_target

用户说“A 机器”:
  match id / label / alias within resource pool

用户说“所有机器”:
  select all matching ResourceKind::Ssh resources

用户说“生产库和缓存”:
  select DB and Redis resources matching aliases / labels

目标不明确:
  ask the user before tool invocation
```

### ToolInvocation

All entry adapters eventually produce `ToolInvocation`.

```rust
pub struct ToolInvocation {
    pub tool_id: ToolId,
    pub arguments: serde_json::Value,
    pub target: Option<ResourceTarget>,
    pub resources: ResourcePool,
    pub permission: PermissionPolicy,
    pub caller: ToolCaller,
    pub audit: AuditContext,
    pub cancellation: CancellationToken,
}
```

Agent-facing tools should use `target` as the unified target parameter:

```json
{
  "target": "prod-a",
  "command": "df -h"
}
```

Compatibility target resolution:

```text
target > connection > connection_id > session_id > default_target
```

The adapter normalizes legacy parameters into `ResourceTarget` before execution. Business tool handlers can still receive legacy fields during migration, but Agent prompt should only expose `target`.

### PermissionPolicy

Runtime permission is a single policy:

```rust
pub struct PermissionPolicy {
    pub mode: PermissionProfile,
    pub read_policy: OperationPolicy,
    pub write_policy: OperationPolicy,
    pub high_risk_policy: OperationPolicy,
    pub per_tool_overrides: HashMap<ToolId, OperationPolicy>,
    pub per_resource_overrides: HashMap<ResourceId, OperationPolicy>,
}

pub enum PermissionProfile {
    Safe,
    Confirm,
    Auto,
    Unrestricted,
}

pub enum OperationPolicy {
    Allow,
    Ask,
    Deny,
}
```

Product profiles:

```text
Safe:
  read-only allow
  write / destructive / open_world deny

Confirm:
  read-only allow
  write / destructive / open_world / high risk ask

Auto:
  read-only / low / medium allow
  high / critical / destructive / open_world ask

Unrestricted:
  allow by default
  critical overrides can still ask if configured
```

Compatibility mappings:

```text
Agent ToolExecutionMode::ReadOnly -> PermissionProfile::Safe
Agent ToolExecutionMode::Manual   -> PermissionProfile::Confirm
Agent ToolExecutionMode::Auto     -> PermissionProfile::Auto

MCP PermissionMode::Deny  -> PermissionProfile::Safe
MCP PermissionMode::Ask   -> PermissionProfile::Confirm
MCP PermissionMode::Allow -> PermissionProfile::Auto
```

This prevents the current class of bugs where Agent appears to allow a tool while MCP denies the underlying call.

### AuditEvent

Every executed or denied tool call emits a unified audit event:

```rust
pub struct AuditEvent {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub tool_id: ToolId,
    pub origin: ToolOrigin,
    pub target_resource: Option<ResourceId>,
    pub caller: ToolCaller,
    pub risk: RiskLevel,
    pub approval_status: ApprovalStatus,
    pub arguments_redacted: serde_json::Value,
    pub result_summary: Option<String>,
    pub started_at: String,
    pub finished_at: Option<String>,
}
```

The audit model must answer:

1. Who asked AI to operate which resources?
2. Which tool was called?
3. What command / SQL / file path was requested?
4. Was user approval required?
5. Was approval granted?
6. What was the result summary?

## Agent Flow

Final Agent turn:

1. Session creates a `ResourcePool` snapshot.
2. `ToolCatalog` filters descriptors by `ResourcePool` and `PermissionPolicy`.
3. Prompt injects:
   - resource pool
   - default target
   - canonical tool list
   - target selection rules
   - permission summary
4. Model emits a canonical tool call and optional `target`.
5. Agent adapter converts model function call to `ToolInvocation`.
6. `ToolRouter` resolves alias, target, permission, and risk.
7. If policy returns `Ask`, runtime emits `ApprovalRequest`.
8. UI approval continues the same turn.
9. `ToolResult` and `AuditEvent` are written to transcript and audit log.

Prompt changes:

1. Rename “当前可操作资源” to “资源池”.
2. Explicitly explain “默认目标” and “可操作资源池”.
3. Tell the model to use `target` for tools.
4. Tell the model not to guess resources outside the pool.
5. For multi-resource requests, instruct the model to call the same tool once per explicit resource unless a tool supports batch input.

## MCP Flow

Final MCP server:

1. `tools/list` reads from the same `ToolRegistry`.
2. MCP adapter converts `ToolDescriptor` to `rmcp::Tool`.
3. MCP compatibility aliases may be exposed depending on protocol version and client compatibility mode.
4. `tools/call` converts MCP arguments to `ToolInvocation`.
5. Permission and approval use the same `PermissionPolicy`.
6. Result conversion is only format adaptation: `ToolResult -> CallToolResult`.

`public_mcp` keeps:

1. loopback runtime
2. discovery file
3. token handshake
4. stdio bridge
5. MCP protocol implementation

`public_mcp` should not own business permission logic after migration.

## UI Flow

The AI context panel becomes a resource pool panel.

UI concepts:

```text
资源池
  默认目标: prod-a

搜索资源...
类型筛选: 全部 / SSH / DB / Redis / Terminal / SFTP

已授权资源
[x] prod-a      ssh      默认
[x] prod-b      ssh
[x] prod-c      ssh
[ ] staging-db  mysql
[ ] redis-prod  redis

Actions:
  设为默认目标
  加入资源池
  移出资源池
  查看作用域
```

Side panel default:

1. Include the current connection only.
2. Set it as `default_target`.
3. Allow adding more resources when the user expands the pool.

Normal Agent tab default:

1. Can load resources from all connections, workspace, tag, or manual selection.
2. The default target is the current connection when one exists.

Approval cards show:

1. tool id
2. target resource label and id
3. risk level
4. permission reason
5. redacted argument summary

Tool result cards group repeated calls by target.

## Error Handling

Runtime errors should be structured and consistent across adapters:

```text
unknown_tool
ambiguous_tool_alias
unsupported_adapter
missing_required_target
target_not_in_resource_pool
ambiguous_target
target_kind_not_supported
permission_denied
approval_denied
invalid_arguments
execution_failed
cancelled
```

Target resolution must fail closed:

1. Missing target with no default target: ask or error.
2. Ambiguous target: ask or error.
3. Target outside resource pool: deny.
4. Target kind not supported by tool: deny.

## Migration Strategy

### Phase 1: Core Models

Scope:

1. Extend `crates/tool_runtime` with final core models.
2. Preserve existing `ToolRegistry`, `ToolHandler`, and `ToolDescriptor` compatibility.
3. Add unit tests for resource pool target resolution, alias resolution, and permission decisions.
4. Do not migrate business tools yet.
5. Do not change UI or Agent behavior yet.

Acceptance:

1. Existing `tool_runtime` tests still pass.
2. Existing `public_mcp` adapter still compiles against `tool_runtime`.
3. New tests prove:
   - first resource can be default target
   - default target is not a resource pool boundary
   - id / label / alias target matching works
   - ambiguous target is rejected
   - safe / confirm / auto / unrestricted profiles decide as specified
   - descriptor aliases map to canonical ids

### Phase 2: Agent Adapter

Scope:

1. Add an adapter from `tool_runtime::ToolDescriptor` to Agent model tools.
2. Add an adapter from Agent model tool calls to `ToolInvocation`.
3. Keep existing `agent_runtime::Tool` as compatibility bridge during migration.
4. Update Agent prompt to use resource pool / default target / target field language.

Acceptance:

1. Existing Agent tests pass.
2. Agent prompt tests cover multi-resource semantics.
3. Agent can see canonical tool ids.
4. Legacy Agent tools still run through compatibility bridge.

### Phase 3: Business Tool Migration

Migration order:

1. DB read tools
2. SFTP read tools
3. SSH / remote read tools
4. write and high-risk tools
5. Redis tools
6. Connections and workspaces tools

Scope:

1. Move Agent-specific DB / SSH tools toward canonical `tool_runtime` descriptors.
2. Add aliases for old Agent names.
3. Normalize `target` to existing `connection` / `session_id` inputs internally.

Acceptance:

1. Old tool names still call the same functionality.
2. New canonical tool names also call the same functionality.
3. Agent prompt only exposes canonical names.
4. High-risk tools produce unified approval requests.

### Phase 4: Public MCP Adapter

Scope:

1. Make MCP `tools/list` derive from unified catalog.
2. Make MCP `tools/call` create `ToolInvocation`.
3. Migrate MCP permission settings to `PermissionPolicy`.
4. Keep compatibility aliases for existing external clients.

Acceptance:

1. MCP protocol tests pass.
2. External MCP clients can call old aliases when compatibility mode is enabled.
3. Agent and MCP no longer disagree on permission decisions for the same tool.

### Phase 5: Resource Pool UI

Scope:

1. Rename context panel concepts to resource pool and default target.
2. Support search, type filtering, multi-select, default target selection.
3. Side panel remains single-resource by default, with explicit expansion.
4. Normal Agent tab supports workspace / all / tag / manual resource sets.

Acceptance:

1. A user can add multiple SSH resources to one Agent session.
2. The default target is visible and changeable.
3. Removing a resource from the pool prevents Agent from targeting it.

### Phase 6: Multi-Resource Execution Experience

Scope:

1. `ToolRouter` supports safe parallel execution for tools with `supports_parallel`.
2. Agent can plan multi-resource tasks using explicit targets.
3. UI groups tool result cards by target.
4. Approval can be per-call or batched when all calls share the same risk and tool.

Acceptance:

1. “检查这 3 台机器磁盘空间并汇总” completes in one Agent turn.
2. Each tool call clearly shows its target machine.
3. High-risk multi-resource operations require confirmation.

## Key Acceptance Scenarios

1. Single connection side panel
   - User asks “看下磁盘”.
   - Agent uses the default SSH target.
   - No resource selection is required.

2. Multiple SSH resources
   - User asks “检查 A/B/C 磁盘”.
   - Agent calls the SSH tool with three explicit targets.
   - Final answer summarizes per-machine results.

3. DB + SSH workflow
   - User asks “查数据库慢查询，再去对应服务器看负载”.
   - Agent first targets DB, then targets SSH resources.

4. Safe profile
   - `df -h` and `SELECT` are allowed.
   - `rm`, `UPDATE`, and `sftp.write` are denied.

5. Confirm profile
   - High-risk tools show approval cards.
   - Approved calls continue the original turn.

6. Public MCP client
   - MCP calls use the same tool directory.
   - Permission outcome matches Agent behavior for the same policy.

## Testing Strategy

Phase 1 tests:

1. `cargo test -p tool_runtime`
2. Resource pool unit tests.
3. Permission policy unit tests.
4. Alias resolution unit tests.
5. Compatibility tests for existing registry behavior.

Later phase tests:

1. `cargo test -p agent_runtime`
2. `cargo test -p public_mcp`
3. `cargo test -p onetcli_runtime`
4. Targeted UI state tests for resource pool builders.
5. Integration-style tests for Agent prompt and approval flow.

Manual smoke scenarios after UI migration:

1. Single SSH side panel asks for disk usage.
2. Multi-SSH Agent tab checks disk usage on three resources.
3. Confirm profile blocks and resumes a high-risk operation.
4. MCP client calls a compatibility alias and canonical id.

## Risks And Mitigations

Risk: Big-bang refactor breaks Agent, MCP, CLI, and UI together.

Mitigation: Phase 1 only adds `tool_runtime` core contract and tests. Business migrations happen one tool family at a time.

Risk: Existing external MCP clients rely on old tool names.

Mitigation: Keep aliases and optionally expose compatibility names through MCP adapter.

Risk: Dotted canonical ids are not valid function names for some model APIs.

Mitigation: Agent adapter derives transport-safe names while preserving canonical `ToolId` internally and in audit.

Risk: `target` migration conflicts with existing `connection` / `session_id` schemas.

Mitigation: Use precedence `target > connection > connection_id > session_id > default_target` during migration. Prompt only exposes `target` after adapter support exists.

Risk: Permission profiles are too broad for advanced users.

Mitigation: Keep product profiles simple but support per-tool and per-resource overrides in `PermissionPolicy`.

Risk: `tool_runtime` gains UI or business dependencies.

Mitigation: Keep `tool_runtime` as a pure core crate. Resource providers and business handlers live in `onetcli_runtime`, `terminal_view`, `db`, `ssh`, `sftp`, and app integration crates.

## First Implementation Plan Boundary

The first implementation plan must target Phase 1 only.

Allowed first-plan files:

```text
crates/tool_runtime/src/lib.rs
crates/tool_runtime/src/ids.rs
crates/tool_runtime/src/resource.rs
crates/tool_runtime/src/descriptor.rs
crates/tool_runtime/src/invocation.rs
crates/tool_runtime/src/permission.rs
crates/tool_runtime/src/audit.rs
crates/tool_runtime/src/router.rs
crates/tool_runtime/tests/*.rs
```

Out of scope for the first implementation plan:

```text
crates/agent_runtime
crates/public_mcp
crates/onetcli_runtime business tool behavior
crates/ai_chat_view UI behavior
main settings migration
```

The first code change must leave existing adapters compiling and passing their current tests.

## Spec Self-Review

Marker scan:

1. No unfinished markers remain.
2. Each migration phase has explicit scope and acceptance criteria.

Consistency check:

1. `tool_runtime` owns core models, permission, approval, audit, registry, and router.
2. Agent, MCP, CLI, and UI are adapters.
3. `default_target` is consistently described as a default, not a boundary.

Scope check:

1. The complete architecture is broad, so implementation is intentionally phased.
2. The first implementation plan is scoped to Phase 1 and is testable on its own.

Ambiguity check:

1. Target resolution precedence is explicit.
2. Permission profile mapping from existing Agent and MCP settings is explicit.
3. Compatibility alias behavior is explicit.
