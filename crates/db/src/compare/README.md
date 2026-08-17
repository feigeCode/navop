# 数据库比较功能

提供数据库表之间的数据比较和结构比较功能，生成同步 SQL。

## 功能特性

### 数据比较
- 支持单列和复合键
- 识别新增、删除、修改的行
- 生成 INSERT、UPDATE、DELETE 语句
- DELETE 默认不选中（P0 安全保护）
- 目标表缺失时自动前置 CREATE TABLE 语句
- 按列类型和目标方言格式化 SQL 字面量（MySQL BIT/时间、数组、NULL、SQL Server N 前缀）
- UPDATE SET 子句按比较列顺序稳定输出
- SQL 注入防护

### 结构比较
- 比较表、列、索引、外键
- 识别新增、删除、修改
- 跨数据库比较时按字段类型语义判断等价性，避免 `INT`/`INTEGER` 等别名产生无效差异
- 生成 DDL 语句
- 跨数据库同步时把源字段类型映射为目标数据库合法类型
- DROP TABLE/COLUMN 默认不选中（P0 安全保护）
- 列类型修改默认不选中
- 有损类型映射生成合法 SQL，但默认不选中并附带警告
- 不支持的类型映射跳过相关 SQL，并写入同步计划警告
- 方言分支按 `DatabaseType` 枚举归一化（外部驱动按 `driver_id` 归类）

## 快速开始

```rust
use db::compare::*;

// 数据比较
let result = compare_data_rows(
    source_rows,
    target_rows,
    vec!["id".to_string()],
    "users",
    "users_backup",
    DataCompareOptions::default(),
)?;

// 生成同步计划
let plan = build_data_sync_plan(&result);

// 结构比较
let result = compare_schemas(
    source_tables,
    target_tables,
    SchemaCompareOptions::default(),
)?;

// 生成同步计划（使用目标数据库插件）
let plan = build_schema_sync_plan_with_plugin(&result, "target_db", Some("public"), plugin);

// 跨数据库比较：显式传入源端和目标端数据库类型
let result = compare_schemas_with_type_mapping(
    source_tables,
    target_tables,
    SchemaCompareOptions::default(),
    Some(SchemaTypeMappingContext::new(
        &source_database_type,
        &target_database_type,
    )),
)?;

// 跨数据库同步：源类型用于映射，目标插件决定 DDL 方言
let plan = build_schema_sync_plan_with_plugin_for_source(
    &result,
    "target_db",
    Some("public"),
    &source_database_type,
    plugin,
);
```

## 模块说明

- `data_model.rs` - 数据比较数据结构
- `data_diff.rs` - 数据比较算法
- `schema_model.rs` - 结构比较数据结构
- `schema_diff.rs` - 结构比较算法
- `type_mapping.rs` - 跨数据库字段类型归一化、等价判断和目标类型映射
- `sync_plan.rs` - 同步计划生成
- `capabilities.rs` - 数据库能力声明
- `task.rs` - 任务进度事件

## 安全保护

所有破坏性操作默认不选中：
- DELETE（数据）
- DROP TABLE（结构）
- DROP COLUMN（结构）
- ALTER COLUMN TYPE（结构）

跨数据库类型映射按兼容性处理：
- `Exact` / `Equivalent` / `Widening`：允许默认选中
- `Lossy`：生成目标数据库合法 SQL，但默认不选中并提示可能丢失的语义
- `Unsupported`：不输出源类型到目标 DDL，跳过相关 SQL 并记录计划警告

## 测试

```bash
cargo test -p db --lib compare
```

单元测试覆盖核心比较、字段类型映射和同步计划安全策略。
