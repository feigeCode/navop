# 数据库值模型与二进制链路改造：设计、实施计划及交接清单

> 日期：2026-09-08。状态：**待实施的设计方案，不是已完成的修复报告**。本次只新增本文档，不修改实现。面向接手实施的模型与审查者；建议按任务编号逐项交付，不一次性重写全仓。
> 调研基线：主仓库曾核对 HEAD `cd7799331b6889264da1a188fc1c8439f57a3b6a`，同时存在其他任务的未提交修改，尤其 `crates/db/src/mysql/plugin.rs`。源码定位以“路径 + 符号”为准，行号仅为调研时参考；实施前重新确认 HEAD、工作区差异及外部扩展 revision。
> 并发变更提示：写入本文后的核验观察到 HEAD 已推进至 `4765447a9adf01d53475efbb02300d0ce712e893`；这不是本文执行了代码提交。下述审查基线不等于冻结快照，接手者须先核对相关符号的最新差异。

## 1. 结论、问题边界与优先级

**建议接受跨层重构，但不推倒重做所有数据库 SDK，也不把所有数据库强制改为 IPC。核心改造是：建立唯一、无损、带类型的查询值模型，让显示文本退出数据契约。** 二进制格式只是问题的一个表现；当前链路也存在数值、时间、未知类型、解码失败与 SQL NULL 混淆的风险。
- 当前 `QueryResult.rows` 是 `Vec<Vec<Option<String>>>`，另用 `binary_cells` 保存原始字节；已有 `typed_view()` 能保证二进制 sidecar 优先，不能把现状说成“完全没有二进制支持”。不足是双重表示、坐标同步、类型丢失，以及部分驱动在生成结果前已经丢失信息。
- IPC 已有 `CellValue` tagged union、base64 Bytes 和字符串 Decimal；宿主 `cell_to_display_value` 又把大部分类型转换成字符串。因此“改 IPC 编码”不等于解决宿主、原生驱动、编辑器和导出问题。
- 截图中的名称、字符集和排序规则显示为 `0x...`，支持“本应作为文本消费的元数据经过了字节/十六进制展示链”的判断；**不能仅凭截图断言实际数据库被重命名、存储损坏，或锁定某个服务器版本/连接参数为唯一根因**。真实连接的列类型、flags、collation、会话字符集以及运行中的构建信息尚需采集。
- “有些数据库正常”需要按驱动、服务器版本、会话字符集、查询形态、字段元数据、原生/IPC 路由和实际构建分别解释；禁止把“内容恰好能解成 UTF-8”作为文本类型判断依据。
- P0：禁止把解码失败变成 NULL，禁止丢弃/裁剪原始字节，禁止从显示字符串写回；P1：统一类型、修复元数据与各驱动映射、迁移消费者；P2：完善大值流式资源、性能与扩展类型覆盖。所有 P0 必须先有失败测试。

### 1.1 “二进制”的三种含义必须分开
| 层次 | 本次范围 | 不应采用的错误推论 |
| --- | --- | --- |
| 数据值：BLOB/bytea/RAW、文本编码、精确数值、时间等 | 所有 SQL 驱动与所有结果消费者 | 字节可打印，所以它就是文本；hex 字符串就是原值 |
| 传输：数据库原生协议、IPC JSON/base64、WIT DTO | 在已有协议之上做正确映射、协商、错误和资源控制 | 使用二进制 wire format 的 PG 字段就是 bytea |
| 可执行程序：宿主、扩展 sidecar、客户端动态库及打包产物 | 核对构建、选择、启动、版本、依赖、旧安装残留和生命周期 | 内置 MySQL/PG 客户端等于应用正在启动 mysqld/postgres 服务端 |

## 2. 现状证据与应保留的有效设计

| 位置 / 符号（调研行号） | 已观察到的行为 | 改造含义 |
| --- | --- | --- |
| `crates/db/src/executor.rs`：`QueryResult`、`BinaryCell`、`typed_view`（143–274） | 字符串 rows + 字节 sidecar；已有宽度、重复/越界坐标校验 | 保留校验与兼容读入能力；最终去掉双份权威值 |
| `crates/db/src/mysql/connection.rs`：`extract_query_cell`、`extract_character_cell`、`is_binary_wire_value`（255–398） | 已结合类型、BINARY_FLAG、collation 63；文本严格解码失败时保留 bytes；hex 预览有长度限制 | 不撤销严格解码或改成 UTF-8 猜测；保留“未解码文本”与真正 Binary 的区别 |
| 同上：`session_charset`、`build_init_commands`（114–164） | 已有版本相关字符集选择；默认路径设置结果字符集，显式路径可 SET NAMES | 不能再按“完全没初始化字符集”的旧假设修补；验证池中新连接、重连、配置变更 |
| `crates/db/src/mysql/plugin.rs`：`restore_mysql_metadata_text`、`list_databases*` | 工作区已有针对元数据列的文本恢复修改 | 属于别人正在处理的补丁；先用测试保护，目标 DTO 通路接替后再删除过渡逻辑 |
| `crates/db/src/postgresql/connection.rs`：`extract_value`（610–738） | 多处 `try_get(...).ok().flatten()`；时间字符串化；数组以类型名后缀识别 | 失败与 NULL 分离；按类型身份/Kind 处理数组；保留精度；现有 Numeric 精确解码应复用 |
| `crates/db/src/mssql/connection.rs`：`extract_value`（37–85） | 尝试有限 Rust 类型，失败后继续尝试/返回 None；缺少若干常用映射 | 按 Tiberius 列类型分派，覆盖 tinyint/smallint/real/Numeric/Uuid 等；不是所有 binary 都已丢失 |
| `crates/db/src/sqlite/connection.rs`、`crates/db/src/duckdb/connection.rs`：`extract_value` | Text 的 UTF-8 转换使用 `.ok()`；Blob 已有 sidecar；DuckDB 部分未知值走 Debug | 非法文本不能变 NULL；运行时类型优先；Debug 不能成为可写回原值 |
| `crates/db/src/oracle/connection.rs`：`extract_value`、二进制提取分支 | 多次 String/数值尝试；RAW/LOB 有字节读取路径 | 类型定向解码、精度与 LOB 生命周期需要补齐；不能未经验证就认定 BFILE 的 Vec<u8> API 不支持 |
| `crates/db/src/clickhouse/connection.rs`：`fetch_json_compact`、`map_rows`、`execute_single` | JSONCompact 中转；FixedString 去尾 NUL；失败路径可能再次执行同一 SQL | 获取路径要能保留任意字节；禁止在解码失败后盲目重执行，尤其有副作用语句 |
| `crates/db/src/tdengine/connection.rs`：结果构造与值格式化（103–142） | 值转字符串，结果没有字节 sidecar | 依据 SDK Value 映射；必须区分文本型 BINARY/VARCHAR 与字节型 VARBINARY，不能看名字猜 |
| `crates/db/src/ipc/connection.rs`：`FetchedRows`、`cell_to_display_value`（584–641） | Bytes 解码后保留 sidecar，但数值、JSON、时间、数组等被字符串化 | 直接 wire value → 宿主 typed value；移除结果获取阶段的显示格式化 |
| `crates/db_view/src/table_data/{data_grid,results_delegate}.rs` | rows、binary maps、original maps、字符串编辑前后值并存 | UI 只持有 typed batch 与 edit overlay；隐藏列、排序、分页不能再同步搬运多份坐标表 |
| `crates/extension-runtime/src/extension_db_gateway.rs`：`sql_results_to_row_batch` | 另有 extension/WIT-facing 值模型转换边界 | 显式适配而非强行合并 WIT；MCP/AI 的直接 QueryResult 依赖本次未证实，需沿 gateway 核验 |

## 3. 业界参考与取舍

参考的是可核验的一手实现和契约，而不是照搬某个产品全部架构。下列来源的链接与核验说明见第 15 节。
- **PostgreSQL protocol / postgres-types**：数据库类型与 text/binary 传输格式分开；String 与 Vec<u8> 接受的数据库类型不同，SQL NULL 与转换错误是不同结果。采用“按原生类型解码 + 明确错误”的模式，不用“试一串类型直到成功”。PG 二进制表示仍应交给 SDK 或局部受测 codec，不另写整个协议栈。[S1][S2]
- **DBeaver**：值处理接口区分读取、绑定等行为，内容对象另有内容访问/展示职责。借鉴“语义值、数据库读写、显示、大内容资源分层”，不引入一个同时负责 SQL、UI、进程和导出的巨大 ValueHandler。[S3]
- **SQLite**：值有运行时 storage class，列声明的 affinity 不等于每个单元格实际类型。声明为 DATE/TEXT 的列不能授权客户端随意改写其 INTEGER/BLOB 原值。[S4]
- **Tiberius**：官方 FromSql 映射明确区分 u8/i16/i32/i64/f32/f64/Numeric/Uuid/字节等。按 SDK 能力建立映射矩阵，验证 Cargo feature，而不是以 String、i64、f64 三种尝试覆盖所有类型。[S5]
- **Go MySQL driver**：`fields.go` 根据原生 field type 与 binary collation 区分 TEXT/BLOB、VARCHAR/VARBINARY 等，而不是扫描内容猜 UTF-8。借鉴“元数据驱动”的判断方式，不把某个驱动的局部规则直接替代本仓全部 flags/collation/表达式处理。[S7]
- **ClickHouse**：选择能表达列名、类型与原始值的获取格式；RowBinaryWithNamesAndTypes 是候选，不代表必须立即手写完整解析器。先做 String/FixedString、Nullable 与复合类型的 SDK 能力验证，再决定使用 typed SDK、格式适配还是局部 codec。[S6]
- **不采用的路线**：继续到处增加 hex/base64 特判；凭字符串反推 SQL 类型；一次性换成 SQLx 解决所有驱动；让所有驱动都走 IPC；把 Arrow 作为全应用强制值 ABI；发明替代已有 driver.json/extension.json/WIT 的“大统一扩展协议”。这些改动不是解决当前问题的必要条件。

## 4. 目标架构与模块边界

推荐新增纯模型 crate **`db-value`**（本文中的新类型、模块、capability 均为拟定名称，不是仓库已存在 API）。它只承载值、类型描述、批次、校验和纯比较辅助；不得依赖 GPUI、Tokio、数据库 SDK、`db` 或 extension runtime。避免放进已有带 UI/连接职责的 `db` 再迫使低层协议反向依赖。
```text
native SDK values ── 各驱动 codec ────────┐
IPC CellValue ────── db::ipc adapter ────┼─> db-value::ResultBatch（唯一值源）
legacy QueryResult ─ 临时兼容入口 ───────┘          │
                       ┌─────────────────────────┼─────────────────────────┐
                   metadata DTO             db_view formatter        export/compare/gateway
                                                │
                                        typed edit / parameter bind
```
- `extension-protocol` 保持独立 wire DTO；转换放 `db::ipc`，不要为共享枚举让它依赖 `db`。`extension-host` 管进程与 transport，`db` 管 SQL 语义；`db → extension-runtime` 仍禁止，以免成环。
- `extension-api/wit`、`extension-component`、`extension-wasm` 保留契约边界。若要扩展 WIT 值能力，独立版本化；不要因 Rust API 无直接引用而删除 WIT 源。
- `IpcDriverRegistry(driver.json)` 与 `ExtensionRuntimeCatalog(extension.json)` 保持既有分层。Redis/MongoDB 保留各自领域类型；本方案不顺手改造它们的命令协议、文档模型或运行时。
- 建议新文件按职责拆分：`crates/db-value/src/{value,cell,column,batch,error,temporal,decimal}.rs`；`crates/db/src/value_codec/` 放公共读写辅助与兼容入口，各驱动目录放自己的 codec；`crates/db_view/src/table_data/value_format/` 放展示与输入。遵守仓库文件/函数上限，不能继续把逻辑堆进巨型 connection/delegate 文件。

### 4.1 唯一权威值与错误模型
| 拟定类型 | 必须表达的语义 | 禁止项 |
| --- | --- | --- |
| `DbValue` | Null、Bool、Int、UInt、Float、Decimal、Text、Binary、BitString、Temporal、Json，以及明确支持的复合值 | 统一 String；将 UTF-8 BLOB 自动升级为 Text；将 SQL NULL 和空值合并 |
| `CellState` | `Decoded(DbValue)`；`Undecoded { native_type, raw, reason }`；`DecodeError { native_type, raw, diagnostic }` | 错误 `.ok().flatten()`；未知值伪装成 NULL、`<TYPE>` 或 Debug 文本 |
| `RawPayload` | Inline shared bytes 或受控的大值引用；同时记录获取表示与来源；不能读取原始值时为显式 Unavailable 原因 | 声称 SDK 没有暴露的原始 bytes 已被完整保存；把未解码 PG wire bytes 当成 bytea |
| `ColumnDescriptor` | 稳定列 ID、原标签、native type 身份、逻辑类型、可空性 Unknown/Yes/No、精度/scale、编码、collation、来源（若可靠） | 用列名作唯一 ID；仅 Debug 类型字符串；不知道 nullable 时默认 false |
| `ResultBatch` | QueryId/generation、schema、rows、完成/截断状态与统计；构造时校验行宽和身份 | 公共可变双份 rows/sidecar；未标记地丢弃尾页；把部分结果当完整结果 |
- `CellState` 的错误定位由 QueryId/RowId/ColumnId 携带；致命 protocol/schema/batch 结构错误终止结果，单个数据解码错误可允许只读展示但必须醒目标记。元数据、写回、无损导出遇到错误值必须明确失败，不能复用 UI 的容错预览。
- 新模型不要求第一阶段解码全部数据库自定义类型；未知类型有安全的 Undecoded 路径即可。已支持类型必须精确；有 raw 才能承诺 raw 往返。无法获取 raw 时必须保留原因并禁写，不许编造字节。
- 状态产生规则固定为：没有 decoder/不支持该原生类型 → Undecoded；选中支持的 decoder 后校验/转换失败 → DecodeError；SDK 明确返回 SQL NULL → Decoded(Null)。`RawPayload::Unavailable` 只是上述异常状态能否取回原始表示的能力，不是第三种单元格错误状态；不得为避免报错把 DecodeError 改记为 Undecoded。

| 状态 | 显示 / 复制 | 绑定 / 比较 / 常规导出与备份 |
| --- | --- | --- |
| Decoded（含 Null） | 按语义显示；完整复制不取 preview | 仅使用对应操作已支持的 codec；不支持的操作明确失败 |
| Undecoded + raw 可用 | 显示类型/原因；用户可显式复制诊断 raw，并标明 wire/native 表示 | 默认禁绑定、同步和语义比较；常规无损备份拒绝，不把 raw 伪装成 Binary 值 |
| DecodeError + raw 可用 | 错误标识；允许显式导出诊断 raw | 禁绑定/同步/常规备份；raw 诊断附件不算可恢复数据备份 |
| Undecoded / DecodeError + raw 不可用 | 显示原因；不能提供“复制原始 bytes” | 禁绑定/同步/无损导出；不可用状态不能生成伪 payload |

- Float 保留 f32/f64 宽度和可保留的位级表示，明确 NaN、±Infinity、负零；Decimal 不经过 f64，采用精确系数/scale 或受验证的精确十进制对象，独立表达数据库支持的特殊数值。原生类型身份另存，不靠值大小猜原始 SQL 类型。
- Temporal 分开 Date、Time、LocalDateTime、Instant/OffsetDateTime、Interval；保留支持的精度与合法范围，并覆盖 PG infinity、MySQL zero-date/超 24 小时 TIME。PG timestamptz 不承诺恢复服务器已不保存的原始时区；显示时区是策略，不是修改原值。
- Json 保存保真文本或不损精度的表示及 native JSON/JSONB 身份，不经 f64 中转；JSONB 不承诺原始空白/键顺序。Array 保留元素类型、维度、下界及元素 NULL；域、枚举、range/composite 等保留原生身份，未实现的进入 Undecoded，禁止先 stringify 再假称无损。
- 只有原生类型需要时才实现 Map/Record/Geo 等语义变体；先明确“查询展示、比较、绑定、导出”各操作能力。必须防止复杂变体在一个出口被悄悄降为 Text；不支持就返回具名 UnsupportedOperation。

### 4.2 字节所有权、批次、行身份与大值
- 小/中值使用共享不可变字节（例如 `Arc<[u8]>`），显示 hex/base64 按需生成且缓存有界；不得同时持有原始 Vec、完整 hex、完整 base64 及每个消费者的拷贝。文本与已解码标量无需默认再保存整份 wire bytes；只有精度/审计需要且预算允许才保留。
- LOB 引用只含不透明资源 ID、长度（可未知）、类型和 generation；resolver/lease 在 `db` 的查询资源所有者中，不放纯值 crate。读取按块、有取消、有超时；结果关闭、事务结束、断线后的行为必须明确。需要跨会话导出时先取得有效 lease 或有界 spool，不能留悬空 locator。
- 批次按行数和字节预算双限制，限制单 cell、嵌套深度、IPC frame/base64 解码输出及预览；数值预算在 T00 用基线确定并成为配置/常量测试，不能只写“避免 OOM”。大 LOB 的完整导出另走流式通道，不能绕过预算一次性加载。
- `RowId = QueryId/generation + 原始结果行序号`；排序/筛选改变 view order，不改变身份。重新查询产生新 generation，旧异步解码/编辑结果不得回填。这个 RowId 只标识结果行，**不是数据库更新键**；写回仍需要可靠主键/唯一键、原表来源和原始值。

### 4.3 接口落点与转换契约（拟定，不是现有签名）
| 接口 / 所有者 | 输入与输出 | 不变量与迁移方式 |
| --- | --- | --- |
| `ResultBatch::try_new` / db-value | schema、generation、typed rows、completion → batch 或结构错误 | 字段保持私有；对外借用 cell/row；切片/投影保留稳定列与行 ID |
| `TypedSqlResult` / db | Rows(batch)、AffectedRows、statement error 等执行结果 | 保持多 statement 顺序和现有 affected rows/耗时语义；不是把所有 SQL 强制解释为表格 |
| typed query 执行入口 / db 的连接与执行层 | 现有执行上下文、SQL、typed parameters、查询限制 → typed results/批次 | 先盘点 DbConnection/SqlExecutor/manager 真实签名再加适配；未迁移驱动允许集中 legacy adapter，但一次请求只执行一次 SQL |
| native codec / 各驱动 | SDK 原生类型和值 → CellState；typed parameter → SDK bind value | 读取与绑定分别报告能力；不要求编造跨 SDK 的万能 Row trait；不能绑定时明确错误而非插值兜底 |
| checked metadata reader / db | cell、预期文本/整数类型、元数据列语义 → 领域 DTO 或具名错误 | 必填字段遇 SQL NULL 与遇解码错误分别报告；业务对象名不接受 Binary preview |
| display formatter / db_view | 借用 CellState、列信息、显示策略、预算 → preview | 只能向显示方向转换；参数绑定、比较、导出不能调用它获取原值 |
- 临时 `LegacyText` 是**来源/能力标记**，不是新的“可以猜解的字符串值”：可实现为 cell provenance。即便列声明为数值，legacy 文本也不因此变成已验证的精确数值；只有明确受测的 importer/parser 才能转换，并记录转换规则。Int/UInt 实现应覆盖已支持 SDK 的 i128/u128 或精确大整数需求，否则明确拒绝，不经 f64。

## 5. 读取、元数据、显示与写回契约

### 5.1 读取与元数据
原生 SDK/IPC 完成一次类型解码后立即进入 ResultBatch；UI、备份与比较不得再次猜解原始内容。每个驱动建立 `native type → logical type → decoder → binder → exporter` 表；先读列元数据再用匹配 codec，动态类型库额外读每个值的 storage class。
- `DatabaseInfo/SchemaInfo/TableInfo/ColumnInfo` 等领域 DTO 必须通过 checked getters 构造，不用表格显示 formatter 产生数据库名。MySQL 元数据固定文本列可使用有证据的会话/结果编码规则严格恢复；失败返回含列来源的错误，不把 `0x...` 当合法对象名继续导航或拼 SQL。
- `query_result_normalization.rs` 中的 schema 辅助只用于来源明确的场景；不把列名相同当成来源相同，不猜 join/alias/expression 的字段类型。结果字符集与表定义字符集分开；会话覆盖必须有 provenance。重复输出标签允许存在，通过列 ID 区分。
- 连接初始化要覆盖首次连接、池扩容、重连、切库、用户显式字符集/排序规则和旧服务器能力；不要为截图无条件执行 SET NAMES，也不要把一次初始化成功视为所有池连接均正确。

### 5.2 UI、编辑及参数绑定
- DataGrid/EditorTableDelegate 只持有 ResultBatch 引用、view order、选择状态、typed edit overlay 与有界 preview cache。移除独立 binary maps/original binary maps；原始快照引用原 batch，编辑器保留“用户输入草稿”和“解析成功的 typed value”，二者不能互相冒充。
- formatter 只负责显示：NULL、空文本、空字节、未解码文本、真实 Binary、解码错误有不同状态；binary 可切 hex/base64/显式文本预览，预览截断需显示总长度与截断标记，切换方式不改变原值。错误不能显示成空白而无标识。
- 排序、筛选和比较使用 typed value 与明确策略；NULL 位置、NaN、跨类型排序、数据库 collation 与客户端排序差异必须定义。不能声称本地 Unicode 排序等同服务器 collation；必要时提供服务器排序模式。
- Binary 编辑必须由明确的 hex/base64/文件模式解析，Text 中的 `0x4142` 永远仍是 Text；区分清空、设 NULL、空 bytes、空 string 和文本 `NULL`。保存前验证长度、精度、目标列类型与是否可写；Undecoded/DecodeError 默认只读。
- 更新/插入走 typed bind parameters，Typed NULL 按目标 schema 绑定；WHERE 乐观并发条件使用原始 typed 值及可靠键。列/表标识符使用 dialect quoting，不拼接用户值。多表/表达式/来源不可靠的结果不得假定可编辑；失败时保留草稿、回滚可回滚事务并显示实际失败单元格。
- GPUI 只做展示和状态更新；数据库 Future 通过状态层的 Tokio-bound 入口执行，不在 render 或普通 `background_spawn` 直接轮询依赖 Tokio 的数据库操作。取消/刷新按 generation 丢弃旧结果；UI 测试优先纯状态 contract，再做真实 view 测试。

## 6. 各内置驱动实施要求

| 驱动 / 当前入口 | 必须完成的映射与约束 | 最低验收样例 |
| --- | --- | --- |
| MySQL：`crates/db/src/mysql/connection.rs`、`plugin.rs` | 保留正确 flags/type/collation 判定；Text 解码错误与 Binary 分开；metadata DTO；BIT 位长、DECIMAL、TIME/zero-date；所有建连路径的字符集 | 同连接下 schema 名/字符集显示为文本，UTF-8 BLOB 仍为 Binary；非法文本有错误和 raw；默认/显式编码、旧服务器、重连均覆盖 |
| PG：`crates/db/src/postgresql/connection.rs` | 复用 PostgresNumeric；替换吞错；BYTEA 独立；按 OID/Kind 区分 array/domain/enum；保留时间精度、infinity；未知 codec 安全退化 | int/decimal/null/空 bytea、纳秒能力对应精度、timestamptz、数组 NULL/维度/下界；类型错误绝不变 NULL |
| MSSQL：`crates/db/src/mssql/connection.rs` | 按 Tiberius ColumnType 分派 u8/i16/i32/i64/f32/f64、Numeric、Uuid、Xml、时间与 offset；检查 feature gates | tinyint=255、smallint、real、decimal 高精度、GUID、varbinary/image、datetime2 精度、offset；NULL 与失败分离 |
| SQLite：`crates/db/src/sqlite/connection.rs` | ValueRef storage class 决定值；声明 affinity 仅为 schema；非法 Text 保留异常；移除影响原值的日期猜测 | 同一列混合整数/文本/BLOB/NULL；非法 UTF-8 Text；空 BLOB；声明 DATE 的整数不被无条件改写 |
| DuckDB：`crates/db/src/duckdb/connection.rs` 与外部 IPC DuckDB | 同时覆盖 `builtin-duckdb` 开/关；精确 Decimal/大整数、时间与支持的嵌套类型；未支持的禁用 Debug 当原值 | 原生与 IPC 查询相同 fixture 的语义/字节一致；超 i64 的类型有准确表示或明确 Unsupported，不经 f64 |
| Oracle：`crates/db/src/oracle/connection.rs` | 按 Oracle type 解码 NUMBER、日期/纳秒时间、RAW/LONG RAW/BLOB/CLOB/BFILE；明确 OCI/SDK能力和 locator 生命周期 | 高精度 NUMBER、RAW 空/非 UTF-8、CLOB 编码、时间小数、LOB 分块/取消/断线；BFILE 权限与外部文件缺失明确报错 |
| ClickHouse：`crates/db/src/clickhouse/connection.rs` | 先选无损读取路径；禁止 FixedString 去尾 NUL 损失原值；String 不默认 UTF-8；保留 Nullable、Decimal、DateTime64 与已支持复合类型；执行与解码失败分开 | 0..255 bytes、尾 NUL、非法 UTF-8、NULL、DateTime64 精度；注入结果解码失败证明带副作用 SQL 不被二次执行 |
| TDengine：`crates/db/src/tdengine/connection.rs` | 核对锁定版本的 taos Value；区分文本 BINARY/VARCHAR/NCHAR 与 VARBINARY；时间精度按服务器配置；不使用兜底 Display 写原值 | 文本/宽字符、raw bytes、空值、NULL、时间精度、未知 Value；未支持版本给出明确能力限制 |

每个驱动必须交付“读取/绑定/导出支持矩阵”，不能只用 SELECT 显示正确验收。数据库返回的物理表示与逻辑值允许不同，但恢复到同库同类型时必须符合定义的等价关系：Binary 按字节相同，精确数值按数值及必要 scale，时间按类型/精度/瞬时语义，JSONB 按语义而非原始排版。

## 7. IPC、WIT 与可执行程序的专项方案

### 7.1 IPC：先停止降级，再增量扩展协议
- 现有 `crates/extension-protocol/src/row.rs` 已有 `CellValue`、`ColumnSpec`，`ParamValue = CellValue`；`query.rs` 已传 typed params/rows。第一步保持现有 variant/serde 形状，将 `db::ipc::cell_to_display_value` 替换为直达新值模型的 adapter，完整保留 type_kind、precision、scale、nullable、extra。
- 现有 Bytes 必须严格校验 base64 和解码后大小，保持空 bytes 与 NULL 区分；Decimal 不转浮点；Datetime 不能去时区/精度后作为权威值。当前 wire 本身不能表达的精确语义标记为能力限制，不能靠宿主猜回来。
- 后续才扩展 undecoded/error/native type identity/大值 handle/更精确 temporal 等能力。复用 `lifecycle.rs` 的 `api_offered/api_used/features/methods`，明确一个实际被双方选中的版本与能力集合；拟定 `query.typed-values.v2` 等名称须先查重和写协议测试。**仅给 serde 新增 enum variant 或 default 字段，不等于旧宿主能兼容。**
- Legacy 会话只发送旧宿主认识的旧 wire shape；无法无损表示的新值应明确拒绝相应操作或返回约定错误，不能伪装成 Text。新宿主读旧扩展先保留已提供的类型；扩展版本升级与宿主模型迁移分开发布。
- T03 必须交付逐 variant 的双向能力表：旧 wire → 新模型、新模型/参数 → 旧 wire，分别标记 lossless / display-only / reject；display-only 仅能进入 UI 投影。任何未知状态、时间/数值精度不足、不可表达参数均须具备拒绝测试；协议结构无法解析时终止该响应，不构造假 NULL。矩阵与测试未完成不得进入 T06；T10 再扩展不能豁免这道门禁。
- 外部仓库 `../navop-extensions/Cargo.toml` 当前曾锁定主仓 `4e18a15e6f98552736563fe91aa5e1ce1ad61dba`；须检查并同步实际依赖 revision 与 lockfile。至少覆盖 `extensions/ipc/duckdb/src/{value,result}.rs` 和 `tests/{extension_protocol,cancel_timeout}.rs`；其中 typed query 与 legacy string result 并存，不能只改一条。
- 外部其他语言驱动必须在 T00 枚举 manifests、启动命令、语言和 SDK，再扩展同一组协议 fixtures；不能用 Rust 编译通过代表 Go/Java/Python/JS 驱动通过。特别测试 JSON 跨 JS 的 i64/u64 精度、异常浮点、base64 大帧和未知 tags；需要 string 编码的升级只在协商后的新契约使用。
- WIT/gateway 的 `DbValue/RowBatch` 不是新模型别名。现有类型可准确映射则转换，不能表达的数值/时间/错误按旧契约明确失败或走新版本；不得把 decimal/大整数强塞 Float。保留多 statement、cursor、权限和取消语义，不趁机换公共 MCP envelope。

### 7.2 内置/扩展程序、加载与启动
- 先从 `crates/db/src/manager.rs`、`crates/db/Cargo.toml`、IPC registry/client 和发布脚本建立“数据库 → 选中后端 → SDK/动态库 → 是否子进程 → 路径 → 构建版本”清单。MySQL/PG 当前是宿主原生客户端路径；DuckDB 可因 feature 选择原生或扩展。若本机实际另有 mysqld/postgres 管理器，须列为独立发现，不能从名称推断存在。
- 增加脱敏诊断快照：host version/commit/build target/features、实际 executable 路径、backend kind、SDK 锁定版本、服务器版本、扩展 ID/version/manifest 路径/可执行文件校验值、协议协商结果、会话编码来源。连接密码、完整 URL、SQL 参数和原始 cell bytes 默认不记录。
- 对真正的 sidecar 检查 resolver 选择、架构/OS 匹配、安装完整性、启动失败、stdout 协议/stderr 日志隔离、退出/取消/超时、升级后旧进程和旧路径复用；能力变化使旧 session 失效，按正常生命周期重建，不能仅换文件仍沿用旧握手。未知 binary 不为探测而自动执行。
- 对内置路径验证 release 包确实包含新宿主代码与期望 features；动态客户端依赖检查加载位置和版本。数据库值修复不需要去替换用户的数据库服务端程序。跨平台至少验证打包安装启动，不能只测 `cargo run`。
- 本项目标是可追溯、可复现和版本不混用，不是重写所有进程框架/安装器。只有复现出的启动或产物选择缺陷才单独修复；截图的十六进制显示不能直接归因于打包或旧二进制。
- T00 的 artifact manifest 必须包含 `backend / host-or-sidecar / target / executable-path / hash / version-commit / features / SDK-revision / dynamic-dependencies-or-statically-linked / selected-manifest / negotiated-capabilities`。MySQL/PG 的 Rust SDK 静态链接到宿主时明确记录“无独立客户端 executable/动态库”，不得要求伪造 libmysql/libpq 清单。

| 产物验收环境 | T11 必须留存的证据 |
| --- | --- |
| macOS `.app` | 按仓库 bundle 脚本的实际参数构建/安装；核对 Bundle executable、架构/签名、动态依赖；从安装包启动后采集诊断并执行 MySQL/PG/IPC fixtures |
| Windows 安装包 | 从安装目录启动；核对 PE 架构、版本/hash、实际加载依赖和 sidecar 路径；覆盖升级后旧进程/旧扩展目录不误用 |
| Linux 安装包 | 核对安装后的 ELF/依赖与 extension resolver 路径；从桌面入口启动而非开发 shell；覆盖环境变量/PATH 不同时后端仍可追溯 |
| 共用判定 | T00 读取实际发布脚本后补充可执行构建/安装/验收命令，不猜脚本参数；T11 保存包 hash、安装启动记录、连接结果和诊断快照，Cargo 单测不能替代此表 |

## 8. 导入导出、备份、复制与比较

| 消费者 | 新契约 | 兼容要求 |
| --- | --- | --- |
| SQL dump / SQL export | typed value → 方言 literal codec；Binary 使用已有 dialect binary literal；标量精确格式化 | 保留 `sql_export.rs` 已有 typed_view 校验与二进制支持；未知/失败值终止，绝不输出预览 |
| CSV/TSV/普通 JSON | 默认维持已有显式限制；用户选择 binary encoding 时记录规则；NULL、空文本、空 bytes 需可区分 | 不悄悄把 Binary 变 hex 字符串并声称可逆；CSV 本身无类型，往返需要 schema/编码约定 |
| XML / 现有备份格式 | 用现有格式契约承载类型与字节；审计并保留已有可用编码 | 先跑旧 golden/restore fixtures；不能为新模型随意改变旧文件含义 |
| 可选 typed archive | 独立版本化 envelope/schema；显式 value tag、bytes base64、decimal 精确文本、错误/未知策略 | 不是第一阶段强制新产品功能；若现有格式能无损则优先复用；旧 reader 拒绝未知版本 |
| Clipboard | “复制显示文本”与“复制精确值/bytes”明确区分，选区按稳定 ID 读取 | 单元格预览截断不影响完整复制；JSON/CSV copy 与导出编码策略一致 |
| 表编辑 / 表导入 | 输入解码 + schema-aware typed bind；导入失败给出行列与原因 | 不经显示 formatter；NULL token、空值和类型转换按导入配置，不改变历史默认而无迁移提示 |
| 数据比较/同步 | typed equality/hash，原始字节与精确数值，明确跨库类型/时区/collation 策略 | `compare/data_paging.rs` 到 RowData 不得再降成显示字符串；同步 SQL 必须来自 typed bind/literal |
| Extension gateway / 间接 MCP、AI | 适配各自版本化 DTO，保留类型和明确错误；大值输出受预算控制 | 先确认实际调用链；不杜撰直接 QueryResult 消费；已有响应协议不能无条件替换 |

执行导出时应记录结果是否完整、数据快照/事务策略与已知类型降级；失败不能留下看似成功的文件，采用临时文件完成后发布或显式 incomplete 标记。只读 UI 可以显示部分结果，无损备份默认不接受包含 DecodeError/Unavailable 的单元格；“跳过错误行”必须是显式选项并有审计计数。

## 9. 迁移策略：扩展 → 迁移 → 收缩

1. **先增加新契约和测试，不改旧数据的含义。** 新 ResultBatch 与旧 QueryResult 作为边界类型并存，不在同一查询结果中维护两套独立可变权威 rows。旧 producer 通过集中 adapter 进入新 batch；有 sidecar 的位置以 bytes 为准，其他位置按 LegacyText 标记来源，禁止解析 `0x...` 或猜数值。
2. **建立最小贯通链路。** 先用 MySQL + PG + IPC fixture 接通 typed batch → 元数据/基础 DataGrid → typed 写回与 SQL 导出，证明确实减少降级；后续驱动再并行迁移。临时给旧只读消费者的显示投影必须命名为 display/legacy projection，不能给写回、备份、同步作为原值。
3. **逐个迁移消费者和驱动，集中收口兼容层。** 每迁移一个 producer/consumer 更新覆盖清单；新路径禁止读取 legacy rows。兼容入口对能力不足明确标记，不能假装旧字符串重新变得精确。
4. **最后移除旧模型及补丁。** 只有全部强语义消费者迁移、跨仓兼容通过、真实库和打包验收完成后，才删除 BinaryCell、sidecar、旧 formatter-as-decoder 和临时元数据恢复。旧持久化 JSON 的读取如仍有真实使用，保留独立版本化 DTO importer，不让历史格式继续决定运行时模型。

## 10. 分阶段实施任务与验收

所有状态初始为待办。依赖表示最低先决条件；每项独立提交、先 Red 再 Green，交付具体测试命令/输出与未覆盖项。**任务范围不等于一个模型一次吞下全部改造**；超过仓库上限时继续按 codec/consumer 拆分。
| 编号 / 依赖 | 修改范围与实施动作 | 交付与验收门禁 |
| --- | --- | --- |
| T00 基线 / 无 | 记录 HEAD/dirty、features、外部 manifests/revision 与第 7.2 节 artifact manifest；定位所有 QueryResult/sidecar/显示写回消费者；实际运行包复现截图并补等价原始列元数据 fixture；检查现有测试启动方式 | 输出可复现输入、driver/artifact matrix，为每项 P0 建失败测试；真实连接不可得可先用 fixture 推进模型，但“截图根因/运行包问题”保持未验收，fixture 不替代证据；不改用户库；确定各预算与发布测试命令 |
| T01 模型 / T00 | 新增 `db-value` 与 workspace 依赖；实现 CellState、ColumnDescriptor、受校验 ResultBatch、基础 scalar/bytes/decimal/temporal、raw availability | 纯 crate 无 GPUI/Tokio/SDK；类型/NULL/错误/宽度/身份 property tests；序列化只在明确边界实现，禁止直接把内存 enum 当永久 wire ABI |
| T02 桥接 / T01 | `executor.rs`、`value_codec/legacy`、SqlResult/执行接口增加 typed 路径；旧数据只经集中 adapter；定义 display-only 投影与拒绝有损强操作 | 旧 JSON fixtures 可读；sidecar 权威、重复/越界失败；无 hex 猜解；新 typed 路径不再生产 sidecar；标记 LegacyText 的能力限制 |
| T03 IPC 保真 / T02 | `ipc/connection.rs`、`import_parse.rs` 适配现有 CellValue/ColumnSpec；暂不强制改 wire；完成第 7.1 节双向能力表 | 旧扩展已表达的类型不经显示中转；逐 variant lossless/reject 测试，禁止有损强操作；非法 base64/大帧失败；现有 IPC contract、cursor/cancel 不回归 |
| T04 MySQL / T02 | 拆出 MySQL codec，迁移结果与元数据 DTO；覆盖 flags/collation/session；保护既有 plugin 补丁 | 原生查询与数据库列表贯通；Binary/未解码 Text 分离；默认/显式编码与池重连测试；截图 fixture 通过，实际包核验记录 executable/hash/features/SDK/会话元数据，否则不声称实际故障已修复 |
| T05 PG / T02 | 拆出 PG codec，复用 Numeric，按 OID/Kind 解析；去掉吞错和有损格式化 | NULL/错误分离；BYTEA、decimal、时间、数组、unknown 测试；已实现类型精确，未实现类型明确只读且不伪造值 |
| T06 最小 UI 贯通 / T03–T05 | `sql_result_tab.rs`、`data_grid.rs`、`results_delegate.rs` 引入 typed batch/view order/edit overlay；同步建立 typed 隐藏列/`__rowid__` 投影，供后续消费者复用 | MySQL/PG/IPC 同表格契约；排序/隐藏列/rowid 剥离后不串值；旧 generation 不回填；不要求此时所有数据库均完成原生迁移 |
| T07 单表写回/SQL 导出贯通 / T06 | 先 MySQL/PG 单表 typed editor/parser/binder、原始值 WHERE、SQL literal；复用 T06 投影，不改尚未迁移的消费者 | NULL/空值/bytes/decimal/时间读改写读；单表 SQL export/restore fixture；拒绝 legacy/unknown/preview 写回；此阶段不宣称全局 dump、分页备份或多 statement 导出已完成，交 T09 验收 |
| T08 其余原生驱动 / T02，合入前 T06–T07 | 分独立任务：MSSQL+SQLite；DuckDB 原生；Oracle；ClickHouse；TDengine。各自仅修改 codec/映射/本驱动测试 | 每个子任务完成第 6 节映射和 SELECT+bind+export 矩阵；ClickHouse 无损获取 spike 未通过不得宣称完成；Oracle/TDengine 实机不可用时列为未验收 |
| T09 全消费者 / T07 | `copy_format.rs`、`import_export/`、`compare/data_paging.rs`、`plugin.rs::strip_hidden_result_columns` 剩余调用者、gateway/WIT adapters；枚举余下 legacy 入口 | copy/export/全局 dump restore/backup/compare 无显示反解析；MySQL/PG 强制跑完整 read-bind-export-restore 矩阵；旧格式 golden、隐藏列/重复标签/分页/多 statement 一致 |
| T10 协议扩展与外仓 / T03、T09 | 只为现有 wire 无法表达的语义扩展 lifecycle/row/query；更新外仓依赖和各语言 fixtures；同步 native/IPC DuckDB | 新旧 host/driver 四组合协议矩阵；协商前不发新 tag；旧能力不足明确失败；WIT 扩展单独版本化与测试 |
| T11 大值/运行产物 / T01 起建立预算，整合依赖 T08–T10 | 完成 LOB leases/spool/分页背压；诊断快照、sidecar resolver/升级失效；按第 7.2 节从实际安装包启动核验，不用开发目录替代 | 有界内存、取消回收、断线句柄行为明确；三平台包 hash/路径/version/features/依赖与实际连接记录；缺环境标为未验收；无默认数据/凭证日志 |
| T12 收缩/发布 / T08–T11 | 删除运行时 legacy rows/sidecar、过渡恢复及无消费者代码；保留必要历史 DTO importer；跨平台回归与分批发布 | 第 13 节全部硬门禁；搜索无新路径 display-to-data；真实库端到端和外仓均有证据；任何阻塞不标完成 |

### 10.1 并行与协调方式
- T00/T01/T02 由一个模型主导契约，不让多个模型各自发明 DbValue。T03、T04、T05 在 T02 接口稳定后可并行；T06/T07 由同一所有者整合垂直链路，减少 delegate/执行接口冲突。
- T08 可按上述五个子任务并行，公共模型变更必须先提交最小契约变更，经主协调者批准后统一适配；探子只做检索/验证，代码所有者执行修改。每项不得顺手扩展到扩展注册架构、Redis/Mongo 或全仓格式化。
- T09 的导出、比较、gateway 可分开实施；T10 与 T11 可部分并行，但只有资源能力和 wire 契约都稳定后才能做大值 IPC 集成验收。T12 必须串行收口，不凭一个 crate 编译通过批量删兼容代码。
- 推荐里程碑：M1=T00–T02（地基）；M2=T03–T07（MySQL/PG/IPC 端到端）；M3=T08–T10（全覆盖与跨仓）；M4=T11–T12（资源、产物、收缩）。不承诺单个模型回合完成；先完成 M2 再决定复杂类型与新导出格式的扩展深度。

## 11. 测试矩阵与实施时命令

| 测试层 | 必须覆盖 |
| --- | --- |
| 模型/property | SQL NULL、空 Text、空 Binary、文本 `NULL`/`0x4142`、0..255 bytes、合法 UTF-8 Blob、非法 UTF-8 Text、嵌入/尾 NUL、超 1024 字节预览；越界/短行/重复身份；clone 不复制整份 payload |
| 精确类型 | 超 JS 安全整数、i64/u64 边界、高精度 decimal/scale、负零/NaN/Infinity、时间小数精度/offset/合法特殊日期、数组维度下界与元素 NULL；JSON 大数与 JSONB 语义 |
| codec/binder | 每个支持类型 valid/null/error/unsupported 四分支；原始值不可得时显式状态；SELECT→edit→bind→SELECT；不能表示的参数禁止自动 string fallback |
| IPC/兼容 | 旧 fixtures、未知 tags、错误/过大 base64、frame 上限、版本未协商、进程退出/超时/取消；新旧 host/driver 四组合；Rust 与实际外部语言运行结果一致 |
| UI/consumer | sort/filter/page/隐藏列/rowid/重复标签/刷新 generation、编辑撤销/冲突；copy 与 export 区别；metadata 不走 formatter；typed compare/hash |
| 导出/恢复 | SQL/XML 既有 golden，JSON/CSV 默认限制，显式 encoding 往返，截断/失败输出不冒充成功备份；未知/错误值 fail closed |
| 真实库/性能 | 复用 `crates/db/tests/real_databases/`；MySQL/PG 强制 read→edit→bind→read、SQL/XML 往返与 JSON/CSV 限制矩阵；SQLite/DuckDB 两路必跑；其他驱动缺环境只能标未验收，不能以 fixture 替代“全部驱动完成”；记录混合行/LOB 峰值 RSS、首屏、取消释放 |

以下是**未来实施时运行的命令，不代表本文档编写时已运行或已通过**。先查看各测试 harness 的真实入口和环境变量；缺依赖/服务时报告跳过原因，不把 0 tests 当覆盖。所有 shell 命令遵循仓库 RTK 约定。
```sh
rtk proxy cargo test -p db-value                          # T01 新 crate 建立后
rtk proxy cargo test -p db -p extension-protocol
rtk proxy cargo test -p db_view
rtk proxy cargo test -p db --features builtin-duckdb
rtk proxy cargo test -p extension-host -p extension-driver
rtk proxy cargo test -p extension-runtime -p extension-api -p extension-component -p extension-wasm
rtk proxy cargo check -p db -p db_view -p extension-host -p extension-runtime -p main
rtk proxy cargo check -p db --features builtin-duckdb
rtk proxy cargo clippy -p db-value -p db -p db_view --all-targets -- -D warnings
rtk proxy cargo test --manifest-path ../navop-extensions/Cargo.toml -p duckdb_driver --test extension_protocol --test cancel_timeout
```
定向回归优先复用：`executor.rs` 的旧 JSON/typed_view 测试、`query_result_normalization.rs`、`import_export/formats/{sql_export_tests,xml_tests,table_import_tests}.rs`、`db_view/src/table_data/copy_format_tests.rs`、`db_view/src/sql_result_tab_tests.rs`、`extension-runtime/src/extension_db_gateway_tests.rs`。新增 native codec 单测应放小模块，真实服务测试按现有 harness 接入，不另造重复基础设施。
性能验收需在 T00 固定同机器/同数据集基线：大量小标量、混合 bytes、少量大 LOB 三类；记录峰值 RSS、首批时间、全量吞吐、滚动延迟、取消后存活资源。首版目标是消除完整 hex/base64 常驻副本并守住字节预算；百分比阈值由实际基线决定，未经测量不宣称“零拷贝”或“性能提高 N 倍”。

## 12. 兼容、灰度、回滚与风险

- 保持新宿主 + 旧驱动可连接并保留旧契约已有精度；旧驱动没提供的类型信息不能恢复。旧宿主 + 新驱动必须协商回旧 shape 或明确拒绝；新宿主 + 新驱动启用新能力。默认模式、测试包和 release 包分别核验，不能拿开发目录的新扩展覆盖用户安装路径来假装兼容。
- 发布顺序：内部 fixture/真实库 → MySQL/PG/IPC 垂直链路 → 其余驱动 → 跨仓新能力 → 清理。临时切换开关只能选完整链路，不能“新读旧写”；回退遇到旧链路无法无损处理的值应禁写/禁导出，而不是静默降级。
- 不在用户数据库做结构或数据迁移。查询结果缓存/历史序列化若存在应另列版本迁移与失效规则；旧预览中已丢失的数据无法从 hex 猜回，需重新查询。旧文件只通过经测试的 DTO importer 读入。
- 主要风险：接口影响广、其他任务同时改动、外仓锁定旧 revision、不同数据库值能力不对齐、LOB 与 session 生命周期、SDK 原始值获取受限、Oracle/TDengine 测试环境缺失。对每项风险记录 owner、复现/缓解措施与阻塞门禁；不能用未测试的“通用 fallback”掩盖。
- 资源/类型能力尚不足时，允许“准确显示不支持，保持只读”，不允许“看起来正常但改错数据”。仅可读成功不能通过写回/备份验收；平台构建没跑不能宣称 Windows/Linux/macOS 全通过。

## 13. 最终验收清单（实施者填写，当前均未完成）

- [ ] MySQL 元数据原生路径和真实运行包复现/核验完成，数据库名、字符集、collation 不再错误走 binary preview；保留原始故障条件与版本证据。
- [ ] 所有 SQL 驱动进入同一 typed 结果契约；runtime 不再以字符串 rows + binary sidecar 为唯一或双重权威；驱动缺失能力有明确说明。
- [ ] NULL / 空文本 / 空 bytes / 未解码 / 解码失败不混淆；没有新的 `.ok().flatten()` 吞掉数据库值解码错误。
- [ ] 精确数值、二进制、时间、支持的复合类型在读取、编辑、绑定、复制、比较、导出各边界满足各自等价关系；未知值绝不伪造。
- [ ] 排序、隐藏列、`__rowid__`、分页、刷新、重复标签不会串值；旧 generation 异步结果不回填，写回使用真实数据库键而非结果行序号。
- [ ] 现有 JSON/CSV 限制与 SQL/XML 二进制能力未回归；预览不能参与备份/恢复；错误/截断输出不冒充完整成功。
- [ ] 内置 MySQL/PG 等原生后端、DuckDB 两路、外部扩展实际 artifact 与协商能力均可追溯；没有仅改 IPC 却遗漏 native 的交付。
- [ ] 外仓依赖/锁文件与新旧协议矩阵验证完成；WIT/公共接口没有未版本化破坏；Redis/Mongo 与扩展注册分层未被误改。
- [ ] LOB/大帧预算、取消、断线、资源释放与诊断脱敏有测试；关键跨平台 release 启动已核验。
- [ ] 每项提交有当前测试证据；未覆盖环境明确列出；旧模型清理有消费者清单证明，不靠“grep 没找到一个名字”认定整个机制可删。

## 14. 给接手模型的执行约束

先读本文、根 AGENTS、`ipc-driver-development` 技能及涉及模块的架构约束。重新检查工作区：尤其 MySQL 元数据补丁不是本文作者的实现，不得 reset/revert。按 T00 开始，每次只选择一个有界任务，声明依赖、变更文件和验收；探子使用干净默认上下文做并行检索，最终方案取舍、修改及验证由主模型承担。
需要更改本文的契约时，先写出原决定、反证、替代方案及对已完成阶段的影响，再更新计划；不能因为局部实现方便退回字符串模型。所有行为/公共契约改动按 Red → Green → Refactor → Verify；没有真实数据库的测试用 codec fixture 先覆盖，但真实库门禁保持未完成。每次交接记录“完成任务、差异、命令/结果、未覆盖项、下一任务”；用户没有授权时不要自动部署、更新服务端数据库或写入其业务库。

## 15. 来源与核验说明

仓库基础约束：`crates/db/README.md`、`docs/superpowers/specs/2026-07-16-native-ipc-drivers-design.md`、根 `AGENTS.md`、`.codex/skills/ipc-driver-development/SKILL.md`、`.codex/skills/gpui-component/references/{coding-guides,design-guides}.md`；源码证据见第 2 节及任务路径。本文区分了静态代码事实、截图解释假设与拟定 API，未执行实现测试或连接用户数据库。
外部资料访问日期为 2026-09-08。搜索工具本次未返回可用正文，已通过 HTTP 直接读取下列可访问的一手文档/源码核对相关接口；`latest/current/devel/main` 链接会变化，实施时必须对照 Cargo.lock 中实际 SDK 版本，不能把当前文档当锁定版本保证。
- [S1] PostgreSQL Protocol Overview：`https://www.postgresql.org/docs/current/protocol-overview.html`。核验类型内容与传输表示的区别；不将 wire binary 等同 bytea。
- [S2] postgres-types FromSql：`https://docs.rs/postgres-types/latest/postgres_types/trait.FromSql.html`。核验 String/BYTEA、NULL、Result 与一维数组适配；复杂数组仍需独立实现/验证。
- [S3] DBeaver DBDValueHandler：`https://raw.githubusercontent.com/dbeaver/dbeaver/devel/plugins/org.jkiss.dbeaver.model/src/org/jkiss/dbeaver/model/data/DBDValueHandler.java`；DBDContent：`https://raw.githubusercontent.com/dbeaver/dbeaver/devel/plugins/org.jkiss.dbeaver.model/src/org/jkiss/dbeaver/model/data/DBDContent.java`。参考职责分离，不声称其所有 fallback 都符合本方案严格策略。
- [S4] SQLite Datatypes：`https://www.sqlite.org/datatype3.html`。核验 storage class、affinity 以及 BLOB 原样存储语义。
- [S5] Tiberius FromSql：`https://docs.rs/tiberius/latest/tiberius/trait.FromSql.html`。核验类型映射和 feature-sensitive 时间/数值能力。
- [S6] ClickHouse RowBinaryWithNamesAndTypes 文档入口：`https://clickhouse.com/docs/interfaces/formats/RowBinaryWithNamesAndTypes`。该入口本次返回 HTTP 403；实施时读取官方文档仓库对应格式定义并完成 SDK spike，**本方案不把未验证的解析细节作为既定事实**。
- [S7] Go MySQL driver `fields.go`：`https://raw.githubusercontent.com/go-sql-driver/mysql/master/fields.go`。核验 field type 与 binary collation 联合决定类型分类的实现；它不是对所有服务器兼容问题的通用补丁。
