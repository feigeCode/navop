# 连接级 AI 访问策略（issue #437）

> 文档状态：Implemented Design
>
> 需求来源：[feigeCode/navop#437](https://github.com/feigeCode/navop/issues/437)
>
> 目标读者：维护 AI 工具面（public MCP / 内置 AI 对话）与连接存储的开发 Agent
>
> 编写日期：2026-10-21（2026-10-22 修订：策略改为随连接同步的字段）

---

## 1. 需求与风险场景

用户用 Navop 管理多台服务器，其中一部分只允许人工操作。风险不是「AI 看不到连接」，而是
**AI 在推理后自行换一条路去做写操作**：用户明确让 AI 用「只读数据库工具」排查问题，AI 若判断
某个已保存连接就是对应的测试库，仍可能通过 `connections.open_session` 或 `ssh.exec` 登到那台
服务器上执行写入/删除命令。

因此需求是：**按连接声明「AI 是否可操作」**，且声明必须在 AI 真正执行命令的那一跳生效，而不是
只影响它「看起来能看到什么」。

补充要求（实际使用场景）：策略属于**连接本身的属性**，必须随连接一起云同步。
「生产就是生产」——在任意一台机器、任意一个客户端上打开同一连接，都应当保持同样的限制，
不能因为换了机器就回到默认放行。

## 2. 结论：给 `StoredConnection` 加字段

评估过三种落点：

| 方案 | 结论 | 原因 |
| --- | --- | --- |
| **`StoredConnection::ai_access_level` 字段** | **采用** | 策略是连接属性，需要随云同步/导入导出在多端保持一致；字段天然进 `ConnectionPlainData` 载荷，带 `#[serde(default)]` 后对旧客户端/旧导出文件向后兼容（缺字段 = `Allowed`） |
| 复用 `params` / `extra_params` | 否决 | SSH/FTP 的 `params` 会经结构体往返丢未知键（`crates/core/src/storage/models.rs` 的 `params_for_storage`）；`extra_params` 仅数据库类型使用且无类型校验、无 UI 展示 |
| 独立表 `connection_ai_access` | 否决（曾实现后回退） | 与连接内容解耦的代价是**不参与云同步**：换设备/换客户端后策略丢失，正是用户要避免的「这台机器禁止、那台机器又能操作」；且删除连接要额外清理、导出也不带策略 |

对应的迁移：`crates/core/migrations/20261022000001_connection_ai_access.sql`
（`ALTER TABLE connections ADD COLUMN ai_access_level TEXT;`），注册在
`crates/core/src/storage/migration.rs` 的 `MIGRATIONS` 数组末尾。

代价与做法：加列需要同步 `ConnectionRow` / `from_row` / `From<ConnectionRow>`、9 处显式列清单
SELECT、4 组写入（云端 upsert 的 UPDATE 与 INSERT、`insert`、`update`），以及
`StoredConnection` 的 11 个 `new_*` 构造器（统一填 `Allowed`）。这些点全部在编译期或清单内，
落地时逐条补齐并跑通测试。

## 3. 数据层

- 列定义：`connections.ai_access_level TEXT NULL`；`NULL` 表示默认 `Allowed`，
  因此迁移不需要回填数据，旧库升级后行为不变。
- 级别类型：`tool_runtime::AiAccessLevel { Allowed, ReadOnly, Denied }`，序列化为
  `"allowed" / "read_only" / "denied"`；**未知字符串按 `Denied` 处理（fail-closed）**，
  避免将来新增级别时旧版本静默放行。
- 仓储门面：`ConnectionRepository::{ai_access_level, ai_access_levels, set_ai_access_level, ai_access_restricted}`。
  写 `Allowed` 等价于写 `NULL`（默认值不落库）。
- 云同步：`ConnectionPlainData` 增加 `ai_access_level`（`#[serde(default)]`），上行写入、
  下行应用到连接，并加入团队冲突判定的 `connection_content_changed` 白名单——否则只改策略
  不会触发重新上传，同步后会静默回退。
- 导入导出：`StoredConnection` 以 serde 序列化，字段随导出文件走；旧文件缺该字段时按
  `Allowed` 反序列化。

## 4. 三层拦截：可见性 → 路由 → 执行

单点拦截不够：AI 的工具调用有多条入口（外部 MCP 客户端、内置 AI 对话、原生 tool_runtime 工具），
且有「资源上下文为空时直接透传目标」的 fail-open 路径。所以策略分三层落地：

### 第 1 层：可见性（尽早不让模型看到）

- `crates/navop_app/src/public_mcp_runtime/resource_pool.rs`：`Denied` 连接不进资源池；`ReadOnly` 连接的
  能力按白名单裁剪（`Query/DatabaseQuery/List/ReadFile`）。终端会话资源按 `session.connection_id`
  回查策略：`Denied` 会话整条移出池，`ReadOnly` 削掉 `TerminalExec/TerminalInput`；活动 Redis
  会话同样按连接 id 收窄。
- `crates/ai_chat_view/src/resource_builder.rs`：内置 AI 的资源目录、`@` 提及、AgentResourceScope
  都按级别过滤，`Denied` 直接不产出，`ReadOnly` 只保留只读能力。
- `crates/navop_runtime/src/connections/management.rs`：`connections.list/find` 不返回 `Denied`
  连接；`connections.show` 命中时给出可操作的中文错误。

### 第 2 层：路由（目标解析失败即拒绝）

资源池剥离后，`ToolRuntimeMcpProvider` 的 `target` 归一化会以 `TargetNotInPool` /
`TargetLacksCapabilities` 直接失败，模型拿不到句柄，也不会触发任何远端命令。

### 第 3 层：执行（唯一的硬闸门）

`crates/navop_app/src/public_mcp_runtime/access_guard.rs` 实现 `tool_runtime::ToolAccessGuard`，挂在
`ToolRegistry::call` 上（`crates/tool_runtime/src/registry.rs`），因此对
**外部 MCP 客户端与内置 AI 对话同时生效**：

- 输入里的 `connection_id` / `connection` / `target` / `session_id` 都会解析成连接 id
  （数字按 id，字符串按连接名，会话 id 通过终端注册表快照映射到 `connection_id`）；
- `Denied` → 直接返回 `access_denied_error`（提示由用户手动操作，禁止绕过）；
- `ReadOnly` → 只有 `ToolAnnotations.read_only == true` 的工具放行，写类工具返回只读错误；
- 挂载点共 4 处：`public_mcp_runtime/tool_registry.rs` 合并后的 registry，以及
  `public_mcp_runtime.rs` 中内置 AI 单独注册的 database / redis / sftp 原生 registry
  （`ToolRegistry::merge` 不继承子 registry 的 guard，必须重新挂）。

## 5. 界面

- 通用组件：`crates/connection_form/src/ai_access.rs` 提供 `ai_access_selector`（三档选择器
  本体）、`ai_access_label` / `level_label` / `level_hint`（文案）与 `stored_level` / `write_level`
  （读、写连接字段）。表单只持有一个 `Entity<AiAccessLevel>`，渲染时交给组件，不再各写一遍
  单选钮布局——数据库 / SSH / Redis / MongoDB / middleware 五个表单现在共用同一份实现，
  新增连接类型时同样只需一行 `ai_access_selector`。
- 保存时随连接一起写库（`StoredConnection::ai_access_level`），失败显式报错。
- 侧栏行内徽标（只读 / 禁止）与右键菜单「AI 访问」子菜单，可直接切换三档；
  写失败会弹通知，不静默失败。菜单使用 `Connection.ai_access_*` 的短文案（与徽标同一命名空间），
  表单使用 `ConnectionForm.ai_access_*` 的完整文案。

## 6. 覆盖矩阵

| 工具面 | 是否受策略约束 | 依赖层 |
| --- | --- | --- |
| `terminal.*` / `ssh.exec` / `terminal.write_keys` | 是 | 1 + 3 |
| `database.*` / `redis.*` / `mongo.*` | 是 | 1 + 3 |
| `sftp.*` | 是 | 1 + 3 |
| `connections.list/find/show/open_session/test/delete/update` | 是 | 1 + 3（`update/delete` 等按 id 的工具靠第 3 层） |
| `workspaces.*` / `diagnostics.*` / `onetcli.app_info` | 不涉及连接，无需约束 | — |

## 7. 已知限制与后续工作

1. **旧版本客户端**：老客户端不认识 `ai_access_level`，上行时会把该列覆盖回默认值，因此
   多端混用时建议升级到同一版本；旧导出文件导入时按 `Allowed` 处理。
2. **团队共享连接**：策略是连接内容的一部分，会随连接共享给团队；若某位成员需要更严格的限制，
   应把该连接改为个人连接或复制一份，而不是依赖本机开关。
3. **审批流顺序**：`PermissionMode::Ask` 下会先弹审批、再由第 3 层拒绝；拒绝结果仍然是拒绝，
   只是多一次提示。若希望提前短路，可把 guard 前移到审批之前。
4. **新增工具面**：新加连接类工具时必须声明 `ToolTargetSpec` 能力，否则只会受第 3 层的
   「名称/id 解析」约束，缺少能力级（只读）判断。
