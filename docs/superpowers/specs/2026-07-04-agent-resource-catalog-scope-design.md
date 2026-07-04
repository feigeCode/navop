# Agent Resource Catalog/Scope Design

## Goal

把 AI 工作台和各业务侧边栏的资源模型拆成两层：全量可发现的 `ResourceCatalog`，以及当前任务实际可操作的 `AgentResourceScope`。目标是让工作台能 `@` / 打开 / 创建所有连接，同时避免“所有连接默认都进执行资源池”导致工具误打目标。

## Current Problem

当前实现把多个概念混在一起：

- `ResourceContext.resources` 既表示当前任务可操作资源，又被用作 prompt 上下文和工具执行池。
- `available_resources` 实际是 catalog，但只作为 `AgentChatViewConfig` 的一个 Vec 存在，生命周期和权限语义不清。
- `tool_runtime::ResourcePool` 既用于 target 解析，又被部分路径当成“全部连接/会话列表”。
- 默认目标由 `ResourceContext::with_resource` 的插入顺序隐式决定，第一个资源会自动成为 current。
- 工作台需要全量连接可见，但不应该因此让所有连接默认都成为当前任务执行目标。

这些问题导致两个典型错误：

- 为了让 `@` 有候选，把所有连接塞进 `ResourceContext`，模型/工具就可能无 target 地默认命中第一个连接。
- 为了保持侧边栏默认当前连接，又会让全量连接 catalog 和当前执行 scope 混在构造器里，入口稍有不慎就切错模式或丢功能。

## Design

### ResourceCatalog

`ResourceCatalog` 表示“系统中可被 AI 发现、引用、打开或加入任务的资源”。它不代表当前任务已经授权使用这些资源。

第一阶段只需要覆盖保存连接和已知运行态资源，不做全局实时订阅服务。

建议结构：

```rust
pub struct ResourceCatalog {
    pub items: Vec<CatalogResource>,
}

pub struct CatalogResource {
    pub id: ResourceId,
    pub kind: ResourceKind,
    pub label: String,
    pub aliases: Vec<String>,
    pub scopes: Vec<ResourceScope>,
    pub capabilities: Vec<ResourceCapability>,
    pub origin: ResourceOrigin,
    pub status: ResourceStatus,
}

pub enum ResourceOrigin {
    SavedConnection,
    ActiveSession,
    GeneratedConnection,
}

pub enum ResourceStatus {
    SavedNotOpen,
    Active,
    Unavailable,
    NeedsAuth,
}
```

第一阶段可以先复用现有 `ResourceRef` 作为 catalog item 的内部载体，但对外命名必须表达 catalog 语义，避免继续把它误当执行池。

### AgentResourceScope

`AgentResourceScope` 表示“当前对话/任务已经选入、允许执行工具的资源范围”。

建议结构：

```rust
pub struct AgentResourceScope {
    pub selected: Vec<ResourceRef>,
    pub default_target: Option<DefaultTarget>,
}

pub struct DefaultTarget {
    pub resource_id: ResourceId,
    pub reason: DefaultTargetReason,
}

pub enum DefaultTargetReason {
    CurrentTerminal,
    CurrentDatabase,
    CurrentConnection,
    UserSelected,
    MentionedFirst,
    RestoredSession,
}
```

第一阶段可以保留 `ResourceContext` 作为 runtime 兼容层，但新增构造/转换方法必须按 `Scope` 语义命名。后续再把字段替换为 `AgentResourceScope`。

### Default Target Rules

默认目标必须显式，不再依赖资源插入顺序。

- 终端侧边栏：scope 初始只包含当前终端/SSH 连接，default reason 为 `CurrentTerminal` 或 `CurrentConnection`。
- 数据库侧边栏：scope 初始只包含当前数据库连接，default reason 为 `CurrentDatabase`。
- Redis/MongoDB 侧边栏：scope 初始只包含当前连接，default reason 为 `CurrentConnection`。
- 全局 AI 工作台：catalog 包含所有连接；scope 初始为空，或只恢复用户明确选择过的 session scope；不要因为 catalog 有资源就自动设置第一个资源为 default。
- 用户 `@` 一个资源：该资源加入 scope；如果 scope 没有 default，则设为 `MentionedFirst`。
- 用户在 UI 中显式选择目标：设置 default reason 为 `UserSelected`。

### Tool Target Resolution

工具执行前的解析规则固定为：

1. 工具参数显式 `target`：从 scope 和 catalog 中解析；如果命中 catalog 但不在 scope，可加入 scope 或要求确认，取决于工具风险等级。
2. invocation 显式 `resource_id`：直接使用。
3. scope 有 `default_target`：使用 default。
4. 否则返回明确错误，要求模型或用户指定 target。

无显式 target 时，resolver 不允许从 catalog 猜测资源。

### Prompt Exposure

模型应该看到两类信息：

- 当前 scope：本任务已选入、可直接操作的资源和默认目标。
- 可用 catalog 摘要：告诉模型可以通过 `@`、连接工具或 list 工具发现/加入更多资源。

prompt 不应把 catalog 表述成“当前可以随便操作的资源池”。

### UI Behavior

`@` 补全读取 catalog，不读取 scope。

资源池/上下文栏显示 scope，不显示全部 catalog。另提供“添加资源”入口从 catalog 加入 scope。

工作台启动时：

- `@` 能看到所有保存连接。
- 会话侧边栏保持工作台模式。
- 当前任务 scope 可以为空。
- 如果用户直接要求执行需要 target 的工具，模型应提示选择目标或调用列表工具展示候选。

侧边栏启动时：

- `@` 能看到所有保存连接。
- scope 初始只有当前连接。
- default target 明确为当前连接。

## Migration Plan

### Phase 1: Compatibility Wrapper

新增 catalog/scope 命名层，内部可暂时复用 `ResourceRef` 和 `ResourceContext`。

- 新增 `ResourceCatalog` 或 `AvailableResourceCatalog`。
- 新增 `AgentResourceScope` 或 `ScopedResourceContext`。
- 保留 `ResourceContext` 到 tool/runtime 的转换。
- 修改构造函数命名，避免 `new_with_context_and_catalog` 被误用于 workbench/sidebar。

### Phase 2: Entry Semantics

统一入口构造规则：

- 工作台：`catalog = all connections`，`scope = empty/restored`。
- 终端侧边栏：`catalog = all connections`，`scope = current terminal`。
- DB/Redis/MongoDB 侧边栏：`catalog = all connections`，`scope = current connection`。

同时补测试覆盖：

- 工作台 `@` 有全量连接，但无显式 target 时不自动选第一个连接执行。
- 侧边栏默认 target 是当前连接。
- `@` 一个资源后，它进入 scope 并可作为工具 target 解析。

### Phase 3: Resolver Rules

把工具 target 解析集中到一个 resolver：

- 明确区分 scope 命中、catalog 命中、未命中、能力不匹配、歧义匹配。
- 无 target 时只允许使用 scope default。
- 对风险较高的工具，如果 target 只来自 catalog 而未进入 scope，需要确认或先加入 scope。

### Phase 4: Naming Cleanup

逐步替换容易误导的字段：

- `available_resources` -> `catalog`
- `ResourceContext.resources` -> `scope.selected`
- `ResourceContext.current` -> `scope.default_target`

这一阶段只做命名和类型收敛，不改变用户可见行为。

## Non-Goals

第一阶段不做：

- 全局实时 `ResourceManager` 服务。
- 连接状态订阅和自动刷新。
- 跨设备/云同步资源 catalog。
- 大规模重写 public MCP 工具体系。
- 移除现有 `ResourceContext` 和 `tool_runtime::ResourcePool`。

## Risks

- 如果一次性替换所有类型，改动面会过大。必须先做兼容包装。
- 如果工作台 scope 初始为空，部分现有工具测试可能假设有 default target，需要逐个改成显式 target 或期望错误。
- 如果 catalog 命中自动加入 scope，需按工具风险等级处理，避免高风险工具绕过用户确认。
- 如果 prompt 同时展示 catalog 和 scope，需要措辞清楚，避免模型把 catalog 当成已授权执行池。

## Acceptance Criteria

- 工作台输入框 `@` 能列出所有保存连接。
- 工作台保留会话侧边栏和 full workbench mode。
- 工作台无明确 target 时，不因 catalog 中存在资源而默认执行到第一个连接。
- 终端/数据库/Redis/MongoDB 侧边栏仍默认当前连接为 target。
- `@` 提及资源后，该资源进入当前任务 scope，并能被工具 target 解析。
- 统一测试覆盖 catalog、scope、default target、tool target resolver 四条链路。

