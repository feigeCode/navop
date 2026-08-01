# Issue #30 数据库体验优化设计

| 项目 | 内容 |
| --- | --- |
| 状态 | Partially Implemented（已实施 #1、#2、#3、#6、#8、#9） |
| 日期 | 2026-08-01 |
| 来源 | GitHub Issue #30「数据库建议」 |
| 已实施 | #1 临时查询快捷键、#2 右侧面板调整宽度、#3 提交前同步右侧编辑器、#6 当前表设计器快捷键、#8 文字选区对比度、#9 Shift 连续整行选择 |
| 后续优化 | #4 MySQL 断线恢复、#5 生产环境标识与水印、#7 表设计器批量列操作 |
| 适用范围 | `db_view`、`db`、`one_ui`、`one-core`、cloud sync、主程序设置页 |

> 本文记录 Issue #30 的整体设计、两批实施结果和后续拆分方向。当前已完成六项低风险或可复用现有状态/事件链的改进；标记为“后续优化”的 #4、#5、#7 仍是设计提案，不代表仓库已经具备对应能力。

## 1. 摘要

Issue #30 包含九项数据库操作体验建议，横跨数据表、右侧大文本编辑器、表设计器、连接恢复、连接元数据、云同步和通用表格选区。若一次性实现，会把低风险 UI 改动和高风险数据语义、连接生命周期改动混在同一个变更中，难以验证和回滚。

当前采用以下拆分：

1. 第一批完成保存同步、表设计器快捷键和文字选区对比度；
2. 第二批继续完成临时查询快捷键、预览面板拖动宽度和 Shift 连续整行选择；
3. 对涉及数据库重试语义、跨设备同步或跨数据库列定义转换的需求，只记录完整方案；
4. 后续按独立 PR 实施，每个 PR 具备自己的回归测试和验收范围。

## 2. Issue 需求映射

| 编号 | 需求摘要 | 当前状态 | 实施说明或后续原因 |
| --- | --- | --- | --- |
| 1 | 从数据表快捷打开临时查询并生成 `SELECT` | 第二批实施 | 复用现有查询 action、Tab 创建和限定表名 SQL 生成链路 |
| 2 | 右侧编辑框拖动改变宽度 | 第二批实施 | 增加 Tab 内宽度状态、拖动柄和 280–800px 尺寸约束 |
| 3 | 右侧编辑后可直接提交，无需先失焦 | 第一批实施 | 现有 `flush_pending` 已能同步值，只缺保存前调用 |
| 4 | MySQL 空闲断线后快速恢复 | 后续优化 | 涉及连接失效识别、重连和“哪些请求允许重试”的安全边界 |
| 5 | 生产环境标识和可选页面水印 | 后续优化 | 涉及连接持久化、云同步、表单、Tab 上下文和安全展示 |
| 6 | 从数据表快捷打开表设计器 | 第一批实施 | 现有事件链完整，只缺 action、默认绑定和设置项 |
| 7 | 表设计器按 ↓ 新增、批量复制粘贴列 | 后续优化 | 当前是单行选区，跨数据库粘贴还涉及能力降级 |
| 8 | 提升编辑框文字选区背景对比度 | 第一批实施 | 通过默认主题 selection token 完成 |
| 9 | 数据表按 Shift 连续选择 | 第二批实施 | 在统一矩形选区中增加稳定 anchor 的整行范围扩展 |

## 3. 第一批实施设计

### 3.1 保存前同步右侧大文本编辑器

当前右侧编辑器由：

- `crates/db_view/src/sidebar/cell_preview_panel.rs`
- `crates/db_view/src/table_data/cell_preview_host.rs`

管理。`CellPreviewPanel::flush_pending` 已能读取未失焦编辑器中的最新文本并回写 `DataGrid`，但工具栏“提交更改”此前直接进入 `DataGrid` 保存流程，未经过 host。

本轮将保存流程调整为：

```text
用户点击“提交更改”
    │
    ▼
DataGrid 发出 SaveChangesRequested
    │
    ▼
CellPreviewHost::flush_pending
    ├── 失败：显示原有错误并停止保存
    └── 成功：DataGrid::save_changes
                  │
                  ├── commit_cell_edit
                  └── 构造并发送数据库变更
```

约束：

- 右侧编辑器值无法解析或回写失败时，不能继续提交旧值；
- 内联单元格编辑仍由 `DataGrid::save_changes` 提交；
- 表数据 Tab 和可编辑 SQL Result 都通过 `CellPreviewHost` 承载，因此共用同一保存顺序；
- 关闭 Tab 的保存流程继续使用现有的显式 flush，不改变其交互语义。

### 3.2 当前表设计器快捷键

现有跳转链路为：

```text
DataGridEvent::OpenTableDesignerRequested
  -> TableDataTabEvent::OpenTableDesignerRequested
  -> DbTreeViewEvent::DesignTable
  -> handle_design_table
```

本轮只在 `DataGridUsage::TableData` 上下文注册新 action，复用该链路，不在 `DataGrid` 内直接创建设计器 Tab。

默认快捷键：

- macOS：`Cmd+Shift+D`
- Windows/Linux：`Ctrl+Shift+D`

快捷键 action ID 为：

```text
db.open_table_designer
```

设置页允许用户覆盖默认绑定；运行时初始化和刷新绑定都从同一个 action ID 读取。

### 3.3 文字选区对比度

右侧大文本编辑器最终使用 `gpui-component` 的全局 selection theme token。本轮只调整默认浅色和深色主题中的：

```text
selection.background
```

不为数据库编辑器增加私有颜色，避免同一种输入控件在不同页面产生不一致的选区行为。主题 schema 会限制 selection 的最终透明度，因此颜色本身需要使用更高明度/饱和度或更深的基色来提升实际合成后的可见性。

本轮目标值：

- 浅色主题：`#2563eb`
- 深色主题：`#60a5fa`

自定义主题仍可覆盖该 token。

## 4. 第二批实施与后续复杂项设计

### 4.1 #1 从数据表快捷打开临时查询

#### 第二批实施结果

用户在数据表页面触发快捷键后，新建临时 SQL 查询 Tab，并预填：

```sql
SELECT * FROM <quoted qualified table>;
```

光标进入 SQL 编辑器，但不自动执行查询。

默认快捷键：

- macOS：`Cmd+Shift+Enter`
- Windows/Linux：`Ctrl+Shift+Enter`

实现复用现有 `OpenSelectedTableQuery` action。`DataGrid` 只发出意图事件，不直接创建 Tab：

```text
OpenSelectedTableQuery
  -> DataGridEvent::OpenTableQueryRequested
  -> TableDataTabEvent::OpenTableQueryRequested
  -> DbTreeViewEvent::CreateNewQuery
  -> handle_create_new_query
```

上层事件继续提供：

- `connection_id`
- database
- optional schema
- table
- `DatabasePlugin`

SQL 生成不在 `DataGrid` 中重新拼接，而是复用现有
`format_query_table_reference`、`build_select_all_sql` 和
`handle_create_new_query`。因此 database/schema/table 的限定与引用仍由已有
`DatabasePlugin` 方言能力处理，并保持“只预填、不自动执行”的原有查询创建语义。

#### 已覆盖与待手工核验

- 快捷键只在数据表上下文生效；
- 新 Tab 使用当前连接和 database/schema；
- 特殊字符、保留字和包含空格的表名能正确引用；
- 只预填 SQL，不自动发起数据库请求；
- Tab 创建失败时保留当前页面并展示错误。

当前自动化测试覆盖 `DataGridEvent` 到 `TableDataTabEvent` 的事件映射。特殊表名引用、焦点落点和 Tab 创建失败提示仍应通过 GUI/集成路径手工核验。

### 4.2 #2 右侧编辑面板拖动宽度

#### 第二批实施结果

`CellPreviewHost` 已增加 Tab 生命周期内的宽度状态：

```rust
preview_width: Pixels
```

面板左侧增加 6px 竖向拖动柄，使用列调整光标。拖动开始时保存初始宽度和初始鼠标横坐标，后续宽度始终按初始状态计算，避免连续 drag move 累加绝对位移导致非线性变化。

- 默认宽度：420px；
- 最小宽度：280px；
- 最大宽度：800px；
- 向左拖动增宽，向右拖动缩窄；
- 拖动期间只更新局部 host 状态；
- 当前只在 Tab 生命周期中保留宽度，不写入用户设置。

纯函数测试已覆盖左右拖动和最小/最大宽度 clamp。窄窗口下根据 host 实际可用宽度动态收紧最大值、窗口缩放、打开/关闭面板、多个 Tab 独立状态和长文本滚动仍需 GUI 手工验证；动态最大宽度可作为后续小优化。

### 4.3 #4 MySQL 空闲断线恢复

#### 当前边界

MySQL 连接当前持有单个可选连接：

```text
crates/db/src/mysql/connection.rs
conn: Arc<Mutex<Option<mysql_async::Conn>>>
```

这不是连接池。TCP keepalive 只能辅助发现网络故障，无法解决 MySQL `wait_timeout` 主动关闭空闲 session 的问题。

#### 推荐策略

1. 对 MySQL driver error 做结构化分类，明确识别 connection-lost / server-gone-away；
2. 连接失效时丢弃旧 `Conn`，重新建立连接；
3. 只对明确只读且幂等的请求自动重试一次；
4. 对 DML、DDL、事务中请求和状态不明的请求只重连、不自动重放；
5. 重连后重新应用必要 session 配置，例如 database、timezone 或其他连接初始化语句；
6. 将“正在重连”和最终失败通过 UI 状态反馈，而不是让页面无响应十余秒；
7. 后续独立评估迁移到 `mysql_async::Pool`，不要把连接池迁移与本次恢复修复强行绑定。

#### 安全原则

自动重试边界必须失败关闭。若无法证明请求只读幂等，就不能重放，否则可能产生重复写入。事务连接丢失后必须明确告知事务结果未知或已经回滚，不能伪装成普通查询失败。

#### 测试

使用设置较低 `wait_timeout` 的 MySQL 容器做集成测试：

- 空闲超时后第一次只读查询自动重连并成功；
- 写请求在连接丢失后不自动重放；
- 显式事务丢失返回可识别错误；
- 错误凭据、DNS、TLS 和权限错误不触发无限重试；
- 同一请求最多自动重试一次。

### 4.4 #5 生产环境标识和可选水印

#### 数据模型

连接环境是连接级元数据，source of truth 应放在 `StoredConnection`，而不是只存在于某个 Tab 的 UI 状态。

建议模型：

```rust
enum ConnectionEnvironmentKind {
    Production,
    Staging,
    Development,
    Testing,
    Custom,
}
```

建议字段：

```text
environment_kind
environment_label
environment_color
watermark_enabled
```

涉及范围：

- `crates/core/src/storage/models.rs`
- `crates/core/src/storage/repository.rs`
- `crates/core/src/cloud_sync/models.rs`
- `crates/core/src/cloud_sync/service.rs`
- `crates/core/src/cloud_sync/connection_sync.rs`
- `crates/db_view/src/common/db_connection_form.rs`
- `crates/core/src/tab_container.rs`

字段必须有向后兼容默认值，并参与连接导入导出及 cloud sync。旧客户端遇到新字段时不得破坏连接基本信息。

#### UI 设计

- 连接树、Tab 标题或工具栏显示环境 badge；
- badge 可复用 `crates/ui/src/badge.rs`；
- 水印仅在与该数据库连接相关的内容区域显示；
- 水印层不参与 hit testing，不阻挡表格点击、编辑和拖动；
- 水印文本只包含环境标签，不能显示 credential、host 参数或完整连接串；
- 非数据库 Tab 不显示数据库环境水印；
- Production 默认使用高警示度但仍符合主题对比度的颜色。

#### 验收标准

- 环境设置在重启和 cloud sync 后保持；
- 同时打开多个不同环境连接时，各 Tab 标识不会串用；
- 水印不会拦截鼠标、焦点、键盘或文本选择；
- 自定义标签和颜色有长度、格式及可读性限制。

### 4.5 #7 表设计器按 ↓ 新增和批量复制粘贴列

#### 选区模型

当前表设计器主要使用：

```rust
selected_index: Option<usize>
```

批量复制粘贴需要升级为：

```rust
selected_indices: BTreeSet<usize>
selection_anchor: Option<usize>
active_index: Option<usize>
```

建议先把 `add_column` 内的单行控件构造提取为可复用函数：

```rust
create_column_row(definition: Option<&ColumnDefinition>, ...)
```

然后分别实现键盘新增和批量粘贴，避免复制两套行初始化逻辑。

#### ↓ 新增

只有满足以下条件时，按 ↓ 才新增一列：

- 焦点位于最后一行的列定义区域；
- 当前没有打开会消费 ↓ 的下拉框或文本输入候选菜单；
- 当前行通过最低限度校验，或产品明确允许先创建空白行。

新增后焦点移动到新行列名输入框，并保持一次按键只新增一行。

#### 剪贴板格式

Navop 自身复制优先写入 versioned JSON envelope：

```json
{
  "type": "navop/table-columns",
  "version": 1,
  "source_database_type": "MySQL",
  "columns": []
}
```

同时可提供 TSV 纯文本 fallback，便于与其他工具交互。粘贴时优先解析 Navop envelope，失败后再尝试受控 TSV。

跨数据库粘贴必须依据目标 `DatabasePlugin` capabilities 降级：

- 不支持的类型需要映射或提示；
- identity/auto increment、unsigned、collation、generated expression 等属性不能静默产生错误 DDL；
- 无法无损转换时展示逐列 warning，并让用户确认；
- 主键、索引和外键是否随列复制，应作为独立能力而不是隐式行为。

#### 测试

- 单列和多列复制顺序稳定；
- 正向/反向多选结果一致；
- 同方言粘贴字段完整；
- 跨方言不支持属性产生 warning；
- malformed/未知版本剪贴板不会修改设计器；
- undo/revert 能覆盖一次批量粘贴。

### 4.6 #9 数据表 Shift 连续整行选择

#### 第二批实施结果

实际选区状态位于：

- `crates/one_ui/src/edit_table/state.rs`
- `crates/one_ui/src/edit_table/selection.rs`

`TableSelection` 已增加：

```text
contains_row(row)
extend_row_to(row, start_col, end_col)
```

实现保持第一次普通单击行的 `anchor_row`，每次 Shift 点击只更新 active 行；正向和反向范围统一通过 `CellRange::normalized()` 处理。只有 delegate 允许多选且按下 Shift 时才扩展，否则仍重置为单行选择。此前不是 Row 模式或没有 anchor 时，Shift 点击会安全退化为单行选择。

行号入口和整行点击入口都已接入范围扩展。渲染通过 `contains_row` 判断范围内的每一行，复制继续复用现有 `selection.all_cells()`，选区列范围跳过行号列并覆盖所有数据列，不为每个单元格创建单独状态。

纯状态测试已覆盖：

- 从上向下扩展；
- 从下向上扩展；
- 多次 Shift 点击时 anchor 保持稳定；
- 范围边界判断；
- 整行范围覆盖全部数据列。

#### 后续生命周期核验

以下操作后必须清理或重新建立 anchor：

- 翻页；
- 修改分页大小；
- 查询刷新；
- 过滤或排序；
- 行删除导致索引变化；
- 数据源整体替换。

这些生命周期场景、删除/工具栏状态以及真实剪贴板结果仍需后续系统性测试。当前实现完成鼠标 Shift 连续整行选择、范围高亮与现有复制路径的状态接入，不包含键盘 Shift+方向键扩展。

#### 手工验收标准

- 单击行号选择单行；
- Shift 单击支持向下和向上连续选择；
- 再次 Shift 扩展时 anchor 稳定；
- 普通单击重置 anchor；
- 选区高亮、复制、删除和工具栏状态一致；
- 翻页/刷新后不会把旧索引错误应用到新数据；
- 大范围选择不产生逐单元格的无界状态膨胀。

## 5. 推荐 PR 拆分

为降低回归风险，按以下顺序拆分；前三项已在第二批完成：

1. **已完成：右侧面板 resize（#2）**
   纯 UI 局部状态，补拖动范围测试和浅色/深色视觉验证。
2. **已完成：临时查询 action（#1）**
   复用已有 quoted qualified table helper 与 Tab 跳转事件链。
3. **已完成：整行范围选区（#9）**
   扩展 `one_ui` selection 模型并接入范围渲染和现有复制路径。
4. **PR D：表设计器多选与复制格式（#7 第一阶段）**
   先实现状态、复制和同方言粘贴。
5. **PR E：表设计器跨方言降级与 ↓ 新增（#7 第二阶段）**
   增加 capability 映射、warning 和键盘焦点行为。
6. **PR F：连接环境模型与 badge（#5 第一阶段）**
   完成 storage migration、表单和 cloud sync。
7. **PR G：水印层（#5 第二阶段）**
   在稳定环境上下文基础上增加水印。
8. **PR H：MySQL 重连与安全重试（#4）**
   独立高风险 PR，必须带容器集成测试和明确的请求重放策略。

## 6. 风险

| 风险 | 影响 | 缓解 |
| --- | --- | --- |
| 保存顺序错误 | 提交旧值或丢失未失焦编辑内容 | 保存必须先 flush；失败时禁止继续 |
| 快捷键上下文过宽 | 在 SQL 编辑器或输入框中误触发 | action 仅挂在 TableData 的 key context |
| selection token 影响全局输入框 | 其他页面视觉发生变化 | 只改默认主题 token，并做浅色/深色抽查 |
| 自动重试写请求 | 重复 DML 或事务不一致 | 只重试可证明幂等的只读请求一次 |
| 环境字段未同步 | 不同设备标识不一致 | storage、导入导出和 cloud sync 同步演进 |
| 跨数据库列粘贴 | 生成无效或含义变化的 DDL | capability 驱动映射，无法无损时显式 warning |
| 行范围按索引保存 | 刷新后选中错误记录 | 数据变化时清 anchor，所有语义来自统一状态 |

## 7. 测试策略

### 已实施项

- 快捷键平台默认值单元测试；
- 保存前 flush 成功/失败契约测试；
- 临时查询 `DataGridEvent` 到 `TableDataTabEvent` 转发测试；
- 预览面板左右拖动和最小/最大宽度 clamp 测试；
- 整行选区正向、反向、稳定 anchor、边界和数据列范围测试；
- `db_view` 定向测试与 crate 编译检查；
- `one-ui` 选区状态测试；
- 默认主题 JSON 解析及 `gpui-component` 测试；
- 主程序编译检查，确保设置页 action ID 和 i18n 配置有效；
- 浅色/深色下手工检查右侧大文本编辑器选区和快捷键焦点行为；
- GUI 手工检查临时查询特殊表名、预览面板窄窗口拖动和 Shift 行选择复制结果。

### 后续

- 补充翻页、刷新、过滤、排序和删除后的整行 selection anchor 生命周期测试；
- 数据库 SQL 生成继续使用各 plugin 的 identifier quoting 测试；
- MySQL 使用低 `wait_timeout` 容器做真实断线恢复集成测试；
- cloud sync 使用新旧模型 round-trip 和冲突测试；
- 表设计器剪贴板使用 JSON 版本、malformed input 和跨方言能力测试；
- GPUI 拖动、焦点、键盘行为在可行处补组件测试，并保留手工视觉检查清单。

## 8. 状态跟踪

| 编号 | 状态 | 后续交付物 |
| --- | --- | --- |
| 1 | 第二批实施 | GUI 核验特殊表名、焦点和失败提示 |
| 2 | 第二批实施 | 窄窗口动态最大宽度与视觉验证 |
| 3 | 第一批实施 | 保存前 flush 和失败短路 |
| 4 | 设计完成，未实施 | 错误分类、安全重试、MySQL 集成测试 |
| 5 | 设计完成，未实施 | storage/cloud sync 模型、badge、水印 |
| 6 | 第一批实施 | action、快捷键、设置页和 i18n |
| 7 | 设计完成，未实施 | 多选状态、剪贴板协议、跨方言降级 |
| 8 | 第一批实施 | 默认浅色/深色 selection token |
| 9 | 第二批实施 | 翻页/刷新生命周期和 GUI 复制验证 |
