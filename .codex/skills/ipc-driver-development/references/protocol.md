# IPC Database Protocol Reference

## Table of Contents

- [Wire Model](#wire-model)
- [Method Families](#method-families)
- [Routing Pattern](#routing-pattern)
- [Schema Metadata](#schema-metadata)
- [DDL Builders](#ddl-builders)
- [Errors and Fallbacks](#errors-and-fallbacks)

## Wire Model

onetcli IPC database drivers speak JSON-RPC-like request/response messages over the host-managed local transport declared in `driver.json`.

Authoritative Rust sources in the host repo:

- `crates/extension-protocol/src/method.rs`
- `crates/extension-protocol/src/conn.rs`
- `crates/extension-protocol/src/schema.rs`
- `crates/extension-protocol/src/query.rs`
- `crates/extension-protocol/src/ddl.rs`
- `crates/extension-driver/src/runtime.rs`

Rust drivers should use `extension_protocol::method` constants instead of literal method strings. Non-Rust drivers should mirror the same strings exactly and keep a test that validates manifest method names.

## Method Families

| Family | Methods | Notes |
| --- | --- | --- |
| Protocol meta | `$/ping`, `$/cancelRequest`, `shutdown` | Process and request lifecycle. |
| Connection | `conn/test`, `conn/open`, `conn/close`, `conn/ping`, `conn/use` | `conn/open` returns `conn_id`; connection-scoped methods receive it. |
| Query/cursor | `query/start`, `cursor/fetch`, `cursor/cancel`, `cursor/close` | Use server-side or emulated cursors. Preserve cancellation behavior. |
| Exec | `exec/run`, `exec/batch` | Non-query SQL execution and batch scripts. |
| Transaction | `tx/begin`, `tx/commit`, `tx/rollback`, optional savepoint methods | Return typed unsupported errors if unavailable. |
| Schema | `schema/databases`, `schema/schemas`, `schema/objects`, `schema/columns`, `schema/views`, `schema/indexes`, `schema/checks`, etc. | Used by tree, details, completion, and data grid. |
| DDL | `ddl/build`, `ddl/build_create_table`, `ddl/build_alter_table`, `ddl/build_drop` | SQL generation only. |
| Data pipe | `data/export`, `data/import_begin`, `data/import_chunk`, `data/import_commit`, `data/import_abort`, `stream/read`, `stream/close` | Used for larger transfer workflows. |
| Host API | `host/*` | Extension-to-host calls when supported by runtime. |

## Routing Pattern

Use two routing layers:

1. Driver/control plane:
   - `init`
   - `conn/test`
   - `conn/open`
   - pure `ddl/build*` when no live database state is required
2. Opened connection:
   - `conn/ping`, `conn/use`
   - query/cursor/exec/tx
   - schema metadata
   - import/export/stream
   - `ddl/build*` as an accepted duplicate path when host injects `conn_id`

For Rust drivers using `extension-driver`, implement:

```rust
impl Driver for MyDriver {
    fn call_connless(&self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::CONN_TEST => handle_conn_test(params),
            method::DDL_BUILD => handle_ddl_build(params),
            other => Err(method_not_found(other)),
        }
    }
}

impl DriverConnection for MyConnection {
    fn call(&mut self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::SCHEMA_OBJECTS => handle_schema_objects(&mut self.state, params),
            method::QUERY_START => handle_query_start(&mut self.state, params),
            method::DDL_BUILD => handle_ddl_build(params),
            other => Err(method_not_found(other)),
        }
    }
}
```

Non-Rust runtimes should implement the same split in their JSON-RPC dispatcher.

## Schema Metadata

Return structured objects matching `extension-protocol/src/schema.rs`.

Important params:

- `schema/databases`: `{ "conn_id": number }`
- `schema/schemas`: `{ "conn_id": number, "database": string }`
- `schema/objects`: `{ "conn_id": number, "database"?: string, "schema"?: string, "kinds": [...] }`
- `schema/object_view`: `{ "conn_id": number, "view": "databases" | "schemas" | "tables" | "columns" | "indexes" | "views" | "functions" | "procedures" | "triggers" | "sequences", "database"?: string, "schema"?: string, "table"?: string }`
- `schema/columns`: `{ "conn_id": number, "database"?: string, "schema"?: string, "table": string }`

Important result fields:

- Database: `name`, optional `charset`, `collation`, `owner`, `size_bytes`, `comment`, `extra`
- Schema: `name`, optional `owner`, `comment`, `extra`
- Object: `name`, `kind`, optional `schema`, `row_count_estimate`, `size_bytes`, timestamps, `comment`, `extra`
- Column: `ordinal`, `name`, `type`, `raw_type`, `nullable`, `default`, primary/unique flags, numeric/string sizing, `comment`, `extra`
- Object view: `{ "title"?: string, "columns": [{ "key": string, "name": string, "width_px"?: number, "align"?: "left" | "center" | "right" }], "rows": string[][] }`

`schema/object_view` is connection-bound and customizes the object-list table before the host falls back to fixed legacy mappings. Declare it only when routed. If the method is absent or returns typed not-supported/method-not-found for a view, the host uses legacy `schema/databases`, `schema/objects`, `schema/columns`, `schema/indexes`, etc. Keep the first column as the object name when rows are clickable database objects.

Catalog rules:

- Query the database's catalog metadata as the source of truth.
- If a database exposes `current_database()` or equivalent, use it for the current/default catalog returned by `schema/databases`.
- If the host sends legacy/default aliases such as `main`, treat them as equivalent filters only when the backend semantics justify it.
- Include `information_schema`, `pg_catalog`, and other system schemas when they are real visible schemas and the request asks for schema/object listings.
- Always qualify data-grid and metadata SQL with the returned catalog/schema names. A bug pattern is returning `character_sets` but later querying `"default_catalog"."character_sets"` instead of `information_schema.character_sets`.

### Table / Column Comment Support

表注释回显、列注释回显、以及设计器"改注释生成变更语句"依赖三条契约，缺一不可：

1. `schema/objects` 必须返回每个对象的 `schema` 和 `comment`。宿主打开表设计器时用 `(schema, name)` 匹配已加载的表信息来取回注释；驱动不返回 `schema` 时宿主只能退化为按表名匹配——同名表跨 schema 时注释会错位或丢失（表现为"表注释 input 不回显"，但列注释正常，因为列走 `schema/columns`）。
2. `schema/columns` 必须返回每列的 `comment`。`ColumnInfo.comment` 缺失时设计器列注释不回显。
3. DDL builder 必须生成 `COMMENT ON TABLE` / `COMMENT ON COLUMN` 语句，且要覆盖：
   - **创建与修改都处理**：`ddl/build_create_table` 对非空注释追加 COMMENT 语句；`ddl/build_alter_table` 对比 `from_spec`/`to_spec` 的注释差异（表注释 + 每列注释），否则"只改了注释"会得到 `-- No changes detected`。
   - **新增列也要处理**：`to_spec` 中新增的列（`from_spec` 里不存在）带非空注释时，必须在 `ALTER TABLE ... ADD` 之后追加对应的 `COMMENT ON COLUMN`。只给已存在列做注释 diff（`if (fromColumn == null) continue;`）会导致"新增列+注释"在 SQL 预览里没有注释语句。回滚时 DROP 列即可清除注释，无需单独的注释 rollback。
   - **回滚**：`alter_table` 的 `rollback_statements` 必须包含还原原注释的 COMMENT 语句。
   - **清空**：空注释要生成 `IS ''` 来清除旧注释，不能直接跳过。
   - **转义**：单引号要转义（SQL 标准 `'` -> `''`）。

协议层注意：给 `ObjectInfo` 增加 `schema` 字段时（`#[serde(default, skip_serializing_if = "String::is_empty")]`），Rust 驱动里显式构造 `ObjectInfo { ... }` 的结构体字面量必须同步补上 `schema` 字段，否则一旦协议更新所有 Rust 驱动编译失败；Go/Java 等按 wire JSON 输出的驱动不受影响（serde default 兜底反序列化）。

### 全量驱动检查清单 / Every-Driver Audit

注释能力的修复永远不要只落在某一个驱动或者宿主层。宿主只负责消费驱动返回的 `schema`/`comment` 与 DDL 语句；**所有**声明了关系表 schema 的 IPC 驱动都要各自实现三条契约。排查问题时的检查顺序：

1. `driver.json` 是否声明了 `schema/objects`、`schema/columns`、`ddl/build_create_table`、`ddl/build_alter_table`；没有关系表的存储（Redis、MongoDB 等）正确地**不声明**这些方法。
2. `schema/objects` 是否返回 `schema` + `comment`。
3. `schema/columns` 是否返回 `comment`。
4. `ddl/build_create_table` 是否对非空注释追加 COMMENT。
5. `ddl/build_alter_table` 是否 diff 表注释+列注释（含新增列）、生成 rollback、`IS ''` 清空、单引号转义。
6. host 兼容层（Go `internal/dbipc/server.go` 的 `stringCell(cols, 3)` / `stringCell(cols, 5)`、`SupportsComments` 开关）只做「驱动没投影时退化为空」的兜底，不能替代驱动自身实现。

历史排查结论（issue #3）：

| 驱动 | 语言/载体 | schema/objects | schema/columns | DDL 注释 | 备注 |
| --- | --- | --- | --- | --- | --- |
| duckdb | Rust | ✅ | ✅ | ✅ | 含 create/alter/rollback |
| opengauss | Rust | ✅ | ✅ | ✅ | 含 create/alter/rollback |
| redis | Rust | N/A | N/A | N/A | 无关系表，不声明 |
| mongodb-* | Rust | N/A | N/A | N/A | 无关系表，不声明 |
| dm | Go 宿主 | ✅ | ✅ | ✅ | `ALL_TAB_COMMENTS`/`ALL_COL_COMMENTS` |
| kingbase | Go 宿主 | ✅ | ✅ | ✅ | 支持 sys_*/pg_* 目录探测 |
| oracle | Go 宿主 | ✅ | ✅ | ✅ | `ALL_TAB_COMMENTS`/`ALL_COL_COMMENTS` |
| oceanbase | Go 宿主 | ✅ | ✅ | ✅ | MySQL 模式 `INFORMATION_SCHEMA` |
| iotdb | Go IPC | ✅ | ✅ | N/A | 无 ALTER/COMMENT，schema/comment 字段已补齐 |
| gbase8s | Java IPC | ✅ | ✅ | ✅ | 含新增列注释 |
| oscar | Java IPC | ✅ | ✅ | ✅ | 含新增列注释 |

## 宿主集成注意（issue #3 教训）

- 不要为单个驱动在宿主里硬编码注释行为：宿主拿到 `schema/objects`/`schema/columns` 的空缺字段时只做兼容兜底，注释生成必须走各驱动的 `ddl/build_*`，否则换一个数据库又坏一次。
- 元数据 SQL 只引用查询上下文里真实存在的目录列。引用一个没有出现在 FROM/JOIN 目标里的列（例如查询没 join 到含 `tabid` 的目录表就引用 `tabid`）会报 `字段 (tabid) 不在查询的任何表中`，要把列限定到已经 join 的目录别名上。
- 同一份 builder 既要服务 SQL 预览也要服务保存执行，不能存在「预览不生成、保存偷偷执行」的第二条路径；预览/保存不一致说明 alter 的注释 diff 有缺口（见上方新增列条目）。
- 宿主对 schema/ddl 的调用要走异步 IPC 请求，避免阻塞 UI 线程；driver 侧保持 DDL builder 为纯函数（不执行 SQL），需要 live state 的元数据查询走连接内异步路径。

## DDL Builders

DDL builders translate declarative specs into SQL strings. They must not execute SQL.

Core params and results:

- `ddl/build`: `{ "conn_id"?: number, "op": "create_table" | "...", "payload": object }` -> `{ "statements": string[], "warnings": string[] }`
- `ddl/build_create_table`: `{ "conn_id"?: number, "spec": TableSpec, "options": CreateTableOptions }` -> `{ "sql": string, "statements": string[] }`
- `ddl/build_alter_table`: `{ "conn_id"?: number, "from_spec": TableSpec, "to_spec": TableSpec, "column_renames": [], "options": AlterTableOptions }` -> `{ "statements": string[], "rollback_statements": string[], "warnings": string[] }`
- `ddl/build_drop`: `{ "kind": ObjectKind, "name": string, "schema"?: string, "database"?: string, "if_exists": bool, "cascade": bool }` -> `{ "sql": string }`

`TableSpec` includes `name`, optional `schema` and `database`, `columns`, `primary_key`, `indexes`, `foreign_keys`, `comment`, and driver-specific `options`.

Keep generic `ddl/build` in sync with specialized methods when both are declared. It is acceptable to implement specialized methods first and route generic ops to the same builder.

## Errors and Fallbacks

Use typed protocol errors:

- Invalid params: malformed JSON or unsupported config shape.
- Connection errors: database connection failed.
- Method not found / not supported: method intentionally unavailable.
- Query errors: SQL/backend execution failure.

Host fallback behavior depends on not-supported errors. If a driver does not implement a method and `driver.json` declares a compatible built-in database type, the host may use built-in SQL builders for DDL generation.

Do not hide unsupported behavior behind successful empty responses unless the protocol explicitly treats empty as meaningful.
