//! Auto 模式粘性会话真机验证（issue #312）。
//!
//! 复刻编辑器管线的会话级调用序列：`create_session_direct` → 多次独立
//! `execute_session` → `close_session_direct`，验证 MySQL `SET @var` 跨执行
//! 存活且物理连接（CONNECTION_ID）不变。这是 db_view 粘性会话所依赖的
//! 底层契约：UI 层的 `run_auto_sql_text` 逐条复用同一 session_id 调用
//! `execute_session`，变量语义由此层保证。

use db::executor::{ExecOptions, QueryResult, SqlResult};
use db::manager::GlobalDbState;

use crate::real_databases::common::env::{mysql_config, skip_database};

#[tokio::test]
async fn mysql_real_sticky_session_keeps_user_variables_across_executions() {
    let Some(config) = mysql_config() else {
        skip_database(
            "MySQL sticky session",
            "ONETCLI_TEST_MYSQL_PASSWORD (and optionally host/port/user)",
        );
        return;
    };

    let mut state = GlobalDbState::new();
    state.register_connection(config);
    let connection_id = "navop-real-mysql".to_string();

    // 1. 建会话（等价编辑器首次执行的懒建粘性会话）
    let session_id = state
        .create_session_direct(connection_id.clone(), None)
        .await
        .expect("sticky session should connect");

    // 2. issue #312 原始场景：SET 语句与 SELECT 分开执行
    let set_results = state
        .execute_session(
            session_id.clone(),
            "SET @startdates = '2026-1-1', @enddates = '2026-2-1'".to_string(),
            None,
        )
        .await
        .expect("SET @var should execute");
    assert_no_errors(&set_results);

    // 3. 单独执行 SELECT：变量必须存活（修复前 per-execution session 会丢）
    let select = session_query(&state, &session_id, "SELECT @startdates, @enddates").await;
    assert_cell(&select, 0, 0, "2026-1-1");
    assert_cell(&select, 0, 1, "2026-2-1");

    // 4. 多次跨执行仍存活，且物理连接不变（粘性 = 同一条连接）
    let identity_before = session_query(
        &state,
        &session_id,
        "SELECT CAST(CONNECTION_ID() AS CHAR)",
    )
    .await;
    let again = session_query(&state, &session_id, "SELECT @startdates").await;
    assert_cell(&again, 0, 0, "2026-1-1");
    let identity_after = session_query(
        &state,
        &session_id,
        "SELECT CAST(CONNECTION_ID() AS CHAR)",
    )
    .await;
    let (before, after) = (
        cell_text(&identity_before, 0, 0),
        cell_text(&identity_after, 0, 0),
    );
    assert_eq!(
        before, after,
        "sticky session must reuse the same physical connection"
    );

    // 5. issue 中的原始查询形态：变量参与 WHERE BETWEEN 过滤
    //    （子查询首行必须是 DATE，首行 int 会把整列钉成 int 类型导致比较失真）
    let between = state
        .execute_session(
            session_id.clone(),
            "SELECT COUNT(1) FROM (SELECT DATE'2026-01-10' AS create_date UNION SELECT DATE'2026-01-15') t \
             WHERE create_date BETWEEN @startdates AND @enddates"
                .to_string(),
            Some(ExecOptions::default()),
        )
        .await
        .expect("BETWEEN query should execute");
    assert_no_errors(&between);
    let SqlResult::Query(count) = &between[0] else {
        panic!("BETWEEN query should return rows");
    };
    // 2026-01-10、2026-01-15 都落在 [2026-1-1, 2026-2-1] 内，两条都命中。
    assert_cell(count, 0, 0, "2");

    // 6. 会话关闭后再开新会话：变量不跨会话（语义等同重连）
    state
        .close_session_direct(&session_id)
        .await
        .expect("sticky session should close");
    let fresh_session = state
        .create_session_direct(connection_id, None)
        .await
        .expect("fresh session should connect");
    let fresh = session_query(&state, &fresh_session, "SELECT @startdates").await;
    assert_null(&fresh, 0, 0);
    state
        .close_session_direct(&fresh_session)
        .await
        .expect("fresh session should close");
}

fn assert_no_errors(results: &[SqlResult]) {
    for result in results {
        if let SqlResult::Error(error) = result {
            panic!("session execution failed: {}", error.message);
        }
    }
}

async fn session_query(state: &GlobalDbState, session_id: &str, sql: &str) -> QueryResult {
    let results = state
        .execute_session(session_id.to_string(), sql.to_string(), None)
        .await
        .expect("session query should execute");
    assert_no_errors(&results);
    let SqlResult::Query(query) = results.into_iter().next().unwrap_or_else(|| {
        panic!("session query should return a result: {sql}");
    }) else {
        panic!("session query should return rows: {sql}");
    };
    query
}

fn cell_text(result: &QueryResult, row: usize, column: usize) -> String {
    let view = result.typed_view().expect("query result should be valid");
    match view.cell(row, column).expect("cell should exist") {
        db::executor::QueryCellRef::Text(value) => value.to_string(),
        other => panic!("expected text at ({row},{column}), got {other:?}"),
    }
}

fn assert_cell(result: &QueryResult, row: usize, column: usize, expected: &str) {
    assert_eq!(cell_text(result, row, column), expected);
}

fn assert_null(result: &QueryResult, row: usize, column: usize) {
    let view = result.typed_view().expect("query result should be valid");
    assert!(matches!(
        view.cell(row, column),
        Some(db::executor::QueryCellRef::Null)
    ));
}
