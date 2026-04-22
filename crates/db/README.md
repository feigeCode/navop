# Database Abstraction Layer

这个 crate 提供了一个统一的数据库抽象层，支持多种数据库类型。

## 架构

- `src/` - 顶层接口和公共类型
  - `plugin.rs` - `DatabasePlugin` trait 与数据库能力入口
  - `plugin_manifest.rs` - 数据库 UI manifest 合同，承载表单、能力位、动作元数据
  - `manager.rs` - 数据库管理器
  - `connection.rs` - 连接接口和连接池
  - `executor.rs` - SQL 执行器
  - `runtime.rs` - Tokio 运行时
  - `types.rs` - 公共类型定义

- `src/mysql/` - MySQL 实现
- `src/postgresql/` - PostgreSQL 实现  
- `src/sqlite/` - SQLite 实现

`crates/db` 现在同时负责两层职责：

- 数据库运行时能力：连接、元数据、SQL 构建、导入导出
- 数据库 UI 合同：`ui_manifest()` 返回纯数据 manifest，供 `db_view` 做统一渲染

新增数据库插件时，除了实现 `DatabasePlugin` 的运行时方法，也应在插件侧返回
对应的 `DatabaseUiManifest`，并通过 `resolve_reference_data()` 提供动态下拉数据
（如 charset / collation / engines）。

## 使用示例

```rust
use db::{DbManager, DatabaseType, DbConnectionConfig};

let manager = DbManager::new();
let plugin = manager.get_plugin(&DatabaseType::MySQL)?;
let connection = plugin.create_connection(config).await?;
```
