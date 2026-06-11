# 数据库比较功能

提供数据库表之间的数据比较和结构比较功能，生成同步 SQL。

## 功能特性

### 数据比较
- 支持单列和复合键
- 识别新增、删除、修改的行
- 生成 INSERT、UPDATE、DELETE 语句
- DELETE 默认不选中（P0 安全保护）
- SQL 注入防护

### 结构比较
- 比较表、列、索引、外键
- 识别新增、删除、修改
- 生成 DDL 语句
- DROP TABLE/COLUMN 默认不选中（P0 安全保护）
- 列类型修改默认不选中

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

// 生成同步计划
let plan = build_schema_sync_plan(&result, "postgresql");
```

## 模块说明

- `data_model.rs` - 数据比较数据结构
- `data_diff.rs` - 数据比较算法
- `schema_model.rs` - 结构比较数据结构
- `schema_diff.rs` - 结构比较算法
- `sync_plan.rs` - 同步计划生成
- `capabilities.rs` - 数据库能力声明
- `task.rs` - 任务进度事件

## 安全保护

所有破坏性操作默认不选中：
- DELETE（数据）
- DROP TABLE（结构）
- DROP COLUMN（结构）
- ALTER COLUMN TYPE（结构）

## 测试

```bash
cargo test -p db --lib compare
```

15 个单元测试覆盖所有核心功能。
