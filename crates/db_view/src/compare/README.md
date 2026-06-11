# 数据库比较 UI 模块

数据库比较功能的用户界面层。

## 模块结构

- `data_compare_dialog.rs` - 数据比较对话框
- `schema_compare_dialog.rs` - 结构比较对话框
- `sync_plan_view.rs` - 同步计划预览
- `executor.rs` - 任务执行器

## 使用示例

```rust
use db_view::compare::*;

// 创建数据比较对话框
let mut dialog = DataCompareDialog::new();

// 设置比较结果
dialog.set_result(result);

// 生成同步计划
let plan = generate_data_sync_plan(dialog.result().unwrap());
dialog.set_sync_plan(plan);

// 显示同步计划
let view = SyncPlanView::new(plan);
println!("{}", view.summary_text());
println!("{}", view.sql_text());
```

## 任务执行

```rust
// 数据比较任务
let params = DataCompareParams {
    source_connection_id: "conn1".to_string(),
    source_database: "db1".to_string(),
    source_table: "users".to_string(),
    target_connection_id: "conn2".to_string(),
    target_database: "db2".to_string(),
    target_table: "users".to_string(),
    key_columns: vec!["id".to_string()],
    // ...
};

let result = execute_data_compare(params, db_state, cx).await?;
```

## 后续开发

- [ ] 完整的 GPUI 渲染组件
- [ ] 连接选择器集成
- [ ] SQL 预览和执行
- [ ] 进度显示和任务取消
