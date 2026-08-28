# 基于 DBX 的 Navop SQL 编辑器完整实现规格

> 文档状态：Implementation Specification
>
> 目标读者：负责 Navop SQL 编辑器、SQL 分析、元数据、执行链路和 GPUI Input 基础设施的开发 Agent
>
> 基准源码：本工作区 `dbx` 与 `navop` 当前源码
>
> 编写日期：2026-08-27
>
> 目标：以 DBX 的真实源码行为为基线，在 Navop 中实现一套统一、可扩展、异步安全的 SQL 编辑器能力

---

## 1. 文档目的

本文不是 UI 效果猜测，也不是只描述“行号左侧放一个运行按钮”的局部需求。本文依据 DBX 当前源码，对 SQL 编辑器的完整能力、内部数据流、边界行为、异步约束和测试基线进行拆解，并给出 Navop（Rust + GPUI）的迁移设计。

其他 Agent 应能依据本文按阶段完成以下能力：

1. 在每条完整、可执行 SQL statement 的起始行左侧显示运行图标；
2. 点击图标精确执行该 statement，不改变当前光标或选区；
3. hover 表、视图、函数等 SQL 对象时展示详细元数据和 DDL；
4. 提供表、字段、别名、CTE、函数、关键字、snippet 等上下文补全；
5. 提供当前 statement 边框、执行状态 marker、错误映射和结果联动；
6. 提供语义诊断、格式化、参数替换、变量展开、`IN (...)` 粘贴、INSERT 值提示等编辑辅助；
7. 正确处理不同数据库方言、quoted identifier、大 schema 和 stale async response；
8. 建立统一的 statement、document revision、metadata scope 和 offset contract，避免各模块独立解析 SQL。

### 1.1 非目标

本文不要求：

- 将 DBX 的 Vue/CodeMirror 代码逐行翻译为 Rust；
- 在一个 PR 中一次性完成所有能力；
- 将展示型 fallback DDL 当作可直接执行的权威 DDL；
- 用 `StreamingSqlParser` 直接承担编辑器增量分析；
- 为实现 SQL 专用功能而破坏 GPUI Input 的通用性。

### 1.2 事实与建议的标记

本文使用以下措辞区分来源：

- **DBX 事实**：DBX 当前源码已经存在的行为；
- **Navop 现状**：Navop 当前源码已经具备的能力；
- **Navop 方案**：建议新增或改造的实现；
- **建议扩展**：不一定与 DBX 完全等价，但为了 Navop 架构完整性建议实现。

---

## 2. 总体结论

DBX SQL 编辑器不是一个单独的“编辑框组件”，而是下列子系统的组合：

```text
长期存活的编辑器状态
        │
        ├── SQL statement range / executable range
        ├── dialect-aware lexical + semantic analysis
        ├── local metadata cache + remote metadata assistant
        ├── completion / signature / snippets
        ├── hover / navigation / context actions
        ├── diagnostics / execution error mapping
        ├── gutter / current-statement frame / decorations
        ├── execution target resolution / parameter preprocessing
        ├── result run / source range / execution state
        └── formatter / editing assists / persistence
```

Navop 实现时必须坚持三个核心原则。

### 2.1 唯一 statement range 来源

以下所有能力必须共用同一份 `SqlStatementRangeService` 或 `SqlAnalysisSnapshot.statements`：

- gutter 图标位置；
- gutter 点击执行范围；
- Run Current；
- current statement frame；
- execution picker；
- diagnostics statement window；
- execution marker；
- database error line/column 回映；
- result statement source range。

禁止 gutter、编辑器命令、driver execution parser、diagnostics 各自采用不同的 SQL 分割算法。

### 2.2 所有异步结果必须绑定 revision 和 scope

每一个异步请求至少绑定：

- document revision；
- connection id；
- catalog/database/schema；
- database type/dialect；
- metadata generation；
- request id 或 cancellation token。

请求返回后，如果其中任何一项已变化，结果必须丢弃，且不得写入当前 cache。

### 2.3 SQL 能力与 GPUI Input 基础能力分层

建议分为三层：

```text
crates/ui
  通用 gutter marker、range decoration、hover/completion host、hitbox、绘制

crates/db
  SQL tokenizer、statement ranges、identifier、semantic model、diagnostics contracts

crates/db_view
  connection scope、metadata provider、execution、result integration、SQL editor facade
```

GPUI Input 不应知道“table”或“SQL statement”，但应知道：

- 某逻辑行有一个可点击 marker；
- 某 byte/rope range 有 decoration；
- 某 marker 有图标、状态、tooltip 和 action；
- 某异步结果属于哪个 document revision。

---

## 3. DBX 能力总览

| 能力 | DBX 当前行为 | Navop 目标 |
|---|---|---|
| Statement 分割 | 方言感知，支持 `;`、`GO`、`DELIMITER`、Oracle `/`、routine block 等 | 建立编辑器专用 range engine |
| 左侧运行图标 | 每条 executable statement 起始行显示 | 增加通用 gutter marker lane |
| 精确执行 | gutter 永远执行对应 statement | 统一 `ExactRange` execution target |
| 当前 statement | 光标与前后空白、分号、注释有明确归属规则 | 与 gutter 使用相同 snapshot |
| 当前 statement frame | 无非空选区时绘制 statement 边框 | 增加 range decoration/layer |
| 表/字段补全 | semantic + metadata + keyword + snippet 合并 | 扩展当前 provider 为多 source |
| CTE/子查询补全 | 可使用局部推导出的输出列 | 扩展 symbol/semantic model |
| 函数补全与签名 | routine、signature、parameter help | 元数据增加 overload/signature |
| Hover | 表结构、注释、列、PK、索引、DDL | SQL hover resolver + detail popover |
| Navigation | table/view/materialized view/routine 等 | Cmd/Ctrl-click 与 context action |
| Diagnostics | unknown table/column、parser error、execution error | 视口级异步诊断 |
| INSERT 值提示 | `VALUES` 中显示对应列名 | inline widget/decorations |
| `SELECT *` 展开 | 结合 source 与 metadata 安全展开 | completion/intention action |
| 参数 | 多种 placeholder、类型输入、preview | execution preprocessing pipeline |
| SQL 变量 | `@set` 声明和引用展开 | 参数处理前执行 |
| Format/Compress | 方言格式化与压缩 | selection/full document action |
| Snippet | 内置、自定义、方言改写 | completion source |
| `IN` 粘贴 | 列表转 SQL literals | editor action |
| 状态持久化 | selection、viewport、tab 生命周期 | tab state snapshot |
| 结果联动 | statement 状态、summary、source focus | execution fingerprint + source map |

---

## 4. 统一数据契约

本节是 Phase 0 必须先确定的 contract。后续实现不得绕过这些结构自行传裸字符串和裸 offset。

### 4.1 Offset 约定

内部 SQL 分析统一使用 **UTF-8 byte offset**：

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SqlTextRange {
    /// Inclusive UTF-8 byte offset.
    pub start_byte: usize,
    /// Exclusive UTF-8 byte offset.
    pub end_byte: usize,
}
```

约束：

- `start_byte <= end_byte <= text.len()`；
- 两端都必须位于 UTF-8 char boundary；
- `end_byte` exclusive；
- UI Rope offset、GPUI text offset、LSP UTF-16 position 只在边界层转换；
- 禁止在同一结构中混用 byte、char 和 UTF-16 offset；
- debug build 中应对 range 做 boundary assertion。

建议新增显式转换 API：

```rust
fn sql_byte_to_rope_offset(text: &Rope, byte: usize) -> Result<usize>;
fn rope_offset_to_sql_byte(text: &Rope, offset: usize) -> Result<usize>;
fn sql_byte_to_line_column(text: &str, byte: usize) -> SqlLineColumn;
fn lsp_position_to_sql_byte(text: &str, position: LspPosition) -> Result<usize>;
```

### 4.2 Document snapshot

```rust
#[derive(Clone)]
pub struct SqlDocumentSnapshot {
    pub revision: u64,
    pub text: Arc<str>,
    pub database_type: DatabaseType,
    pub scope: SqlMetadataScope,
}
```

Revision 规则：

- 每次文档内容变化递增；
- 不因 selection、viewport 或主题变化递增；
- connection/database/schema/dialect 变化即使文本未变，也必须使 analysis/completion 请求失效；
- revision 不要求跨应用启动持久化，只需当前 editor instance 内单调递增。

### 4.3 Connection/metadata scope

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SqlMetadataScope {
    pub connection_id: ConnectionId,
    pub catalog: Option<Arc<str>>,
    pub database: Option<Arc<str>>,
    pub schema: Option<Arc<str>>,
    pub database_type: DatabaseType,
    pub generation: u64,
}
```

`generation` 在以下情况递增：

- connection 切换；
- database/catalog/schema 切换；
- session context 变化；
- DDL 执行后 metadata invalidation；
- 用户主动刷新 metadata；
- driver completion context version 变化。

### 4.4 Qualified identifier

```rust
#[derive(Clone, Debug)]
pub struct QualifiedIdentifier {
    pub parts: Vec<IdentifierPart>,
    pub range: SqlTextRange,
}

#[derive(Clone, Debug)]
pub struct IdentifierPart {
    /// SQL 中的原始文本，不含或包含 quote 的策略必须全局一致。
    pub raw: Arc<str>,
    /// 按当前 dialect 规则归一化后的查找值。
    pub normalized: Arc<str>,
    pub quoted: bool,
    pub quote_style: Option<SqlIdentifierQuoteStyle>,
    pub range: SqlTextRange,
}
```

必须保留：

- `"Foo"`；
- `` `Foo` ``；
- `[Foo]`；
- 未加引号的 `foo`。

不能在 tokenizer 后无条件 lowercase，因为 quoted identifier 的大小写语义可能不同。

### 4.5 Analysis snapshot

```rust
#[derive(Clone)]
pub struct SqlAnalysisSnapshot {
    pub document_revision: u64,
    pub scope_generation: u64,
    pub statements: Arc<[SqlStatementRange]>,
    pub tokens: Arc<[SqlToken]>,
    pub semantic_model: Arc<SqlSemanticModel>,
    pub diagnostics: Arc<[SqlDiagnostic]>,
}
```

Snapshot 是只读、可共享、带版本的。UI 和后台任务不可原地修改旧 snapshot。

---

## 5. Statement Range Engine

### 5.1 DBX 事实

DBX 的 `SqlTextRange` 结构包含：

```ts
type SqlTextRange = {
  from: number
  to: number
  sql: string
}
```

其中：

- `to` 是 exclusive；
- 正常 range 不包含 statement 末尾分号；
- 正常 range 不包含语句间空白；
- 内部 `RawStatement` 额外包含 `hitFrom`；
- `hitFrom` 表示光标位于前置缩进/空白时，仍可归属该 statement 的起点；
- `from` 仍是实际 SQL 内容的第一个字符。

证据：

- `dbx/apps/desktop/src/lib/sql/sqlStatementRanges.ts:7-15`
- `dbx/apps/desktop/src/lib/sql/sqlStatementRanges.ts:42-51`
- `dbx/apps/desktop/src/lib/sql/sqlStatementRanges.ts:282-358`

### 5.2 Navop 目标模型

```rust
#[derive(Clone, Debug)]
pub struct SqlStatementRange {
    /// 光标归属范围的起点，可包含前置空白。
    pub hit_start_byte: usize,

    /// 实际 executable SQL，不含分隔符和 statement 间空白。
    pub sql_range: SqlTextRange,

    /// 可选分隔符范围，例如 `;`、GO 行、Oracle `/`。
    pub delimiter_range: Option<SqlTextRange>,

    /// gutter marker 所在 logical source row。
    pub executable_line: usize,

    pub kind: SqlStatementKind,
    pub batch_index: usize,
}

pub enum SqlStatementKind {
    Sql,
    Procedure,
    Function,
    Trigger,
    AnonymousBlock,
    MongoCommand,
    RedisCommand,
    ElasticsearchRequest,
    Directive,
}
```

### 5.3 必须支持的 lexical state

Scanner 至少需要维护：

```rust
enum SqlLexicalState {
    Normal,
    SingleQuotedString,
    DoubleQuotedIdentifierOrString,
    BacktickIdentifier,
    BracketIdentifier,
    LineCommentDash,
    LineCommentHash,
    BlockComment { depth: usize },
    DollarQuotedString { tag: Arc<str> },
}
```

注意：

- PostgreSQL 支持 dollar quote；
- 某些数据库支持嵌套块注释，能力应由 dialect profile 控制；
- MySQL `#` 可能是 comment，但 MyBatis `#{name}` 不是；
- MySQL 中 `--` 是否为注释需要遵守其空白规则；
- SQL Server 方括号 identifier 内的 `;`、`GO` 不得切分；
- string/comment 内的 delimiter 永远不能切 statement。

### 5.4 必须支持的 delimiter/batch

DBX 已处理：

- 顶层 `;`；
- MySQL `DELIMITER`；
- SQL Server `GO` / `GO n`；
- Oracle 行首单独 `/`；
- Oracle PL/SQL block；
- MySQL procedure/function/trigger routine block；
- Mongo、Redis、Elasticsearch 的特殊 command range。

主要证据：

- `sqlStatementRanges.ts:292-558`
- `sqlStatementRanges.ts:689-710`
- `sqlStatementRanges.ts:1510-1564`
- `sqlStatementRanges.ts:1778-1845`
- `sqlStatementRanges.ts:1969-2001`
- `sqlStatementRanges.ts:2075-2085`

### 5.5 Current statement 归属规则

`statement_at_cursor(offset)` 必须满足：

1. offset 先 clamp 到 `[0, text.len()]`；
2. cursor 位于 statement SQL 内容内部时返回该 statement；
3. cursor 位于 statement 前的缩进空白，可归属后一个 statement；
4. cursor 位于 statement 后的同一行空白或 delimiter gap，可归属前一 statement；
5. cursor 位于纯空白行或纯注释行时返回 `None`；
6. 前导普通注释不能被误认为 executable statement；
7. executable directive 可将运行图标定位到 directive 后真正 SQL 起始行；
8. 返回的 executable range 仍从实际 SQL 内容开始，不包含命中用空白。

DBX 证据：

- `sqlStatementRanges.ts:605-640`
- `executableStatementRangeCache.ts:89-130`
- `statementDelimiter.ts:8-70`

### 5.6 Cache

建议：

```rust
pub struct SqlStatementRangeCache {
    revision: u64,
    database_type: DatabaseType,
    parameter_syntax_key: u64,
    ranges: Arc<[SqlStatementRange]>,
    by_sql_start: HashMap<usize, usize>,
    by_executable_line: HashMap<usize, usize>,
}
```

仅在以下变化时重建：

- document revision；
- database type/dialect；
- parameter syntax/profile；
- parser behavior version。

查询必须是 O(log n) 或 O(1)：

- `statement_at_cursor(offset)`；
- `statement_starting_on_line(row)`；
- `statements_intersecting(range)`；
- `statement_by_sql_start(byte)`。

### 5.7 不应直接复用 `StreamingSqlParser`

Navop 当前 `crates/db/src/streaming_parser.rs` 面向大文件/大脚本流式执行，公开模型是 source、bytes read 和 progress，不是长期存活编辑器的 revision snapshot。

实现原则：

- 编辑器 range engine 放在新模块；
- 如两者要复用分隔逻辑，抽出无 UI、无 I/O 的纯 scanner；
- `StreamingSqlParser` 和 editor service 分别做 adapter；
- 不让 UI 依赖流式 parser 的读取状态；
- 当前 `streaming_parser.rs` 有既存未提交修改，本设计实现不得覆盖或回滚。

---

## 6. 左侧运行图标与通用 Gutter

### 6.1 DBX 用户行为

- 只有每条 executable statement 的起始行显示 play icon；
- 多行 statement 不在每一行重复显示；
- 前导空行和普通注释行不显示；
- MySQL routine 整体只显示一个运行入口；
- 只响应左键 `mousedown`；
- 查不到 statement range 时不处理；
- gutter 点击始终执行对应 statement，即使全局执行模式设置为 All；
- 点击 gutter 不主动 focus editor，不把 viewport 滚回旧 cursor；
- marker 可显示 running、succeeded、failed；
- 文档变化时旧 marker 清空；
- execution state effect 到达时重新生成 marker。

证据：

- `dbx/apps/desktop/src/components/editor/QueryEditor.vue:1675-1687`
- `dbx/apps/desktop/src/components/editor/QueryEditor.vue:4704-4749`

### 6.2 GPUI 通用 API

在 `crates/ui/src/input/state.rs` 新增通用模型：

```rust
#[derive(Clone)]
pub struct InputGutterMarker {
    pub id: SharedString,
    pub logical_row: usize,
    pub lane: InputGutterLane,
    pub icon: IconName,
    pub state: InputGutterMarkerState,
    pub tooltip: Option<SharedString>,
    pub enabled: bool,
    pub action: Option<InputGutterAction>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputGutterLane {
    PrimaryAction,
    Diagnostic,
    Breakpoint,
    Custom(u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputGutterMarkerState {
    Idle,
    Running,
    Success,
    Error,
    Disabled,
}
```

建议 `InputGutterAction` 不直接持有 SQL closure，而使用稳定 id 发通用 event：

```rust
pub enum InputEvent {
    // existing events...
    GutterMarkerMouseDown {
        marker_id: SharedString,
        logical_row: usize,
        button: MouseButton,
    },
}
```

### 6.3 Layout 与 hit-testing

修改 `crates/ui/src/input/element.rs`：

1. gutter 由固定 lane 组成：
   - marker lane；
   - fold lane；
   - line number lane；
2. gutter 总宽度取决于启用 lane，不应因某一行有无 marker 而抖动；
3. marker 使用 logical source row，而不是 soft-wrap visual row；
4. 一条 source line 被 soft wrap 后，只在第一个 visual fragment 显示；
5. 横向滚动只影响文本，不移动 gutter；
6. marker 每个实例有独立 hitbox；
7. marker hitbox 优先于 line-number/fold/text hitbox；
8. `mousedown` 命中后 stop propagation，避免改变 cursor/selection；
9. tooltip 使用通用 hover/popover 生命周期；
10. fold 后不可见行的 marker 不绘制。

### 6.4 SQL facade

`crates/db_view/src/sql_editor.rs` 增加：

```rust
impl SqlEditor {
    pub fn set_statement_gutter_markers(
        &mut self,
        markers: Vec<SqlStatementGutterMarker>,
        cx: &mut Context<Self>,
    );
}

pub struct SqlStatementGutterMarker {
    pub statement_start_byte: usize,
    pub logical_row: usize,
    pub state: SqlExecutionMarkerState,
}
```

Marker id 推荐编码为：

```text
sql-statement:{document_revision}:{start_byte}:{end_byte}
```

事件处理时不能只相信 marker 中的旧 SQL。必须：

1. 解析 marker id；
2. 确认当前 document revision 相同；
3. 从当前 snapshot 重新按 start/end 查 range；
4. 创建 `ExactRange` execution request。

---

## 7. 执行目标解析

### 7.1 DBX 优先级

DBX 的执行选择逻辑：

1. 有非空 selection：执行 selection；
2. 否则构造 current/all candidates；
3. shortcut 路径 bypass picker；
4. toolbar 点击在设置开启且存在多个候选时可显示 picker；
5. gutter 永远执行精确 statement range。

DBX 的 `ExecutionSnapshot` 包含：

- `fullSql`
- `selectedSql`
- `cursorPos`
- `selectionFrom`
- `selectionTo`
- 可选 `editorViewportRequestId`

证据：

- `QueryEditor.vue:822-885`
- `sqlExecutionTarget.ts:7-14`
- `sqlExecutionTarget.ts:45-78`

### 7.2 Navop 目标模型

```rust
pub enum SqlExecutionTarget {
    Selection(SqlTextRange),
    CurrentStatement,
    AllStatements,
    ExactRange(SqlTextRange),
}

pub struct SqlExecutionRequest {
    pub request_id: u64,
    pub document_revision: u64,
    pub full_sql: Arc<str>,
    pub target: SqlExecutionTarget,
    pub resolved_sql: Arc<str>,
    pub source_range: Option<SqlTextRange>,
    pub statement_identity: Option<SqlStatementIdentity>,
    pub scope: SqlMetadataScope,
    pub transaction_mode: TransactionMode,
    pub open_in_new_result_tab: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SqlStatementIdentity {
    pub document_revision: u64,
    pub start_byte: usize,
    pub end_byte: usize,
    pub sql_fingerprint: u64,
}
```

### 7.3 入口行为

| 入口 | Target | 是否使用 picker | 是否改 selection/focus |
|---|---|---:|---:|
| Gutter play | `ExactRange` | 否 | 否 |
| Run shortcut | Selection 或 Current | 否 | 否 |
| Run All shortcut | `AllStatements` | 否 | 否 |
| Toolbar Run | Selection/Current/All | 可配置 | 否 |
| Context menu Run Selection | `Selection` | 否 | 右键前同步 selection |
| Context menu Run Current | `CurrentStatement` | 否 | 右键前同步 pointer target |

### 7.4 执行前处理顺序

推荐统一 pipeline：

```text
resolve target from snapshot
  → extract source SQL
  → expand declared SQL variables
  → extract unresolved parameters
  → optional parameter dialog
  → substitute parameters
  → construct dialect-specific batch script
  → execute
  → bind result to statement identity
```

变量展开必须先于 placeholder 参数替换。未声明变量应保留给参数系统。

### 7.5 执行状态

```rust
pub enum SqlExecutionState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Skipped,
}
```

文档一旦编辑：

- 旧 marker 不能继续附着到新文本；
- 可以保留历史 result tab；
- 但 result tab 的 source highlight 必须标明它属于旧 revision；
- 若 fingerprint 无法匹配当前 statement，不得自动跳转到错误 range。

---

## 8. SQL 分析层

### 8.1 推荐模块

```text
crates/db/src/sql_editor/
├── document.rs
├── dialect.rs
├── identifier.rs
├── statement_ranges.rs
├── analysis.rs
├── semantic_model.rs
├── diagnostics.rs
├── completion.rs
└── metadata.rs
```

如需降低首期改动，至少先新增：

```text
statement_ranges.rs
identifier.rs
analysis.rs
metadata.rs
```

### 8.2 Tokenizer

Navop 当前 tokenizer 已支持：

- keyword；
- ident/quoted ident；
- string；
- number；
- line/block comment；
- dot/comma/semicolon/parenthesis/operator；
- byte spans；
- token at offset。

现有文件：

- `navop/crates/db/src/sql_editor/sql_tokenizer.rs`

需要扩展：

- database dialect profile；
- backtick 和 bracket identifier；
- PostgreSQL dollar quote；
- incomplete string/comment 的稳定 token；
- parameter token；
- executable directive；
- token 中保留 raw text 与 normalized identifier；
- error/recovery token；
- statement scanner 可复用的 lexical state。

### 8.3 Semantic model

最低目标：

```rust
pub struct SqlSemanticModel {
    pub statements: Vec<SqlSemanticStatement>,
    pub references: Vec<SqlReference>,
    pub scopes: Vec<SqlScope>,
}

pub struct SqlScope {
    pub parent: Option<SqlScopeId>,
    pub row_sources: Vec<SqlRowSource>,
    pub projection_columns: Vec<SqlDerivedColumn>,
    pub ctes: Vec<SqlCte>,
}

pub enum SqlRowSource {
    Table(SqlTableReference),
    Cte(SqlCteReference),
    DerivedTable(SqlDerivedTable),
    TableFunction(SqlRoutineReference),
}
```

必须解析或保守推导：

- `FROM table [AS] alias`；
- `JOIN table alias`；
- qualified table；
- CTE 名称和输出列；
- subquery/derived table alias 和 projection；
- `SELECT expr AS alias`；
- INSERT target；
- UPDATE target；
- function call；
- visible scope 与 correlated parent scope。

语义模型面对不完整 SQL 时应“尽可能返回已知信息”，而不是因为 parse error 整体失败。

### 8.4 Statement window

DBX 在有 live CodeMirror state 时优先使用增量 syntax tree；tree 尚未 parse 到 cursor、出现 recovery error 或 dollar quote 可能截断时，回退 bounded scanner。

Navop 不一定复制 CodeMirror tree，但应保留相同策略：

1. 优先使用当前 revision 的 analysis snapshot；
2. snapshot 未完成或 cursor 不在已分析范围时，使用轻量 bounded scanner；
3. bounded scanner 默认向两侧最多扩展 32 KiB；
4. 超长 statement 可按需要扩大窗口；
5. 不在 UI thread 强制等待完整解析。

---

## 9. 自动完成

### 9.1 DBX 分层

DBX completion 的概念流程：

```text
cursor lexer/parser
  → CompletionContext
  → local semantic candidates
  → metadata tree/cache candidates
  → remote completion assistant
  → merge/filter/rank/dedupe
  → editor CompletionResult
```

这意味着“字段完成提示”不是简单地把所有列名放进一个数组。

### 9.2 Completion context

目标 context：

```rust
pub enum SqlCompletionContextKind {
    Start,
    Keyword,
    Catalog,
    Schema,
    Table,
    Routine,
    Column,
    AliasColumn,
    InsertTarget,
    InsertColumn,
    UpdateTarget,
    SetClause,
    Condition,
    JoinSource,
    JoinCondition,
    OrderBy,
    GroupBy,
    FunctionArgument,
    ExecArgument,
}
```

Request：

```rust
pub struct SqlCompletionRequest {
    pub request_id: u64,
    pub document_revision: u64,
    pub cursor_byte: usize,
    pub trigger: SqlCompletionTrigger,
    pub scope: SqlMetadataScope,
    pub replacement_range: SqlTextRange,
    pub context: SqlCompletionContext,
    pub analysis: Arc<SqlAnalysisSnapshot>,
}
```

### 9.3 Candidate 数据

```rust
pub struct SqlCompletionCandidate {
    pub label: Arc<str>,
    pub insert_text: Arc<str>,
    pub replacement_range: SqlTextRange,
    pub kind: SqlCompletionItemKind,
    pub detail: Option<Arc<str>>,
    pub documentation: Option<Arc<str>>,
    pub filter_text: Option<Arc<str>>,
    pub sort_text: Option<Arc<str>>,
    pub source: SqlCompletionSourceKind,
    pub score: i32,
    pub deprecated: bool,
    pub snippet: bool,
    pub commit_characters: Arc<[char]>,
}

pub enum SqlCompletionItemKind {
    Catalog,
    Database,
    Schema,
    Table,
    View,
    MaterializedView,
    Column,
    Alias,
    Function,
    Procedure,
    Package,
    Trigger,
    Sequence,
    Keyword,
    Snippet,
    Value,
}
```

字段 candidate 至少保存：

- column name；
- physical table；
- source alias；
- source qualifier SQL；
- schema；
- data type；
- nullable；
- comment。

表 candidate 至少保存：

- catalog/database/schema；
- object type；
- detail/comment；
- apply name；
- boost。

Routine candidate 至少保存：

- parent schema/name；
- signature；
- comment；
- overload identity。

### 9.4 Completion source

```rust
pub trait SqlCompletionSource: Send + Sync {
    fn complete(
        &self,
        request: SqlCompletionRequest,
        cx: &mut AsyncApp,
    ) -> Task<Result<SqlCompletionSourceResult>>;
}

pub struct SqlCompletionSourceResult {
    pub request_id: u64,
    pub document_revision: u64,
    pub scope_generation: u64,
    pub candidates: Vec<SqlCompletionCandidate>,
    pub incomplete: bool,
}
```

建议 source：

- `DialectKeywordCompletionSource`
- `SemanticCompletionSource`
- `MetadataCompletionSource`
- `RoutineCompletionSource`
- `SnippetCompletionSource`
- `HistoryCompletionBoostSource`

由 `MergedSqlCompletionProvider` 合并。不要让 schema refresh 直接替换整个 provider。

### 9.5 表和 schema 补全

必须支持：

- `table`；
- `schema.table`；
- MySQL 风格 `database.table`；
- `database.schema.table`；
- quoted schema 后的 dot completion；
- catalog/database/schema 切换；
- table/view/materialized view 类型区分；
- INSERT/UPDATE target；
- JOIN source；
- 自动 alias，可配置；
- Oracle 等不支持 table alias `AS` 的方言差异。

生成 alias 时：

- 多词表名可使用首字母；
- alias 冲突时加编号；
- 不覆盖显式 alias；
- 设置关闭时不生成；
- quoted table 需要按方言生成合法 alias。

### 9.6 字段完成提示

字段 completion 的解析顺序：

1. 从当前 statement semantic scope 收集 row source；
2. 分辨 physical table、CTE、derived table 和 table function；
3. 对 `alias.` 只返回该 source 的 columns；
4. 对未限定字段，根据所有 visible row source 合并；
5. 同名列需要显示来源并降低歧义；
6. CTE/subquery 的投影列不需要远端 metadata；
7. physical table cache miss 时异步请求 columns；
8. remote 返回前可显示已有 local candidates；
9. remote 返回后仅在 request/revision/scope 仍有效时刷新 popup。

必须覆盖：

- `SELECT | FROM users u` → users fields；
- `SELECT u.| FROM users u` → only `u` fields；
- `JOIN orders o ON u.id = o.|` → orders fields；
- correlated subquery 可看到合法 parent scope；
- `UPDATE users SET |` → target table fields；
- `INSERT INTO users (|)` → target table fields；
- `ORDER BY |` 可包含 projection alias；
- projection alias 在 WHERE/GROUP BY/HAVING/ORDER BY 的可见性按 dialect 控制；
- 不完整 metadata 时不应错误展开 `*` 或报 unknown column。

### 9.7 `SELECT *` 展开

DBX 支持结合 semantic source 和 metadata 展开 wildcard。

Navop 行为：

- 单表：
  ```sql
  SELECT *
  FROM users;
  ```
  展开为列列表。

- alias：
  ```sql
  SELECT u.*
  FROM users u;
  ```
  展开时保留 `u.` qualifier。

- 多表：
  - 保持 FROM/JOIN source 顺序；
  - 同名列根据设置使用 qualifier；
  - metadata 不完整或 stale 时拒绝操作，不生成猜测结果；
  - 异步返回后确认旧 SQL 与当前 document revision/range 内容一致。

### 9.8 Function/routine 与 signature help

Completion：

- function name；
- signature；
- return type；
- schema/package；
- overload；
- snippet-style parameters；
- 已有 `(` 时不重复插入。

Signature help：

- cursor 位于 argument list 时显示；
- 标出 active parameter；
- overloaded routine 可切换；
- 文档编辑或 cursor 离开 call 时关闭；
- metadata scope 变化使旧 signature 失效。

### 9.9 Snippet

DBX 内置 prefix 包括：

- `sel`
- `ins`
- `upd`
- `cte`
- `join`
- `case`
- `ct`
- `ex`
- `nex`
- `at`
- `ci`

并有 Manticore 专用模板。

Navop 应支持：

- 内置 snippet；
- 用户自定义 snippet；
- tab stop；
- next/previous field；
- dialect-specific body；
- 只有用户仍使用内置默认 body 时才自动做 dialect 改写；
- 用户自定义 body 永不被 silently rewrite。

典型方言改写：

- `LIMIT`
- `TOP`
- `FIRST`
- `FETCH FIRST`
- `ROWS`
- `ROWNUM`
- ClickHouse `ALTER TABLE ... UPDATE`
- 不同 `ALTER TABLE ADD COLUMN` 形式。

### 9.10 排序、过滤和去重

DBX 排序综合：

1. context/variable/exact；
2. fuzzy score；
3. history boost；
4. type boost。

DBX type boost 示例：

- column: 180
- table: 160
- schema: 120
- function: 90
- keyword: 0

Navop 推荐评分：

```text
final_score =
    exact_prefix_score
  + context_score
  + semantic_visibility_score
  + item_kind_score
  + history_score
  + fuzzy_score
  - qualification_penalty
  - ambiguity_penalty
```

匹配至少支持：

- exact；
- case-insensitive prefix；
- identifier initials；
- substring；
- ordered fuzzy subsequence；
- 可选汉字拼音首字母。

去重 key 不能只用 label，应包含：

```text
kind + qualified identity + insert_text + signature
```

### 9.11 Completion stale 防护

必须有单调递增 `completion_epoch` 或 request id。

以下事件使旧请求失效：

- document changed；
- cursor/selection 变化导致 context 改变；
- popup abort；
- connection/catalog/database/schema 变化；
- dialect/profile 变化；
- metadata generation 变化；
- IME composition start；
- editor deactivation；
- completion setting/snippet setting 变化。

每个关键 `await` 后都要检查：

```rust
if !completion_guard.is_current(
    request_id,
    document_revision,
    scope_generation,
) {
    return Ok(None);
}
```

---

## 10. Metadata Model 与 Cache

### 10.1 Navop 当前问题

当前 `SqlSchema` 主要包含：

- tables；
- global columns；
- functions；
- columns_by_table。

缺少：

- catalog/database/schema；
- object kind；
- table/view distinction；
- PK/FK/index；
- nullable/default/comment；
- routine overload；
- qualification/quoted state；
- revision/version。

当前 schema 加载还存在逐表串行请求 columns 的 N+1 风险。

### 10.2 Metadata contract

```rust
pub trait SqlMetadataProvider: Send + Sync {
    fn search_objects(
        &self,
        request: SearchSqlObjectsRequest,
        cx: &mut AsyncApp,
    ) -> Task<Result<SearchSqlObjectsResponse>>;

    fn list_columns(
        &self,
        request: ListSqlColumnsRequest,
        cx: &mut AsyncApp,
    ) -> Task<Result<ListSqlColumnsResponse>>;

    fn get_object_detail(
        &self,
        request: GetSqlObjectDetailRequest,
        cx: &mut AsyncApp,
    ) -> Task<Result<SqlObjectDetail>>;

    fn get_ddl(
        &self,
        request: GetSqlObjectDdlRequest,
        cx: &mut AsyncApp,
    ) -> Task<Result<SqlObjectDdl>>;
}
```

### 10.3 Object model

```rust
pub enum SqlObjectKind {
    Catalog,
    Database,
    Schema,
    Table,
    View,
    MaterializedView,
    Function,
    Procedure,
    Package,
    Trigger,
    Sequence,
}

pub struct SqlObjectRef {
    pub kind: SqlObjectKind,
    pub catalog: Option<Arc<str>>,
    pub database: Option<Arc<str>>,
    pub schema: Option<Arc<str>>,
    pub name: Arc<str>,
    pub quoted_parts: Arc<[bool]>,
}

pub struct SqlColumnMetadata {
    pub name: Arc<str>,
    pub data_type: Arc<str>,
    pub nullable: Option<bool>,
    pub default_value: Option<Arc<str>>,
    pub comment: Option<Arc<str>>,
    pub ordinal: Option<u32>,
    pub primary_key: bool,
    pub generated: bool,
}

pub struct SqlObjectDetail {
    pub object: SqlObjectRef,
    pub comment: Option<Arc<str>>,
    pub columns: Arc<[SqlColumnMetadata]>,
    pub primary_key: Arc<[Arc<str>]>,
    pub indexes: Arc<[SqlIndexMetadata]>,
    pub foreign_keys: Arc<[SqlForeignKeyMetadata]>,
    pub signature: Option<Arc<str>>,
}
```

### 10.4 Search assistant contract

DBX remote assistant 请求包括：

- connection id；
- database；
- schema；
- object kinds；
- mask；
- case sensitivity；
- global search；
- max results；
- search comments/definitions；
- parent schema/name；
- match mode。

Response 包括：

- candidates；
- incomplete；
- fallback used。

Navop 应保留等价字段，以便：

- completion；
- object search；
- navigation；
- hover cache miss；
- large schema lazy lookup。

### 10.5 Cache key

至少包含：

```rust
pub struct SqlMetadataCacheKey {
    pub connection_id: ConnectionId,
    pub catalog: Option<Arc<str>>,
    pub database: Option<Arc<str>>,
    pub schema: Option<Arc<str>>,
    pub object: Option<Arc<str>>,
    pub object_kind: Option<SqlObjectKind>,
    pub quoted_parts: Arc<[bool]>,
    pub database_type: DatabaseType,
    pub session_context: Option<Arc<str>>,
}
```

禁止仅按裸 `table_name` 缓存 columns。

### 10.6 Cache 写入规则

请求发起时捕获：

- connection generation；
- database generation；
- scope generation。

返回后：

```rust
if current_scope.generation != requested_generation {
    // Discard. Do not write into cache.
    return;
}
```

建议分层 cache：

- object search：短 TTL；
- columns：中 TTL；
- object detail：中 TTL；
- DDL：中/长 TTL，但 DDL 后立即 invalidation；
- negative result：很短 TTL；
- session-scoped metadata：绑定 session id/version。

### 10.7 大 schema 策略

禁止连接后串行加载全部表的全部字段。

推荐：

1. 初始只加载 catalog/database/schema/table/view 基础索引；
2. 当前 statement 引用的表优先加载 columns；
3. `alias.` 或 INSERT target 触发按表加载；
4. hover 触发 detail/DDL lazy load；
5. 使用 bounded concurrency；
6. 支持分页和 `incomplete`；
7. cache entry 记录 memory cost；
8. LRU 驱逐；
9. completion popup 本地响应预算内先返回，再增量补远端。

---

## 11. Hover、对象详情与导航

### 11.1 Identifier 定位

DBX 行为：

- cursor 可位于 token 内，也可刚好位于 token 后；
- 未加引号的 SQL keyword 不作为对象；
- 支持一至三/四段 qualified identifier；
- database/schema/table 含义按 dialect 解释；
- metadata 查找可 case-insensitive，但 quoted state 必须保留；
- local lookup 必须校验 connection/database/schema/object。

主要证据：

- `queryCursorTableTarget.ts:32-108`
- `queryCursorTableTarget.ts:155-181`

### 11.2 Hover resolve pipeline

```text
pointer position
  → GPUI offset
  → SQL byte offset
  → token/qualified identifier
  → reject string/comment/keyword
  → semantic reference resolution
  → local metadata cache
  → optional remote detail/DDL
  → revision + scope validation
  → render popover
```

失败行为：

- 找不到对象：静默不显示；
- metadata 请求失败：hover 内可显示轻量错误或静默关闭；
- 不弹全局 error toast；
- 旧请求返回：丢弃；
- pointer 已离开目标：不显示。

### 11.3 支持对象类型

至少：

- table；
- view；
- materialized view；
- procedure；
- function；
- package；
- trigger。

建议扩展：

- sequence；
- schema；
- database；
- column。

### 11.4 Table hover 内容

详情应包括：

- object type；
- catalog/database/schema；
- comment；
- columns；
- type length/precision/scale；
- nullable；
- default；
- generated/extra；
- column comment；
- primary key；
- indexes；
- foreign keys；
- backend DDL；
- companion `COMMENT ON` / `CREATE INDEX`。

### 11.5 DDL 权威性

规则：

1. driver/backend `get_ddl` 是权威来源；
2. sanitize 只能清理展示噪声；
3. parse/reformat 不安全时展示 sanitized raw DDL；
4. 不得因为 formatter 失败而丢掉结构；
5. metadata 构造的 `CREATE TABLE` 只作 fallback；
6. fallback 必须标注“根据元数据生成的预览，不保证可执行”；
7. fallback 不得进入“复制并执行”默认路径。

DBX 证据：

- `hoverTableSql.ts:5-117`
- `hoverTableSql.ts:290-354`
- `hoverTableSql.ts:753-825`

### 11.6 Hover layout

参考 DBX：

- max width 900px；
- width 80vw；
- max height 约半个 viewport；
- content max height 480px；
- vertical scroll；
- 超宽 SQL 支持 horizontal scroll；
- wheel 和 Shift+wheel；
- resize 时重新布局；
- destroy 时清理 controller/listener。

GPUI 版本不必逐像素一致，但必须：

- 不遮满整个编辑器；
- DDL 可横向滚动；
- 长 column list 可纵向滚动；
- popover 可复制；
- pointer 离开、editor destroy、tab deactivation 时清理异步任务与 listener。

### 11.7 Navigation

交互：

- Cmd/Ctrl + click；
- context menu；
- hover 中 action；
- completion resolve action。

目标：

- 打开对象结构；
- 打开数据；
- 查看 DDL/source；
- 跳到已打开对象 tab。

Table actions：

- View Data
- Edit Structure
- View DDL

View/materialized view actions：

- View Data
- Edit View
- View Source
- View DDL

右键菜单必须使用本次 pointer 坐标同步出的 target，不能复用旧 hover/cursor target。

---

## 12. Diagnostics

### 12.1 DBX 当前范围

DBX 核心 diagnostics 包括：

- unknown table；
- unknown column；
- parser error；
- database execution error 的 line/column 映射。

它不是完整 SQL lint 规则集。

### 12.2 Contract

```rust
pub struct SqlDiagnostic {
    pub id: u64,
    pub document_revision: u64,
    pub range: SqlTextRange,
    pub severity: SqlDiagnosticSeverity,
    pub code: Option<Arc<str>>,
    pub message: Arc<str>,
    pub source: SqlDiagnosticSource,
    pub related: Arc<[SqlDiagnosticRelatedInfo]>,
}

pub enum SqlDiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}

pub enum SqlDiagnosticSource {
    Parser,
    Semantic,
    Metadata,
    Execution,
}
```

### 12.3 诊断逻辑

Parser：

- unclosed string/comment；
- invalid token；
- parser recovery error；
- line/column/span。

Semantic：

- unknown table；
- unknown column；
- unknown alias；
- ambiguous column 可作为 warning；
- metadata 不完整时避免误报；
- 多表同名字段不能直接报 unknown；
- CTE、subquery、correlated scope 正确处理。

Execution：

- 只有 result 对应的 SQL fingerprint/revision 与当前文档相符时显示；
- driver line/column 转为 source range；
- execution target 是 selection/exact range 时加上 source base offset；
- 无法可靠映射时只在 result panel 显示，不在错误位置画波浪线。

### 12.4 Viewport 分析

DBX 按可见 viewport 相交的完整 statements 分析。

Navop 策略：

1. 从 `visible_row_range()` 转为 byte range；
2. 用 statement snapshot 扩展到完整 statement；
3. 同一 statement 只分析一次；
4. cursor 所在超长 statement 即使只可见中段也必须完整分析或采用 safe window；
5. 视口外旧 diagnostics 可暂时保留；
6. 更新时只替换受影响 statement ranges；
7. Oracle PL/SQL、procedure/function batch 可先跳过 semantic unknown-column 规则；
8. Mongo/Elasticsearch 等非 SQL language mode 跳过 SQL diagnostics。

### 12.5 Debounce 与 stale

使用独立 `diagnostic_run_id`，不得复用 completion request id。

- 文档变化：递增、清当前 stale decoration、重新调度；
- 默认 debounce 500ms；
- completion active 或用户处于 table-name 输入状态时可延长；
- 每个 metadata await 后检查 run id/revision/generation；
- tab deactivate 取消 timer 并使 run id 失效；
- activate 后重新调度可见范围。

---

## 13. 当前 Statement Frame 与 Decorations

DBX 在以下条件显示当前 statement 边框：

- setting 开启；
- selection 为空；
- cursor 可解析到 executable statement。

规则：

- executable range 本身可以不含 `;`；
- frame 可把紧邻 trailing `;` 纳入；
- 注释、空行或非 delimiter 字符会阻断 frame 延伸；
- selection 非空时 frame 隐藏，避免和 selection 视觉冲突。

Navop 可在 Input 增加：

```rust
pub struct InputRangeDecoration {
    pub id: SharedString,
    pub range: Range<usize>,
    pub kind: InputRangeDecorationKind,
    pub style: InputRangeDecorationStyle,
}
```

建议绘制顺序：

```text
line background
  → current statement frame
  → selection/search match
  → syntax text
  → semantic underline
  → diagnostics squiggle
  → hover highlight
  → caret
  → inline widget
```

---

## 14. INSERT 值列名提示

### 14.1 用户行为

在：

```sql
INSERT INTO users (name, age)
VALUES ('Alice', 18),
       ('Bob', 20);
```

每个 value expression 前显示不可编辑的灰色提示：

```text
name: 'Alice', age: 18
```

### 14.2 解析

需要识别：

- target table；
- database/schema/table；
- explicit column list；
- 每一行 `VALUES (...)`；
- 每行顶层 expression 起点；
- `INSERT ... SELECT` projection；
- 无 explicit columns 时从 metadata 按 ordinal 获取表字段。

### 14.3 性能

参考 DBX：

- 只分析 viewport 与主 cursor 附近；
- 单侧默认最多扩展 32 KiB；
- doc/viewport 变化后约 80ms debounce；
- 重解析前 widget range 随 edit mapping；
- 同一 offset 去重；
- metadata cache miss 时先触发异步加载，不显示错误列名。

### 14.4 GPUI 表达

可以复用现有 inline completion 的布局基础，但必须区分：

- inline completion 是可接受的 ghost text；
- INSERT value hint 是不可接受、不可编辑的 decoration widget。

建议新增 `InputInlineWidget` 通用模型，或在 range decoration 中增加 `before_text`，但不要伪装为真实文档文本。

---

## 15. 编辑辅助

### 15.1 单引号 caret

行为：

- 光标在自动生成的 `''` 中按 `'`：跳过右侧 closing quote；
- 已有 opening quote 且缺少 closing quote：插入 SQL escaped quote；
- 普通位置交给默认输入；
- read-only、多 selection、非空 selection 时不拦截；
- auto-close 关闭时不拦截。

### 15.2 `IN (...)` 列表粘贴

输入来源：

- 当前 selection；
- clipboard。

支持：

- comma；
- tab；
- newline；
- 已有 `IN (...)`；
- 简单 slash 列表；
- `NULL`；
- 单引号 escape。

保护：

- 最大 1 MiB；
- 最多 10,000 values；
- 单个普通文本拒绝；
- 日期、URL、绝对路径不能误判为 slash list；
- cursor 已在 `IN` / `NOT IN` 后时只插括号内容。

### 15.3 Selection case conversion

提供：

- Uppercase；
- Lowercase。

要求：

- 只处理非空 selection；
- 保持转换后的 range 选中；
- string literal 保持原文；
- PostgreSQL dollar quote 保持；
- dialect-specific quoted string/identifier/executable comment 保护；
- 不做简单的整个字符串 `to_uppercase()`。

### 15.4 SQL 参数

支持 descriptor：

```rust
pub enum SqlParameterSyntax {
    Positional,
    Named,
    Shell,
    MyBatis,
    SqlServer,
}

pub enum SqlParameterValueKind {
    String,
    Number,
    Boolean,
    Null,
    Raw,
}
```

执行前：

- extract descriptors；
- 去重但保留 occurrence；
- 显示参数 dialog；
- 类型和值输入；
- 替换后 SQL preview；
- execute。

Literal：

- null → `NULL`；
- raw → 原样；
- number → 不加 quote；
- string → dialect-aware quote/escape；
- placeholder 在 string 内时使用 string fragment escape，不重复包裹。

MyBatis foreach：

- item/index；
- open/separator/close；
- collection 展开；
- enabled syntaxes 限制识别范围。

### 15.5 `@set` SQL 变量

支持：

```sql
@set ids = (1, 2, 3);
SELECT * FROM users WHERE id IN ${ids};
```

规则：

- 声明 key case-insensitive；
- value 保留原始 SQL；
- 不在 string/comment/quoted identifier 内替换；
- `@@version` 等原生变量不误替换；
- declaration context 可来自 full document；
- 只删除本次 execution target 内的 declaration；
- 未声明 placeholder 留给参数系统；
- 变量展开先于参数 dialog。

### 15.6 Comment、line edit 与 folding

必须保留或新增：

- toggle line comment；
- toggle block comment；
- duplicate line；
- delete line；
- move line；
- indent/outdent；
- delete blank lines；
- fold/unfold；
- nested `BEGIN...END`；
- CASE；
- CTE/subquery；
- UNION；
- string/comment 中的关键字不参与 folding。

### 15.7 Drag/drop table 与 column

建议支持：

- schema tree 中 table 拖入 editor；
- column 拖入 editor；
- payload 带 source connection/database/schema/database type；
- context 不完整时拒绝；
- 按源 dialect quote identifier；
- 不依赖系统 `dataTransfer` 再反查已失效对象；
- multi-line selection/drop 位置正确。

---

## 16. 格式化与压缩

### 16.1 能力

- format selection；
- format full document；
- compress selection/document；
- SQL；
- Mongo；
- Elasticsearch；
- JSON；
- XML。

### 16.2 安全行为

- 输入上限建议 1,000,000 chars；
- XML-looking 内容不送 SQL formatter；
- editing surface 遇到 parse error 返回原文并提示；
- display surface formatter 失败返回 sanitized raw text；
- async format 返回时确认 document revision 和原 range 内容未变化；
- formatter 不得覆盖用户在等待期间的新编辑。

### 16.3 配置

- keyword case；
- type case；
- function case；
- identifier case；
- indentation；
- tab width；
- logical operator newline；
- FROM layout；
- expression width；
- query spacing；
- dense operators；
- semicolon；
- parameter style；
- JSON import/export。

### 16.4 Compress

必须保护：

- string 内空白；
- quoted identifier；
- MySQL executable/versioned/optimizer comments；
- PostgreSQL dollar quote；
- nested block comments；
- SQL Server bracket identifier；
- 未闭合 block comment 原文。

---

## 17. 快捷键、Toolbar 与 Context Menu

### 17.1 快捷键行为

至少：

- execute；
- execute in new result tab；
- execute all；
- save；
- format；
- compress；
- `SELECT *` expansion；
- find/replace；
- indent/outdent；
- duplicate/delete/move line；
- undo/redo；
- uppercase/lowercase；
- line/block comment；
- fold；
- `IN` paste；
- completion；
- signature help；
- next/previous snippet field。

DBX 的关键约束：

- Enter 不是执行，而是插入保持缩进的新行；
- Enter 先关闭 completion，并短暂抑制自动 completion；
- Tab 只有在 selection 为空时才允许 completion/snippet 接管；
- shortcut execution bypass picker；
- Vim 模式时 Vim keymap 优先。

### 17.2 Toolbar

DBX toolbar 能力包括：

- execute/cancel；
- Explain/cancel Explain；
- Explain Analyze/Autotrace；
- Format；
- Compress；
- keyword case；
- semantic diagnostics toggle；
- dangerous command toggle；
- save/open；
- import result archive；
- `IN` paste；
- multi-database execute；
- auto/manual transaction；
- commit/rollback。

Navop 可按现有产品范围分期，但 action contract 应预留：

```rust
pub enum SqlEditorAction {
    Execute,
    ExecuteAll,
    ExecuteNewResult,
    Cancel,
    Explain,
    ExplainAnalyze,
    Format,
    Compress,
    ExpandWildcard,
    PasteAsInList,
    ToggleDiagnostics,
    Commit,
    Rollback,
}
```

### 17.3 Context menu 同步顺序

严格顺序：

1. pointer coordinate → text offset；
2. 根据 pointer 同步 selection；
3. 同步 executable statement range；
4. 同步 object target；
5. 关闭旧 hover；
6. 构建并打开 menu。

不能先打开菜单，再异步更新 target。

菜单建议：

- execute selection/current；
- execute in new result tab；
- object actions；
- expand `SELECT *`；
- comments；
- format/compress；
- copy/cut/paste；
- uppercase/lowercase；
- paste as SQL IN；
- find/replace；
- delete blank lines；
- select all。

---

## 18. 结果面板与编辑器联动

### 18.1 DBX 能力

- editor/result split pane；
- multiple result runs；
- result/summary/chart/messages；
- running/error/cancel/evicted/empty states；
- statement summary；
- statement status；
- 点击 statement 预览；
- 双击 statement 聚焦 source；
- execution end 后可滚动到 cursor statement；
- statement marker 与 result fingerprint 对齐。

### 18.2 Navop source map

```rust
pub struct SqlExecutionResultSource {
    pub request_id: u64,
    pub document_revision: u64,
    pub source_range: Option<SqlTextRange>,
    pub sql_fingerprint: u64,
    pub statement_index: Option<usize>,
}
```

行为：

- result row/run 保留 source identity；
- 单击 result statement：编辑器 preview range；
- 双击：如果 current document revision 相同，scroll and select/focus；
- revision 不同但 fingerprint 唯一匹配时可提示用户跳转；
- 无法匹配时只展示 SQL snapshot，不在当前文档错误定位；
- 新 execution 不继承旧 execution 的 focused result index；
- message result 与 data result 的默认 focus 规则明确。

### 18.3 可编辑结果集

DBX 还有 SQL 分析支撑结果表 inline edit：

- 识别单表可编辑 SELECT；
- projection 到 source column 映射；
- primary key identity；
- join/quoted alias；
- set operation、DISTINCT、aggregation、computed column 等保守拒绝；
- 映射不唯一时只读，不猜测。

这部分可作为后续 phase，但 semantic model 设计不应阻断它。

---

## 19. 编辑器生命周期与状态

### 19.1 DBX 原则

DBX 使用：

> 单一长期存活的 `EditorView` + Compartment 局部重配置

而不是 settings/props 每次变化时重建编辑器。

Navop 对应原则：

- `InputState`/`SqlEditor` entity 长期存活；
- theme、wrap、read-only、completion、diagnostics 开关通过状态更新；
- 不因 connection/schema 刷新而重建文本 entity；
- 保留 undo history、selection、viewport、composition。

### 19.2 Update flow

文档变化：

- increment revision；
- invalidate completion；
- invalidate diagnostics；
- clear current execution marker projection；
- schedule statement/semantic analysis；
- emit model change；
- persist selection/viewport（debounced）。

Selection/cursor 变化：

- update current statement frame；
- update signature help；
- update context action target；
- 不递增 document revision。

Viewport 变化：

- schedule viewport diagnostics；
- schedule INSERT hints；
- update visible semantic highlights；
- 不重算全文 metadata。

### 19.3 IME

Composition start：

- invalidate completion；
- 暂停会破坏 composition 的自动 edit；
- 不高频回写外部 model。

Composition end：

- flush final text；
- increment revision once or按实际 transaction；
- schedule diagnostics；
- schedule completion；
- 不接受 composition 前发出的 async result。

### 19.4 Deactivate/activate

Deactivate：

- flush selection/viewport；
- cancel hover pending；
- invalidate completion；
- cancel diagnostics timer；
- stop background UI refresh；
- unregister transient pointer/drop listeners；
- 不销毁 text state。

Activate：

- restore listeners；
- restore viewport/selection；
- schedule visible diagnostics；
- refresh metadata-dependent decorations；
- 按产品策略 restore focus。

Destroy：

- cancel timers/tasks/animation frames；
- remove window/pointer/scroll listeners；
- destroy popovers；
- release metadata subscriptions；
- clear marker hitboxes；
- 不让 callback 捕获已销毁 entity。

---

## 20. Navop 当前能力与差距

### 20.1 已有能力

`crates/db_view/src/sql_editor.rs`：

- SQL code editor；
- completion provider；
- hover provider 注入；
- actions；
- line number；
- soft wrap；
- schema/table/column/function 基础 completion；
- tokenizer + context inferrer + symbol table。

`crates/db_view/src/sql_editor_view.rs`：

- connection/database/schema；
- current/all/selected/cursor statement execution；
- transaction/session；
- result container；
- toolbar/editor/result 布局。

`crates/ui/src/input/state.rs` 与 `element.rs`：

- line number；
- line decoration background；
- diagnostics style；
- hover provider；
- completion menu；
- inline completion；
- visible row range；
- mouse point → offset；
- fold gutter；
- line number hitbox；
- range layout/paint。

### 20.2 主要差距

| 层 | 缺口 |
|---|---|
| Input | 通用 marker lane、marker icon/state/tooltip/action、独立 hitbox |
| Input | editor document revision 的公开 contract |
| Input | SQL diagnostics 可消费的公共 range model |
| Input | generic inline widget/decorations |
| SQL tokenizer | 完整方言 lexical state、dollar quote、delimiter/routine |
| Statement | editor-safe unified statement snapshot |
| Semantic | CTE、derived table、projection、reference graph |
| Completion | 多 source merge、stale request contract、qualified replacement |
| Metadata | 作用域、object kind、detail、DDL、revision cache |
| Hover | qualified symbol resolver 和 object detail renderer |
| Diagnostics | parser/semantic/metadata/execution 统一模型 |
| Execution | `ExecutionRequest`、source map、exact range |
| Result | revision/fingerprint 绑定 |

### 20.3 Provider overwrite 风险

当前 `set_db_completion_info` 会重建 provider。如果直接新增 DBX 风格 provider，schema refresh 可能把它覆盖。

必须改为：

- 一个长期存活的 `MergedSqlCompletionProvider`；
- schema/DB completion info 只更新 metadata source 的数据；
- 外部 provider 作为独立 source；
- 不通过 setter 替换整个 provider object。

---

## 21. 文件级实施落点

### 21.1 `crates/db`

#### 新增

```text
crates/db/src/sql_editor/document.rs
crates/db/src/sql_editor/dialect.rs
crates/db/src/sql_editor/identifier.rs
crates/db/src/sql_editor/statement_ranges.rs
crates/db/src/sql_editor/analysis.rs
crates/db/src/sql_editor/semantic_model.rs
crates/db/src/sql_editor/diagnostics.rs
crates/db/src/sql_editor/metadata.rs
```

#### 修改

```text
crates/db/src/sql_editor/sql_tokenizer.rs
crates/db/src/sql_editor/sql_symbol_table.rs
crates/db/src/sql_editor/sql_context_inferrer.rs
crates/db/src/sql_editor/mod.rs
```

职责：

- 纯 SQL 数据结构和算法；
- 不依赖 GPUI；
- byte offset；
- deterministic unit tests；
- incomplete SQL 保守恢复。

### 21.2 `crates/ui`

#### 修改

```text
crates/ui/src/input/state.rs
crates/ui/src/input/element.rs
crates/ui/src/input/lsp/hover.rs
crates/ui/src/input/lsp/completions.rs
crates/ui/src/input/popovers/completion_menu.rs
```

职责：

- 通用 gutter/decorations；
- hitbox；
- marker event；
- async provider host；
- tooltip/completion rendering；
- 不出现 SQL-specific object types。

可按现有目录风格新增：

```text
crates/ui/src/input/gutter.rs
crates/ui/src/input/decorations.rs
crates/ui/src/input/inline_widget.rs
```

### 21.3 `crates/db_view`

#### 修改

```text
crates/db_view/src/sql_editor.rs
crates/db_view/src/sql_editor_view.rs
```

职责：

- SQL facade；
- metadata provider；
- analysis scheduling；
- connection scope；
- execution request；
- result integration；
- context menu；
- toolbar/actions。

如文件继续膨胀，建议拆分：

```text
crates/db_view/src/sql_editor/
├── completion_provider.rs
├── hover_provider.rs
├── metadata_cache.rs
├── execution.rs
├── diagnostics.rs
└── actions.rs
```

### 21.4 不应修改的边界

- 不要把 SQL parser 逻辑写进 `render_sql_editor`；
- 不要把 SQL-specific marker 写死在 `TextElement`；
- 不要让 `SqlEditor` 自己串行拉全库所有 columns；
- 不要让 completion 直接持有 stale connection entity；
- 不要在未协调时覆盖 `crates/db/src/streaming_parser.rs` 的既有修改。

---

## 22. 分阶段实施计划

### Phase 0：Contract 与测试基线

交付：

- `SqlTextRange` byte offset contract；
- `SqlDocumentSnapshot` revision；
- `SqlMetadataScope` generation；
- `QualifiedIdentifier`；
- `SqlExecutionRequest`；
- `SqlCompletionRequest`；
- 测试 fixture 与 DBX parity case list。

验收：

- 所有模块只使用显式 range type；
- 不再用不注明单位的 `(usize, usize)` 跨层传 SQL range；
- 文档 revision 单调递增；
- scope generation 可观测。

### Phase 1：Statement Range Engine

交付：

- quote/comment/dollar quote；
- `;`；
- `GO`；
- `DELIMITER`；
- Oracle `/`；
- routine/block；
- current statement lookup；
- executable line；
- cache。

验收：

- gutter/current/frame 对同一 cursor 返回同一 statement；
- 不依赖 UI；
- 完成 DBX statement range 测试移植。

### Phase 2：通用 GPUI Gutter

交付：

- marker model；
- lane layout；
- independent hitbox；
- icon/state/tooltip；
- mouse event；
- soft-wrap/fold/scroll 行为。

验收：

- 多行 statement 只显示一次；
- 点击不改变 selection/focus；
- 文档变更清 marker；
- UI snapshot/manual test 覆盖。

### Phase 3：精确执行

交付：

- `ExactRange`；
- selection/current/all priority；
- gutter execution；
- statement identity；
- running/success/error/cancel marker；
- result source fingerprint。

验收：

- gutter 永远执行点击的 statement；
- 当前 cursor 在另一 statement 不影响；
- 编辑文档后旧 marker 不落到新 statement；
- SQL Server batch 正确。

### Phase 4：Metadata Model 与 Cache

交付：

- object/column/detail/DDL model；
- scope-aware key；
- generation guard；
- lazy column fetch；
- bounded concurrency；
- cache invalidation。

验收：

- 同名跨 schema table 不串数据；
- connection 切换后旧响应不写 cache；
- 大 schema 不执行逐表串行列加载。

### Phase 5：Completion

交付：

- merged provider；
- table/schema/database；
- fields；
- alias-qualified fields；
- CTE/derived table；
- routine/signature；
- snippets；
- keyword；
- wildcard expansion；
- rank/dedupe；
- stale cancellation。

验收：

- 文档测试矩阵中 completion 核心 case 通过；
- `alias.` 首屏响应不被全库扫描阻塞；
- schema refresh 不覆盖 provider。

### Phase 6：Hover 与 Navigation

交付：

- qualified identifier；
- semantic target；
- object detail；
- DDL preview；
- Cmd/Ctrl-click；
- context actions。

验收：

- quoted/multi-part identifier 正确；
- cache scope 正确；
- old hover response 不显示；
- DDL fallback 明确标注非权威。

### Phase 7：Diagnostics

交付：

- parser error；
- unknown table/column；
- execution error mapping；
- viewport scheduling；
- run id；
- squiggle/hover detail。

验收：

- 多表歧义不误报；
- metadata incomplete 不误报；
- stale result 不绘制；
- long statement 中段可见仍能诊断。

### Phase 8：Editing Utilities

交付：

- format/compress；
- comment/case；
- wildcard expansion action；
- INSERT value hints；
- `IN` paste；
- quote caret；
- parameters；
- variables；
- snippets/signature。

### Phase 9：Persistence 与 Result Integration

交付：

- selection anchor/head；
- viewport；
- current statement frame；
- result runs；
- statement summary；
- execution source navigation；
- tab activate/deactivate lifecycle。

---

## 23. 验收测试矩阵

### 23.1 Statement ranges

必须覆盖：

- 多条顶层 statement；
- 末尾无分号；
- string 中分号；
- escaped quote；
- double quote/backtick/bracket identifier；
- line/block comment 中分号；
- PostgreSQL dollar quote；
- MyBatis placeholder 与 hash-comment；
- MySQL `DELIMITER`；
- procedure/function/trigger；
- nested `CASE/BEGIN`；
- SQL Server `GO`，并忽略 string/comment 中的 `GO`；
- Oracle/Xugu/GaussDB/SAP HANA block；
- cursor 在 statement 内容；
- 前置缩进；
- 同行分号后空白；
- trailing comment；
- 下一 statement 起点；
- 单独下一行分号；
- 空行/纯注释返回 None；
- Elasticsearch REST request。

DBX 基准：

- `apps/desktop/src/lib/__tests__/sql/sqlStatementRanges.spec.ts`

### 23.2 Gutter

必须覆盖：

- 多行 statement 精确映射；
- routine 只有一个 marker；
- placeholder-only line；
- directive 后 executable line；
- SQL Server temp table；
- 普通前导块注释不附着；
- continuation line 不显示 marker；
- delimiter gap；
- trailing whitespace；
- doc revision/dialect 变化重建 cache；
- left mouse only；
- click 不移动 cursor；
- running/success/error state。

DBX 基准：

- `apps/desktop/src/lib/__tests__/sql/executableStatementRangeCache.spec.ts`

注意：DBX 缺少完整 gutter UI render test，Navop 应新增 GPUI element/hitbox 测试。

### 23.3 Completion

必须覆盖：

- keyword typing trigger；
- `ORDER BY DESC` 不误判为 table context；
- quoted mixed-case schema/sequence；
- escaped apostrophe；
- dialect-isolated function；
- existing `(` 不重复；
- `*` 单表/多表/alias/重复字段/FROM 顺序；
- stale/incomplete metadata 拒绝 wildcard；
- qualified schema/database；
- alias-qualified column；
- JOIN；
- CTE/subquery；
- WHERE/INSERT/UPDATE；
- alias generation；
- Oracle no `AS`；
- explicit completion 在受限模式仍可用；
- snippet；
- routine parameter/signature；
- large catalog performance。

DBX 基准：

- `apps/desktop/src/lib/__tests__/sql/sqlCompletion.context.spec.ts`
- `apps/desktop/src/lib/__tests__/sql/sqlCompletion.signature.spec.ts`
- `apps/desktop/src/lib/__tests__/sql/sqlCompletion.snippet.spec.ts`
- `packages/app-tests/sqlCompletion.test.ts`
- `packages/app-tests/sqlCompletionRoutineParameters.test.ts`
- `packages/app-tests/sqlCompletionPerformance.test.ts`

### 23.4 Hover/navigation

必须覆盖：

- no/single/composite PK；
- empty columns；
- SQL Server `varchar(max)` / `datetime2(7)`；
- precision/scale；
- indexes/comments/constraints；
- partition/distribution；
- sanitize MySQL/MariaDB charset/collation；
- PostgreSQL access-control tail；
- preserve companion statements；
- raw DDL；
- defaults containing comma；
- unsupported type fallback；
- catalog/database/schema cache scope；
- cross-database bare-name rejection；
- statement-local navigation target；
- stale hover response。

DBX 基准：

- `apps/desktop/src/lib/editor/__tests__/hoverTableSql.spec.ts`
- `apps/desktop/src/lib/__tests__/sql/sqlNavigation.spec.ts`

### 23.5 Diagnostics

必须覆盖：

- missing table/column；
- empty/incomplete metadata；
- alias；
- correlated subquery；
- nested scope；
- multi-table ambiguity；
- parser line/column；
- severity/message/span；
- viewport complete statement；
- long statement middle viewport；
- SQL Server `GO` batch；
- procedure/function/Oracle PL/SQL skip；
- Mongo/Elasticsearch skip；
- stale document；
- execution range base offset。

DBX 基准：

- `packages/app-tests/sqlSemanticDiagnostics.test.ts`

### 23.6 Formatting

必须覆盖：

- whitespace compress；
- comment handling；
- preserve strings/identifiers；
- empty input；
- already compressed；
- executable comma placement；
- unclosed block comment；
- MySQL executable/versioned/optimizer comments；
- backslash escape；
- `#` comment；
- MySQL `--` rule；
- PostgreSQL dollar quote/nested comment/E-string；
- SQL Server bracket identifier。

DBX 基准：

- `packages/app-tests/sqlFormatter.test.ts`

### 23.7 Editing

必须覆盖：

- quote caret；
- large paste；
- text edits；
- newline indentation；
- line/block comments；
- selection case；
- `IN` paste；
- INSERT value hints；
- trimmed selection；
- folding；
- context-menu target ordering；
- table/column drag-drop；
- parameters；
- variables；
- editable query conservative rejection。

DBX 基准：

- `apps/desktop/src/lib/__tests__/sql/sqlQuoteCaret.spec.ts`
- `apps/desktop/src/lib/__tests__/sql/sqlSelectionCase.spec.ts`
- `apps/desktop/src/lib/__tests__/sql/sqlInListPaste.spec.ts`
- `apps/desktop/src/lib/__tests__/editor/*`
- `apps/desktop/src/components/editor/__tests__/QueryEditorContextMenu.spec.ts`
- `packages/app-tests/sqlParameters.spec.ts`
- `packages/app-tests/sqlVariables.spec.ts`
- `packages/app-tests/sqlAnalysis.test.ts`
- `packages/app-tests/queryEditorTableDrop.test.ts`

### 23.8 Execution/result

必须覆盖：

- selection/current/all/exact range；
- Mongo current command；
- SQL Server `GO` batch；
- trailing whitespace；
- current frame 与 gutter range 一致；
- stale marker；
- cancel；
- error source mapping；
- new result tab；
- result default focus；
- result source double-click。

DBX 基准：

- `packages/app-tests/sqlExecutionTarget.test.ts`
- `packages/app-tests/sqlBatchScript.test.ts`
- `apps/desktop/src/lib/__tests__/sql/currentStatementFrame.spec.ts`
- `packages/app-tests/sqlserverResultFocus6189.test.ts`

---

## 24. 性能目标

以下为 Navop 建议目标，不是 DBX 源码中明确 SLA。

### 24.1 编辑输入

- 普通 keypress 的同步 SQL 工作：P95 < 4ms；
- UI thread 不进行远端 metadata I/O；
- 不在每次 keypress 全文重建所有 columns；
- statement range scanner 对普通文档 O(n)，cache 后 cursor lookup O(log n)；
- 100k 字符文档滚动无明显卡顿；
- 1M 字符文档仍可编辑，重分析按 viewport/statement。

### 24.2 Completion

- local semantic/keyword candidates：P95 < 30ms；
- cache-hit table/column completion：P95 < 50ms；
- remote metadata 不阻塞首批 local popup；
- popup 最大 candidates 有上限；
- large schema 搜索分页；
- 每个 editor 同类 remote request 可合并/取消。

### 24.3 Hover

- cache hit：目标 50ms 内；
- cache miss：显示 loading skeleton 或延迟出现；
- pointer 移动后旧请求立即逻辑失效；
- DDL formatting 在 background task。

### 24.4 Diagnostics

- 默认 debounce 500ms；
- viewport analysis 优先；
- metadata enrichment bounded concurrency；
- 不因 diagnostics 阻塞输入或 completion。

---

## 25. 高风险与强制规则

### 25.1 Provider overwrite

**风险**：`set_db_completion_info` 重建 provider，导致新 provider 被 schema refresh 覆盖。

**强制规则**：provider 长期存活，更新 source 数据，不替换 provider。

### 25.2 Stale async

**风险**：旧 completion/hover/metadata/diagnostics 返回后污染新文档。

**强制规则**：所有 async response 检查 request id + revision + scope generation。

### 25.3 Statement splitter divergence

**风险**：gutter 显示 A，Run Current 执行 B，driver 再切成 C。

**强制规则**：编辑器所有 target 使用同一 snapshot；driver 接收已解析 execution request，除协议必要外不二次猜 cursor statement。

### 25.4 Offset mismatch

**风险**：Rust byte、Rope char、UTF-16 混用，在中文/emoji 前后错位。

**强制规则**：内部 byte offset，边界显式转换，测试包含中文和 emoji。

### 25.5 Multi-database semantics

必须显式处理：

- Oracle schema；
- MySQL `database.table`；
- PostgreSQL `database.schema.table`；
- `uses_schema_as_database`；
- IPC schema switch；
- session-scoped current database/schema。

### 25.6 Quoted identifier

不能无条件 lowercase：

- `"Foo"`；
- `` `Foo` ``；
- `[Foo]`。

### 25.7 Large schema

禁止：

- 连接时逐表串行加载 columns；
- completion 每次遍历无限 catalog；
- cache key 只有 table name；
- popup 等待全量 remote 才显示。

### 25.8 Hover DDL

禁止：

- metadata fallback 冒充 backend DDL；
- formatter parse error 后返回空；
- sanitize 改变 statement 语义；
- 在 hover 失败时弹打断用户的全局 toast。

### 25.9 Right-click stale target

菜单 target 必须在打开前同步，异步菜单 action 绑定本次 context-menu identity。

### 25.10 IME

composition 期间不得触发会替换 composition range 的 completion/edit action。

### 25.11 Result marker

旧 result 只能绑定旧 revision/fingerprint，不能按相同行号直接挂到新文档。

### 25.12 Resource leak

hover、popover、timer、pointer/scroll/window listener、background task 在 deactivate/destroy 时必须清理。

---

## 26. Definition of Done

只有满足以下全部条件，才可认为“DBX 风格 SQL 编辑器核心改造”完成。

### 26.1 Statement 与执行

- [ ] 每条 executable statement 起始行显示运行图标；
- [ ] 多行 SQL 只显示一次；
- [ ] comment/string/dialect delimiter 不误切；
- [ ] gutter 精确执行且不移动 cursor；
- [ ] current/all/selection/exact range 规则一致；
- [ ] current statement frame 与 gutter range 一致；
- [ ] execution marker 有 running/success/error/cancel；
- [ ] 编辑后旧 marker 不错位。

### 26.2 Completion

- [ ] table/schema/database；
- [ ] field completion；
- [ ] alias-qualified fields；
- [ ] JOIN/INSERT/UPDATE context；
- [ ] CTE/derived table columns；
- [ ] projection alias dialect rules；
- [ ] routines/signatures；
- [ ] keyword/snippet；
- [ ] `SELECT *` safe expansion；
- [ ] rank/filter/dedupe；
- [ ] stale cancellation；
- [ ] large schema 不阻塞。

### 26.3 Hover/navigation

- [ ] qualified/quoted identifier；
- [ ] table/view/materialized view/routine；
- [ ] columns/type/nullability/default/comment；
- [ ] PK/index/FK；
- [ ] backend DDL；
- [ ] marked fallback DDL；
- [ ] scope-safe cache；
- [ ] Cmd/Ctrl-click 和 context actions；
- [ ] stale hover 不显示。

### 26.4 Diagnostics/editing

- [ ] parser error；
- [ ] unknown table/column；
- [ ] execution error mapping；
- [ ] viewport/stale guard；
- [ ] formatter/compress；
- [ ] quote caret；
- [ ] `IN` paste；
- [ ] case/comment actions；
- [ ] parameters/variables；
- [ ] INSERT value hints。

### 26.5 Lifecycle/quality

- [ ] document revision contract；
- [ ] metadata generation contract；
- [ ] byte offset contract；
- [ ] deactivate/activate/destroy cleanup；
- [ ] DBX 边界测试已移植；
- [ ] GPUI gutter hitbox/render test；
- [ ] 中文/emoji offset test；
- [ ] large catalog performance test；
- [ ] 没有覆盖用户现有 `streaming_parser.rs` 修改。

---

## 27. DBX 源码索引

### 27.1 Editor 主组件与布局

- `dbx/apps/desktop/src/components/editor/QueryEditor.vue`
- `dbx/apps/desktop/src/components/layout/EditorToolbar.vue`
- `dbx/apps/desktop/src/components/layout/ContentArea.vue`

### 27.2 Statement 与 execution

- `dbx/apps/desktop/src/lib/sql/sqlStatementRanges.ts`
- `dbx/apps/desktop/src/lib/sql/executableStatementRangeCache.ts`
- `dbx/apps/desktop/src/lib/sql/statementDelimiter.ts`
- `dbx/apps/desktop/src/lib/sql/sqlExecutionTarget.ts`
- `dbx/apps/desktop/src/lib/sql/currentStatementFrame.ts`
- `dbx/apps/desktop/src/lib/editor/codemirrorStatementGutter.ts`
- `dbx/apps/desktop/src/lib/editor/codemirrorCurrentStatementFrameLayer.ts`

### 27.3 Completion 与 semantic

- `dbx/apps/desktop/src/lib/sql/sqlCompletion.ts`
- `dbx/apps/desktop/src/lib/sql/sqlCompletionLookupTarget.ts`
- `dbx/apps/desktop/src/lib/sql/sqlSnippetTemplates.ts`
- `dbx/apps/desktop/src/lib/sql/sqlSyntaxTreeWindow.ts`
- `dbx/apps/desktop/src/lib/sql/semantic/completion.ts`
- `dbx/apps/desktop/src/lib/sql/semantic/model.ts`
- `dbx/apps/desktop/src/lib/sql/semantic/references.ts`
- `dbx/apps/desktop/src/lib/metadata/completionTreeIndex.ts`
- `dbx/apps/desktop/src/stores/connectionStore.ts`
- `dbx/crates/dbx-core/src/types.rs`
- `dbx/crates/dbx-core/src/schema.rs`

### 27.4 Hover/navigation

- `dbx/apps/desktop/src/lib/sql/queryCursorTableTarget.ts`
- `dbx/apps/desktop/src/lib/sql/sqlNavigation.ts`
- `dbx/apps/desktop/src/lib/editor/hoverTableSql.ts`
- `dbx/apps/desktop/src/lib/editor/sqlHoverLayout.ts`
- `dbx/apps/desktop/src/lib/editor/sqlSignatureTooltip.ts`

### 27.5 Diagnostics

- `dbx/apps/desktop/src/lib/sql/semantic/diagnostics.ts`
- `dbx/apps/desktop/src/lib/sql/sqlDiagnostics.ts`

### 27.6 编辑辅助

- `dbx/apps/desktop/src/lib/sql/insertValueHints.ts`
- `dbx/apps/desktop/src/lib/editor/codemirrorInsertValueHints.ts`
- `dbx/apps/desktop/src/lib/sql/sqlQuoteCaret.ts`
- `dbx/apps/desktop/src/lib/sql/sqlInListPaste.ts`
- `dbx/apps/desktop/src/lib/sql/sqlSelectionCase.ts`
- `dbx/apps/desktop/src/lib/sql/sqlParameters.ts`
- `dbx/apps/desktop/src/lib/sql/sqlVariables.ts`
- `dbx/apps/desktop/src/components/editor/SqlParameterDialog.vue`
- `dbx/apps/desktop/src/lib/sql/sqlAnalysis.ts`

### 27.7 Format/history/save

- `dbx/apps/desktop/src/lib/sql/sqlFormatter.ts`
- `dbx/apps/desktop/src/lib/sql/sqlFormatterConfig.ts`
- `dbx/apps/desktop/src/lib/sql/autoFormat.ts`
- `dbx/apps/desktop/src/stores/historyStore.ts`
- `dbx/apps/desktop/src/stores/savedSqlStore.ts`
- `dbx/apps/desktop/src/lib/sql/sqlFileOpen.ts`
- `dbx/apps/desktop/src/composables/useExternalSqlFileChanges.ts`

---

## 28. Navop 源码索引

- `navop/crates/db/src/sql_editor/sql_tokenizer.rs`
- `navop/crates/db/src/sql_editor/sql_symbol_table.rs`
- `navop/crates/db/src/sql_editor/sql_context_inferrer.rs`
- `navop/crates/db/src/streaming_parser.rs`
- `navop/crates/db_view/src/sql_editor.rs`
- `navop/crates/db_view/src/sql_editor_view.rs`
- `navop/crates/ui/src/input/state.rs`
- `navop/crates/ui/src/input/element.rs`
- `navop/crates/ui/src/input/lsp/hover.rs`
- `navop/crates/ui/src/input/lsp/completions.rs`
- `navop/crates/ui/src/input/popovers/completion_menu.rs`

---

## 29. 推荐给实施 Agent 的执行方式

1. 不要从 UI 图标开始孤立实现；先完成 Phase 0 contract；
2. Phase 1 先移植 statement range tests，再实现 scanner；
3. Phase 2 的 gutter 必须作为 GPUI Input 通用能力，不写死 SQL；
4. Phase 3 后即可交付用户最直观的“每条完整 SQL 左侧运行”能力；
5. metadata 和 completion 分开做，先改 provider 生命周期，避免后续被覆盖；
6. 每个 async 功能先写 stale-response test，再接远端；
7. hover 首先实现 cache-hit table detail，再逐步接 DDL 和 navigation；
8. diagnostics 必须在 statement/semantic snapshot 稳定后实现；
9. 每个 phase 都应保持 editor 可运行，不做大爆炸重写；
10. 所有涉及 `streaming_parser.rs` 的改动先确认并保留当前工作树中的既有修改。

最终架构的判断标准不是“看起来像 DBX”，而是：

- SQL statement 语义一致；
- 异步结果不会错位；
- qualified/quoted identifier 不丢信息；
- 大 schema 不阻塞；
- execution、completion、hover、diagnostics 共用统一 snapshot；
- GPUI Input 基础设施仍然是通用、可复用的。
