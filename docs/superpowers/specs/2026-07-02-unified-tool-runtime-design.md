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
   - 目前缺少资源池、统一目标解析、统一权限、审批、审计和 invocation contract。

2. `crates/agent_runtime`
   - 仍有自己的 `ResourceContext`、`ToolSpec`、`ToolRegistry`、`ToolInvocation`、`ToolRouter`。
   - Agent 工具名已经对已迁移工具收敛到 runtime canonical id 的 function-name
     归一化形式，例如 `db.exec` -> `db_exec`、`sftp.read` -> `sftp_read`。
   - Agent 权限模式是 `ToolExecutionMode::Auto / ReadOnly / Manual`。

3. `crates/public_mcp`
   - 有自己的 `PublicMcpToolRegistry`、`PublicMcpToolProvider`、`PermissionMode::Deny / Ask / Allow`。
   - 已有 `ToolRuntimeMcpProvider` 可以将 `tool_runtime::ToolRegistry` 暴露为 MCP tools。
   - 仍存在 MCP 侧权限判断和工具 provider 聚合语义。

UI 侧的 `ResourceContext.current` 已接近默认目标语义，但产品文案仍是“上下文 / 当前资源”，不是“默认目标 / 资源池”。

## Migration Tracking

This section is the checkpoint for future workers. Update it whenever a phase is
started, finished, split, or blocked.

Last updated: 2026-07-03

Tracking rules:

1. Keep this section as the global source of truth for migration status.
2. Mark a row `Done` only after the code or doc checkpoint is committed and targeted
   verification has been run.
3. Mark a row `In progress` when relevant changes exist in the worktree, when a
   checkpoint has partial verification, or when manual smoke is still pending.
4. Add a new row when a phase splits into a separately testable checkpoint.
5. Keep the current active checkpoint explicit so future workers can resume without
   rereading the whole conversation.

Status labels:

```text
Done        Code is committed and targeted verification passed.
In progress Code exists or is being changed, but the checkpoint is not committed or fully verified.
Planned     Scope is documented, but implementation has not started.
Blocked     Work cannot continue without a product or technical decision.
```

| Area | Status | Evidence | Next checkpoint |
| --- | --- | --- | --- |
| Phase 1 core models | Done | `39fa5d7a feat(tool_runtime): add unified core models` | Keep compatibility while later phases consume the models. |
| Phase 2 Agent adapter contracts | Done | `f08ea51d feat(agent_runtime): add tool runtime adapter contracts` | Continue replacing legacy Agent tool implementations by family. |
| Phase 2b Agent registry bridge | Done | `7589fe89 feat(agent_runtime): bridge tool runtime registry` | Use the bridge for each migrated business tool family. |
| Phase 3a DB read tools | Done | `bdc88e56 feat(agent): bridge database read tools through tool runtime` | Migrate write/high-risk DB operations only after unified approval is ready. |
| Phase 3b SSH structured command tools | Done | `1be59bb2 feat(public_mcp): canonicalize ssh command tools` | Keep `ssh.exec` structured and non-interactive. |
| Phase 3c `terminal.exec` runtime contract | Done | `f0777d07 feat(public_mcp): add terminal exec runtime contract` | Wire live terminal providers and validate terminal input behavior. |
| Phase 3c live terminal provider | Done | `crates/terminal` exposes `TerminalInputHandle`; `crates/terminal_view` registers a `TerminalExecSessionHandle` that writes `command + "\n"` into the live terminal input path. Targeted provider tests and Public MCP/runtime checks passed on 2026-07-02. | Start Agent/UI prompt, approval-card, and tool-card integration. |
| Phase 3c Agent/UI terminal exec selection | Done | Agent prompt tells the model to use `terminal_exec` for visible terminal execution and `ssh_exec` for structured SSH execution; tool and approval card titles label `terminal_exec` as terminal execution. `agent_runtime` tests and `ai_chat_view` checks passed on 2026-07-02. | Run manual app smoke for visible terminal execution. |
| Phase 3d Redis canonical command tool | Done | `0573668 feat(onetcli_runtime): canonicalize redis command tool` made `redis.command` the canonical `onetcli_runtime` Redis tool id. It initially kept `redis.execute_command` as a runtime alias, but that compatibility path was later removed by Phase 3i. Red/green Redis tests, `cargo check -p onetcli_runtime`, and `git diff --check` passed on 2026-07-03. | Add read-oriented Redis convenience tools such as `redis.keys`, `redis.get`, and `redis.set` only when their schemas and permission/risk contracts are explicit. |
| Phase 3e Redis convenience tools | Done | `0623ec8 feat(onetcli_runtime): add redis convenience tools` adds canonical `redis.keys`, `redis.get`, and `redis.set` to `onetcli_runtime`, keeps `redis.keys/get` read-only, requires write permission for `redis.set`, and splits Redis runtime implementation into submodules under 300 lines each. Red/green Redis tests, `cargo test -p onetcli_runtime`, `cargo check -p onetcli_runtime`, and `git diff --check` passed on 2026-07-03. | Consider Public MCP convenience Redis tools only after deciding whether external clients should get the same high-level commands or only the generic `redis.command`. |
| Phase 3f Agent Redis runtime bridge | Done | `9073061 feat(agent): bridge redis runtime tools` registered `onetcli_runtime::redis_tools::redis_tool_registry(repo)` through the Agent `tool_runtime` bridge. Agent initially kept legacy `redis_execute_command`; that compatibility path was later removed by Phase 3i. TDD red/green registry tests, Agent runtime adapter tests, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Keep Redis Agent surface canonical-only: `redis_command`, `redis_keys`, `redis_get`, `redis_set`. |
| Phase 3g Agent SFTP runtime bridge | Done | `6b4236a feat(agent): bridge sftp runtime tools` registered `onetcli_runtime::sftp_tools::sftp_tool_registry(repo)` through the Agent `tool_runtime` bridge. Agent initially kept legacy `ssh_*` file tools; that compatibility path was later removed by Phase 3i. Red/green registry tests, SFTP runtime tests, Agent runtime adapter tests, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Keep SFTP Agent surface canonical-only: `sftp_list`, `sftp_read`, `sftp_write`, `sftp_stat`, `sftp_upload`, `sftp_download`. |
| Phase 3h Agent DB exec runtime bridge | Done | `e48fd27 feat(agent): bridge database exec runtime tool` switched the Agent DB bridge from `database_read_tool_registry(repo)` to the full `database_tool_registry(repo)`. Agent initially kept legacy `db_execute_sql`; that compatibility path was later removed by Phase 3i. Red/green registry tests, DB runtime tests, Agent runtime adapter tests, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Keep DB write execution canonical-only through `db.exec`. |
| Phase 3i Canonical-only tool surface | Done | `dae1dab feat(tools): remove legacy tool aliases` removes legacy DB/SFTP native Agent modules, stops registering old Redis Agent tools, removes `redis.execute_command` and `ssh.remote_*` aliases, and updates Agent prompt rules to use canonical runtime-derived function names only. Full `agent_runtime`, `onetcli_runtime`, and `public_mcp` tests plus targeted main registry tests and checks passed on 2026-07-03. | Continue adding missing capabilities only as canonical `tool_runtime` tools; do not add compatibility aliases for old names. |
| Phase 3j DB metadata canonical tools | Done | `c8de98c feat(database): add canonical metadata tools` adds `db.tables`, `db.describe_table`, and `db.sample_rows` to `onetcli_runtime::database_tools`, exposes them through the Agent runtime bridge as `db_tables`, `db_describe_table`, and `db_sample_rows`, and keeps them read-only. `cargo test -p onetcli_runtime --test database_tools`, `cargo test -p main agent_runtime_tool_registry`, `cargo test -p onetcli_runtime`, `cargo check -p onetcli_runtime`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Keep adding missing capabilities only as canonical runtime tools. |
| Phase 3k Agent target adapter | Done | `edc2e2c feat(agent): expose runtime targets through target` makes runtime-backed Agent tools expose `target` instead of provider fields such as `connection`, `connection_id`, and `session_id`; maps `target` or the default resource back to the provider field before calling the runtime handler; rejects provider target fields at the Agent adapter boundary; and updates the Agent resource prompt to say resource pool/default target. `cargo test -p agent_runtime`, `cargo test -p main agent_runtime_tool_registry`, `cargo check -p agent_runtime`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Continue moving Public MCP/CLI/runtime invocation contracts toward first-class `target` once each adapter can resolve resource pools. |
| Phase 4 Public MCP app registry merge | Done | `main/src/public_mcp_runtime/tool_registry.rs` collects enabled `tool_runtime::ToolRegistry` values, merges them, and exposes one `ToolRuntimeMcpProvider`. Terminal toolset now exposes both `ssh.exec` and `terminal.exec` through the real app registry path. Public MCP runtime tests passed on 2026-07-02. | Continue replacing remaining Public MCP-specific permission settings with unified `PermissionPolicy`. |
| Phase 4b Public MCP Redis canonical command tool | Done | `6c99c8e feat(public_mcp): canonicalize redis command tool` made Public MCP Redis `tools/list` expose `redis.command`. It initially accepted `redis.execute_command`; that alias was later removed by Phase 3i. `public_mcp` Redis tests, main registry Redis test, `cargo check -p public_mcp`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Keep Public MCP Redis surface canonical-only. |
| Phase 4c Public MCP Redis convenience tools | Done | `6d5fc34 feat(public_mcp): add redis convenience tools` exposes `redis.keys`, `redis.get`, and `redis.set` through Public MCP and the real app registry path. `redis.keys/get` are read-only and `redis.set` is mutating and approval-gated. The old `redis.execute_command` alias was later removed by Phase 3i. Red/green Public MCP convenience tests, `cargo test -p public_mcp`, main Redis registry test, `cargo check -p public_mcp`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Keep Public MCP Redis surface canonical-only. |
| Phase 4d Public MCP target adapter | Done | `3a558e6 feat(public_mcp): expose runtime targets through target` makes runtime-backed MCP tools expose `target` instead of provider fields such as `connection`, `connection_id`, and `session_id`; rejects those provider fields at the MCP adapter boundary; and maps `target` back to the current handler field only as an internal migration adapter. `cargo test -p public_mcp --test tool_runtime_target_adapter`, `tool_runtime_adapter`, `redis_tools`, `redis_convenience_tools`, `remote_ops`, `internal_functions`, `cargo check -p public_mcp`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Add MCP resource-pool id/label/alias resolution and continue moving CLI/runtime-core invocation paths toward first-class `target`. |
| Phase 4e Public MCP resource-pool target resolution | Done | `f1f92f7 feat(public_mcp): resolve runtime targets from resource pool` adds an optional `ResourcePool` snapshot to `ToolRuntimeMcpProvider`, resolves MCP `target` by resource id / label / alias before mapping to the handler field, and rejects ambiguous resource targets. `cargo test -p public_mcp --test tool_runtime_target_adapter`, `tool_runtime_adapter`, `redis_tools`, `redis_convenience_tools`, `remote_ops`, `internal_functions`, `cargo check -p public_mcp`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Real app saved-connection pool wiring is covered by Phase 4f; next expand active session resources. |
| Phase 4f Public MCP app resource pool | Done | `f6611e2 feat(public_mcp): build app resource pool` builds a saved-connection `ResourcePool` from the real app `ConnectionRepository`, attaches it to the merged `ToolRuntimeMcpProvider`, maps saved connection ids/names/host aliases to runtime targets, and proves a DB tool can be called through a host alias. `cargo test -p main public_mcp_runtime::tool_registry`, `cargo test -p public_mcp --test tool_runtime_target_adapter`, `tool_runtime_adapter`, `redis_tools`, `redis_convenience_tools`, `remote_ops`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Extend app resource pools to active terminal sessions and active Redis connection snapshots, not only saved connections. |
| Phase 4g Agent approval for Public MCP runtime tools | Done | `62e696a fix(agent): require approval for public mcp runtime tools` makes the Public MCP -> Agent adapter derive Agent risk from MCP annotations, so destructive/open-world runtime tools such as `ssh.exec` and `terminal.exec` produce Agent approval requests in Auto mode. After the user approves, the adapter calls the underlying Public MCP/runtime tool with an internal approved context instead of letting the external MCP permission mode silently deny the already-approved Agent call. `cargo test -p public_mcp --test agent_runtime_adapter`, `cargo test -p public_mcp agent_runtime_adapter`, `cargo test -p public_mcp --test tool_runtime_adapter`, `cargo test -p agent_runtime high_risk`, `cargo test -p public_mcp --test protocol remote_exec`, `cargo test -p main agent_runtime_tool_registry`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Run manual UI smoke that `ssh_exec` / `terminal_exec` show confirmation cards and continue after approval. |
| Phase 4h Live terminal target resolution | Done | `d1709b4 fix(public_mcp): resolve live terminal targets` changes `ToolRuntimeMcpProvider` from a startup resource-pool snapshot to a call-time `ResourcePoolProvider`, adds active terminal sessions to the real app resource pool, marks `ssh.exec`, `ssh.session_diagnostics`, and `terminal.exec` as `ResourceKind::Terminal` targets, and resolves saved SSH ids, host/IP aliases, and prompt-like strings such as `root@zn-54:~` through linked active terminal sessions. Agent `ResourceContext` now carries aliases and the Agent runtime adapter uses `ToolTargetSpec` for kind-aware resolution. `cargo test -p public_mcp --test tool_runtime_target_adapter`, `cargo test -p agent_runtime --test tool_runtime_target_adapter`, `cargo test -p main public_mcp_runtime::tool_registry`, `cargo test -p tool_runtime`, `cargo test -p ai_chat_view resource_builder`, `cargo test -p public_mcp --test tool_runtime_adapter`, `cargo test -p public_mcp --test remote_ops`, `cargo test -p agent_runtime --test tool_runtime_adapter`, `cargo test -p public_mcp --test agent_runtime_adapter`, `cargo check -p public_mcp`, `cargo check -p main`, and `git diff --check` passed on 2026-07-03. | Extend the same dynamic resource-pool provider pattern to active Redis snapshots and any future app-local resources. |
| Phase 4 Public MCP runtime permission policy | Done | `PermissionMode` now maps to `tool_runtime::PermissionPolicy` for runtime-backed MCP tools. `Allow` maps to Auto, so high-risk/destructive/open-world tools such as `ssh.exec`, `terminal.exec`, and generic Redis command execution still require approval. Public MCP and app runtime tests passed on 2026-07-02. | Migrate settings/UI terminology from MCP permission mode to unified permission profile. |
| Phase 4 Public MCP settings profile wording | Done | `McpPermissionMode` keeps old persisted values but exposes profile ids `safe/confirm/auto`; Public MCP runtime config carries `permission_profile`; settings UI labels now show Safe / Confirm / Auto. Core settings and app runtime tests passed on 2026-07-02. | Later storage migration can replace `permission_mode` only when a broader settings migration is planned. |
| Phase 5a Resource Pool UI wording/filtering | Done | `6010dad7 feat(ai_chat): add resource pool display model`, `e8985154 feat(ai_chat): rename context selector to resource pool`, `d7b82ab0 feat(ai_chat): filter resource pool by type`, `84d8fe65 test(ai_chat): document resource pool default target semantics`, `ad195aab docs: track resource pool ui checkpoint` | Keep wording and default-target semantics while wiring broader catalogs. |
| Phase 5b Resource Pool membership | Done | `1e980c66 feat(ai_chat): add resource pool item display model`, `f271dc59 feat(ai_chat): add available resource catalog`, `07a8b217 feat(ai_chat): map resource catalog to pool rows`, `666b55a2 feat(ai_chat): render resource pool membership actions`, `f7099082 feat(ai_chat): handle resource pool membership changes`, `cf3babff feat(ai_chat): build resource catalog for pool management`, `00094e15 docs: track resource pool management checkpoint` | Wire real entry points so sidebars pass broader catalogs instead of only the selected pool. |
| Phase 5c Sidebar catalog wiring | Done | `a2cd12e feat(ai_chat): wire resource catalog into sidebars` wires DB, Redis, and Mongo sidebars through catalog-aware default panel APIs. Focused `ai_chat_view` tests and checks for `ai_chat_view`, `db_view`, `redis_view`, and `mongodb_view` passed on 2026-07-02. | Start Phase 5d resource source presets or run manual resource-pool smoke. |
| Phase 5d Resource source presets | Done | `9f10d13 feat(ai_chat): add resource source option model`, `cd8839d feat(ai_chat): derive resource source options`, `f90e41b feat(ai_chat): apply resource source presets`, and `a78388c feat(ai_chat): render resource source presets` add current/all/type/manual source presets while keeping workspace/tag disabled until real metadata exists. `resource_source`, `resource_pool`, and `cargo check -p ai_chat_view` passed on 2026-07-02. | Run manual resource-pool smoke and keep persisted workspace/tag presets for a later storage-backed checkpoint. |
| Phase 6 Multi-resource execution | Done | `23cbec8 feat(agent): batch executable tool calls`, `29e6f01 feat(agent): dispatch parallel-safe tool batches`, `4743915 test(agent): preserve parallel tool safety semantics`, `2172776 feat(ai_chat): show tool result target resources`, `d51e7a2 feat(ai_chat): group tool results by target`, `16ab3e3 feat(agent): batch sibling high risk approvals`, and `a6bc8cf feat(ai_chat): render batched tool approvals` add safe parallel dispatch, preserve approval gating, keep observation order deterministic, show target resource labels on tool result cards, fold consecutive same-target tool results under a target header, let one approval resume same-response sibling high-risk tool calls, and render batched approval cards with per-call summaries. `agent_runtime` Phase 6 checks passed on 2026-07-02; `ai_chat_view` target grouping, batched approval UI, and `agent_runtime` batched approval checks passed on 2026-07-03. | Run manual terminal/resource-pool smoke and continue migrating remaining tool families through `tool_runtime`. |

### Active Checkpoint

Current checkpoint: Phase 3c manual terminal-exec smoke, Agent approval smoke, and remaining tool-runtime migration.

Purpose:

1. Verify `terminal.exec` writes into the visible terminal pane with real app behavior.
2. Verify `ssh_exec` and `terminal_exec` show Agent confirmation cards before high-risk
   execution and continue after approval.
3. Run manual resource-pool smoke for source presets.
4. Keep side-panel sessions single-resource by default while allowing explicit expansion
   from a broader catalog.
5. Keep persisted workspace/tag presets deferred until a real workspace/tag resource
   catalog source exists.
6. Avoid guessing a terminal catalog until `TerminalSidebar` has a real full-connection
   source.

Last completed checkpoint:

```text
d1709b4 fix(public_mcp): resolve live terminal targets
```

Last checkpoint verification run:

```bash
rtk cargo test -p public_mcp --test tool_runtime_target_adapter
rtk cargo test -p agent_runtime --test tool_runtime_target_adapter
rtk cargo test -p main public_mcp_runtime::tool_registry
rtk cargo test -p tool_runtime
rtk cargo test -p ai_chat_view resource_builder
rtk cargo test -p public_mcp --test tool_runtime_adapter
rtk cargo test -p public_mcp --test remote_ops
rtk cargo test -p agent_runtime --test tool_runtime_adapter
rtk cargo test -p public_mcp --test agent_runtime_adapter
rtk cargo check -p public_mcp
rtk cargo check -p main
rtk git diff --check
```

Result: all commands exited 0. `cargo check -p main` still reports the existing
`block v0.1.6` future-incompat warning, which can remain.

Current product decision:

1. `ssh.exec` remains the structured non-interactive SSH command tool.
2. `terminal.exec` is the new live terminal-surface tool for “像在终端里输入一样执行”.
3. 已迁移工具不再保留旧工具名或旧 alias；旧名称应直接失败，而不是静默转发。
4. `redis.command` is the canonical Redis command tool in `onetcli_runtime` and
   Public MCP; `redis.execute_command` is no longer accepted.
5. `redis.keys` and `redis.get` are read-only Redis tools in `onetcli_runtime` and
   Public MCP; `redis.set` is mutating and requires write permission / approval.
6. Agent registry now includes runtime-backed Redis tools through the `tool_runtime`
   bridge. The model-facing function names are normalized as `redis_command`,
   `redis_keys`, `redis_get`, and `redis_set`; the canonical runtime ids remain
   `redis.command`, `redis.keys`, `redis.get`, and `redis.set`.
7. Agent registry now includes runtime-backed SFTP tools through the `tool_runtime`
   bridge. The model-facing function names are normalized as `sftp_list`,
   `sftp_read`, `sftp_write`, `sftp_stat`, `sftp_upload`, and `sftp_download`;
   the canonical runtime ids remain `sftp.list`, `sftp.read`, `sftp.write`,
   `sftp.stat`, `sftp.upload`, and `sftp.download`.
8. Agent registry now includes runtime-backed DB write execution through the
   `tool_runtime` bridge. The model-facing function name is `db_exec`; the canonical
   runtime id remains `db.exec`. Legacy `db_execute_sql` is no longer registered.
9. Agent registry now includes runtime-backed DB metadata tools through the
   `tool_runtime` bridge. The model-facing function names are `db_tables`,
   `db_describe_table`, and `db_sample_rows`; the canonical runtime ids remain
   `db.tables`, `db.describe_table`, and `db.sample_rows`.
10. Runtime-backed Agent tool schemas expose `target` instead of provider-specific
   fields such as `connection`, `connection_id`, or `session_id`. The Agent adapter
   maps `target` or the default resource back to the provider field before calling
   the runtime handler, and rejects those provider fields if the model sends them.
11. Runtime-backed Public MCP tool schemas expose `target` instead of provider-specific
   fields. The MCP adapter rejects provider fields from clients and maps `target`
   back to the provider field only inside the adapter while handlers are still being
   migrated.
12. `ToolRuntimeMcpProvider` can carry a call-time `ResourcePoolProvider`. MCP
   `target` is resolved by resource id, label, alias, tool-supported resource kind,
   and linked resources before being passed to the current runtime handler field.
   Ambiguous or unknown targets are rejected instead of guessed. Static snapshots
   are still supported only for tests and non-app adapters.
13. The real Public MCP app registry now attaches a dynamic app resource pool to the
   merged runtime provider. Saved connection ids, names, `cloud_id`, host/path aliases,
   and active terminal sessions can resolve MCP `target`. For terminal tools, a saved
   SSH connection target such as `21` or `10.2.4.54` resolves through the linked active
   terminal session when that terminal resource has the saved connection id as an alias.
14. Agent-facing prompts should expose canonical ids only after the relevant adapter can
   route them safely.
15. Agent-facing Public MCP adapter tools derive Agent approval risk from MCP
   annotations. Destructive/open-world tools must pause for Agent confirmation in Auto
   mode; after approval, the adapter treats that specific Agent path as approved so
   external MCP permission settings do not cause a second silent denial.

Next recommended checkpoints:

1. Run the Phase 3c manual smoke where a command appears in the visible terminal pane
   through `terminal.exec`.
2. Run manual Agent approval smoke for `ssh_exec` and `terminal_exec`.
3. Run manual resource-pool smoke for source presets.
4. Extend the real Public MCP app resource pool beyond saved connections and active
   terminal sessions to include active Redis connection snapshots.
5. Continue moving CLI and runtime-core invocation paths toward first-class `target`
   resolution rather than provider-specific fields.

## Design Principles

1. `tool_runtime` 是唯一真实执行层。
2. `agent_runtime` 负责会话、turn、prompt、计划、审批事件、transcript，不再长期定义业务 Tool trait。
3. `public_mcp` 负责 MCP 协议、transport、discovery、stdio bridge，不再长期拥有业务工具目录。
4. CLI 和 UI 只做入口适配、参数展示、结果展示和用户操作。
5. 所有业务能力统一注册到 `tool_runtime::ToolRegistry`。
6. Agent、MCP、CLI、UI 都从同一个 registry 派生工具列表。
7. 权限、资源、审批、审计、风险等级在 runtime core 统一处理。
8. `default_target` 只是默认目标，不是资源池边界。
9. 模型只看到 canonical tool id 派生出的 function name；旧工具名和旧 alias 不再兼容。
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

terminal.exec

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
db.sample_rows

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

Removed legacy aliases:

```text
ssh.remote_exec
ssh.remote_command_poll
ssh.remote_command_output
ssh.remote_command_cancel

db_execute_sql
redis.execute_command
redis_execute_command

ssh_list_dir
ssh_read_file
ssh_write_file
ssh_file_stat
```

These names should fail instead of being routed through compatibility aliases. If a
missing capability is still needed, add it as a new canonical `tool_runtime` tool.

### SSH Command Surface

`ssh.exec` is the structured, non-interactive SSH command tool. It should keep the
command text close to what users type in a terminal, but it does not write into the
visible terminal PTY and does not claim that the right-side terminal pane executed
the command. Its Agent-facing schema keeps the command as one shell line and uses
`target` only to select the resource:

```json
{
  "target": "ssh-prod-a",
  "command": "df -h && echo \"===INODE===\" && df -i",
  "cwd": "/root",
  "timeout_ms": 60000
}
```

The command string is displayed unchanged in approval cards and tool result cards.
Adapter-specific details such as `session_id`, command polling, and output collection
should stay behind the adapter boundary. The remaining schema cleanup is to make
Agent-facing tools accept `target` only, then let each adapter translate that target
into provider-specific connection/session identifiers:

```text
target -> ssh session / saved connection / terminal session
```

This keeps Agent behavior consistent with terminal input: if a user can paste the
same command into the terminal, the model should be able to call `ssh.exec` with that
command text and an explicit resource target. It does not provide the "executed in the
visible terminal" product effect.

### Terminal Execution Surface

`terminal.exec` is a separate tool for the product effect where Agent actions execute
inside an existing visible terminal session. This tool is additive: `ssh.exec` and
`ssh.command.*` remain available for structured remote execution, but legacy
`ssh.remote_exec` and `ssh.remote_command_*` aliases are no longer accepted.

`terminal.exec` writes the command into the target terminal session as if it were typed
by an operator, submits it with Enter, and observes terminal output from that same PTY
stream. The right-side terminal pane should show the command echo and output because the
terminal session is the execution surface.

Agent-facing schema:

```json
{
  "target": "terminal-ssh-prod-a",
  "command": "df -h && echo \"===INODE===\" && df -i",
  "submit": true,
  "wait_for_output": true,
  "timeout_ms": 60000
}
```

Semantics:

1. `target` must resolve to a `ResourceKind::Terminal` resource that is backed by an
   active terminal view/session.
2. `command` is inserted into the terminal input exactly as provided.
3. `submit=true` appends Enter after the command. If `submit=false`, the command is only
   staged in the terminal input and the tool result reports that no command was run.
4. `wait_for_output=true` waits for a bounded output snapshot or shell-integration
   completion signal when available. If no reliable completion signal exists, the result
   must say that the command was submitted and include the observed output delta, not
   fabricate an exit code.
5. Tool cards and audit records must clearly label the operation as "executed in
   terminal" and show the target terminal plus command.

Safety:

1. `terminal.exec` is high-risk/open-world by default because it writes into a live
   terminal session.
2. Approval details must show the exact command and target terminal before submission.
3. Audit must record that execution happened through the terminal surface, not the
   structured SSH executor.

This split avoids changing existing clients while supporting the requested terminal
execution experience.

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

Entry adapters must expose `target` only. They must reject provider-specific target
fields such as `connection`, `connection_id`, and `session_id` instead of accepting
them as compatibility aliases.

During migration, an adapter may map `target` back to the provider field required by
an existing handler, but that mapping is internal and must not be visible in Agent or
MCP-facing schemas.

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
6. `ToolRouter` resolves target, permission, and risk for the canonical tool id.
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
3. MCP adapter exposes canonical tool ids only; old aliases should return an unknown-tool error.
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
3. Add unit tests for resource pool target resolution, removed-alias rejection, and permission decisions.
4. Do not migrate business tools yet.
5. Do not change UI or Agent behavior yet.

Acceptance:

1. Existing `tool_runtime` tests still pass.
2. Existing `public_mcp` adapter still compiles against `tool_runtime`.
3. New tests prove:
   - first resource can be default target
   - default target is not a resource pool boundary
   - id / label / resource alias target matching works
   - ambiguous target is rejected
   - safe / confirm / auto / unrestricted profiles decide as specified
   - removed tool aliases return unknown-tool errors

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
4. `terminal.exec` live terminal execution tool
5. write and high-risk tools
6. Redis tools
7. Connections and workspaces tools

Scope:

1. Move Agent-specific DB / SSH tools toward canonical `tool_runtime` descriptors.
2. Do not add aliases for old Agent names; removed names must fail closed.
3. Normalize `target` to existing `connection` / `session_id` inputs internally.
4. Add `terminal.exec` as a new terminal-surface tool instead of changing `ssh.exec`
   into terminal UI execution.

Acceptance:

1. Removed tool names return unknown-tool errors instead of compatibility routing.
2. Canonical tool names call the same functionality through `tool_runtime`.
3. Agent prompt only exposes canonical names.
4. High-risk tools produce unified approval requests.
5. `terminal.exec` can execute through a visible terminal session without removing or
   changing structured `ssh.exec`.

### Phase 4: Public MCP Adapter

Scope:

1. Make MCP `tools/list` derive from unified catalog.
2. Make MCP `tools/call` create `ToolInvocation`.
3. Migrate MCP permission settings to `PermissionPolicy`.
4. Reject legacy aliases instead of silently routing them to canonical tools.

Acceptance:

1. MCP protocol tests pass.
2. External MCP clients receive a clear unknown-tool error for removed aliases.
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

3. Visible terminal execution
   - User asks “就在这个终端里执行 df -h”.
   - Agent uses `terminal.exec`, not `ssh.exec`.
   - The command appears in the terminal pane and output is produced by the same terminal session.
   - The tool card labels the call as terminal execution and does not hide that it wrote into a live terminal.

4. DB + SSH workflow
   - User asks “查数据库慢查询，再去对应服务器看负载”.
   - Agent first targets DB, then targets SSH resources.

5. Safe profile
   - `df -h` and `SELECT` are allowed.
   - `rm`, `UPDATE`, and `sftp.write` are denied.

6. Confirm profile
   - High-risk tools show approval cards.
   - Approved calls continue the original turn.

7. Public MCP client
   - MCP calls use the same tool directory.
   - Permission outcome matches Agent behavior for the same policy.

## Testing Strategy

Phase 1 tests:

1. `cargo test -p tool_runtime`
2. Resource pool unit tests.
3. Permission policy unit tests.
4. Removed-alias rejection tests.
5. Existing registry behavior tests.

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
4. MCP client calls a removed alias and receives an unknown-tool error; canonical id still works.

## Risks And Mitigations

Risk: Big-bang refactor breaks Agent, MCP, CLI, and UI together.

Mitigation: Phase 1 only adds `tool_runtime` core contract and tests. Business migrations happen one tool family at a time.

Risk: Existing external MCP clients rely on old tool names.

Mitigation: Treat the canonical-only surface as an intentional breaking change. Removed
tool ids return clear unknown-tool errors, and follow-up release notes should list the
canonical replacements.

Risk: Dotted canonical ids are not valid function names for some model APIs.

Mitigation: Agent adapter derives transport-safe names while preserving canonical `ToolId` internally and in audit.

Risk: `target` migration conflicts with existing `connection` / `session_id` schemas.

Mitigation: Entry adapters expose and accept `target` only. They reject
provider-specific target fields at the boundary, then map `target` to the current
handler field internally until the handler is target-native.

Risk: Live terminal execution can be confused with structured SSH execution.

Mitigation: Keep `terminal.exec` separate from `ssh.exec`. Tool cards, approvals, and audit
events must label `terminal.exec` as writing into a live terminal session. `terminal.exec`
must not fabricate an exit code when only terminal output was observed.

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
3. Canonical-only behavior for removed aliases is explicit.
