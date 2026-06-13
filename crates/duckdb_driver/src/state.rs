//! v2 server 的每连接状态机。
//!
//! 每个 TCP / local socket 客户端都会得到一份 [`ConnectionState`],
//! 维护这个客户端打开的所有 conn(数据库会话)和所有 cursor(查询结果)。
//! conn / cursor id 在客户端内部唯一,跨客户端不共享(driver 进程整体可能跑
//! 多个客户端,但客户端之间互不可见对方的资源)。

use std::collections::HashMap;
use std::time::Instant;

use crate::duckdb_session::DuckDbSession;
use extension_protocol::conn::ConnId;
use extension_protocol::data::{CsvOptions, FailedRow, ImportId, ImportOptions, StreamId};
use extension_protocol::query::{CursorId, TxId};
use extension_protocol::row::{CellValue, ColumnSpec};

/// 单个游标的服务端状态。
pub struct CursorState {
    pub columns: Vec<ColumnSpec>,
    conn_id: ConnId,
    sql: String,
    params: Vec<CellValue>,
    next_row: u64,
    fetch_size: Option<u32>,
    max_rows: Option<u64>,
    done: bool,
}

impl CursorState {
    pub fn new(
        conn_id: ConnId,
        columns: Vec<ColumnSpec>,
        sql: String,
        params: Vec<CellValue>,
        fetch_size: Option<u32>,
        max_rows: Option<u64>,
    ) -> Self {
        Self {
            columns,
            conn_id,
            sql,
            params,
            next_row: 0,
            fetch_size,
            max_rows,
            done: max_rows == Some(0),
        }
    }

    pub fn conn_id(&self) -> ConnId {
        self.conn_id
    }

    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn params(&self) -> &[CellValue] {
        &self.params
    }

    pub fn offset(&self) -> u64 {
        self.next_row
    }

    pub fn fetch_size(&self) -> Option<u32> {
        self.fetch_size
    }

    pub fn remaining_max_rows(&self) -> Option<u64> {
        self.max_rows
            .map(|max_rows| max_rows.saturating_sub(self.next_row))
    }

    pub fn advance(&mut self, fetched: usize, requested: u32) {
        self.next_row = self.next_row.saturating_add(fetched as u64);
        if fetched < requested as usize {
            self.done = true;
        }
        if self.remaining_max_rows() == Some(0) {
            self.done = true;
        }
    }

    pub fn cancel(&mut self) {
        self.done = true;
    }

    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// 单个导出 stream 的服务端状态。
pub struct ExportStreamState {
    conn_id: ConnId,
    sql: String,
    columns: Vec<String>,
    format: ExportStreamFormat,
    next_row: u64,
    max_rows: Option<u64>,
    done: bool,
    pending: Vec<u8>,
}

pub enum ExportStreamFormat {
    Csv {
        options: CsvOptions,
        header_written: bool,
    },
    Ndjson,
}

/// 单个导入任务的服务端状态。
pub struct ImportState {
    conn_id: ConnId,
    table_ref: String,
    columns: Vec<String>,
    options: ImportOptions,
    inserted: u64,
    failed: Vec<FailedRow>,
    failed_count: u64,
    next_row: u64,
    started: Instant,
}

impl ImportState {
    pub fn new(
        conn_id: ConnId,
        table_ref: String,
        columns: Vec<String>,
        options: ImportOptions,
    ) -> Self {
        Self {
            conn_id,
            table_ref,
            columns,
            options,
            inserted: 0,
            failed: Vec::new(),
            failed_count: 0,
            next_row: 0,
            started: Instant::now(),
        }
    }

    pub fn conn_id(&self) -> ConnId {
        self.conn_id
    }

    pub fn table_ref(&self) -> &str {
        &self.table_ref
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn options(&self) -> &ImportOptions {
        &self.options
    }

    pub fn inserted(&self) -> u64 {
        self.inserted
    }

    pub fn failed(&self) -> &[FailedRow] {
        &self.failed
    }

    pub fn failed_count(&self) -> u64 {
        self.failed_count
    }

    pub fn next_row(&self) -> u64 {
        self.next_row
    }

    pub fn started(&self) -> Instant {
        self.started
    }

    pub fn record_inserted(&mut self) {
        self.inserted = self.inserted.saturating_add(1);
        self.next_row = self.next_row.saturating_add(1);
    }

    pub fn record_failed(&mut self, message: String, code: i32) -> Option<FailedRow> {
        let row_index = self.next_row;
        self.failed_count = self.failed_count.saturating_add(1);
        self.next_row = self.next_row.saturating_add(1);
        let failed = FailedRow {
            row_index,
            message,
            code,
        };
        if self.options.track_failed_rows {
            self.failed.push(failed.clone());
            Some(failed)
        } else {
            None
        }
    }
}

impl ExportStreamState {
    pub fn new(
        conn_id: ConnId,
        sql: String,
        columns: Vec<String>,
        format: ExportStreamFormat,
        max_rows: Option<u64>,
    ) -> Self {
        Self {
            conn_id,
            sql,
            columns,
            format,
            next_row: 0,
            max_rows,
            done: max_rows == Some(0),
            pending: Vec::new(),
        }
    }

    pub fn conn_id(&self) -> ConnId {
        self.conn_id
    }

    pub fn sql(&self) -> &str {
        &self.sql
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub fn format_mut(&mut self) -> &mut ExportStreamFormat {
        &mut self.format
    }

    pub fn offset(&self) -> u64 {
        self.next_row
    }

    pub fn remaining_max_rows(&self) -> Option<u64> {
        self.max_rows
            .map(|max_rows| max_rows.saturating_sub(self.next_row))
    }

    pub fn advance(&mut self, fetched: usize, requested: u32) {
        self.next_row = self.next_row.saturating_add(fetched as u64);
        if fetched < requested as usize {
            self.done = true;
        }
        if self.remaining_max_rows() == Some(0) {
            self.done = true;
        }
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    pub fn pending(&self) -> &[u8] {
        &self.pending
    }

    pub fn append_pending(&mut self, bytes: Vec<u8>) {
        self.pending.extend(bytes);
    }

    pub fn drain_pending(&mut self, max_bytes: usize) -> Vec<u8> {
        let take = max_bytes.min(self.pending.len());
        self.pending.drain(..take).collect()
    }
}

/// 整个客户端的状态。
pub struct ConnectionState {
    conns: HashMap<ConnId, DuckDbSession>,
    cursors: HashMap<CursorId, CursorState>,
    streams: HashMap<StreamId, ExportStreamState>,
    imports: HashMap<ImportId, ImportState>,
    txs: HashMap<TxId, ConnId>,
    active_tx_by_conn: HashMap<ConnId, TxId>,
    next_conn_id: ConnId,
    next_cursor_id: u64,
    next_import_id: u64,
    next_tx_id: u64,
}

impl Default for ConnectionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ConnectionState {
    pub fn new() -> Self {
        Self {
            conns: HashMap::new(),
            cursors: HashMap::new(),
            streams: HashMap::new(),
            imports: HashMap::new(),
            txs: HashMap::new(),
            active_tx_by_conn: HashMap::new(),
            next_conn_id: 1,
            next_cursor_id: 1,
            next_import_id: 1,
            next_tx_id: 1,
        }
    }

    pub fn open_conn(&mut self, session: DuckDbSession) -> ConnId {
        let id = self.next_conn_id;
        self.next_conn_id += 1;
        self.conns.insert(id, session);
        id
    }

    /// 用单个已建立的连接构造状态(driver runtime 模型:每个 worker 独占一个连接,
    /// conn_id 由 driver 进程统一分配并贯穿宿主侧)。游标随该连接独立计数与回收。
    pub fn with_conn(conn_id: ConnId, session: DuckDbSession) -> Self {
        let mut conns = HashMap::new();
        conns.insert(conn_id, session);
        Self {
            conns,
            cursors: HashMap::new(),
            streams: HashMap::new(),
            imports: HashMap::new(),
            txs: HashMap::new(),
            active_tx_by_conn: HashMap::new(),
            next_conn_id: conn_id.saturating_add(1),
            next_cursor_id: 1,
            next_import_id: 1,
            next_tx_id: 1,
        }
    }

    pub fn close_conn(&mut self, id: ConnId) -> bool {
        let removed = self.conns.remove(&id).is_some();
        if removed {
            self.streams.retain(|_, stream| stream.conn_id() != id);
            self.imports
                .retain(|_, import_state| import_state.conn_id() != id);
            if let Some(tx_id) = self.active_tx_by_conn.remove(&id) {
                self.txs.remove(&tx_id);
            }
        }
        removed
    }

    pub fn get_conn(&self, id: ConnId) -> Option<&DuckDbSession> {
        self.conns.get(&id)
    }

    pub fn get_conn_mut(&mut self, id: ConnId) -> Option<&mut DuckDbSession> {
        self.conns.get_mut(&id)
    }

    pub fn open_cursor(&mut self, cursor: CursorState) -> CursorId {
        let id = format!("c-{}", self.next_cursor_id);
        self.next_cursor_id += 1;
        self.cursors.insert(id.clone(), cursor);
        id
    }

    pub fn get_cursor_mut(&mut self, id: &str) -> Option<&mut CursorState> {
        self.cursors.get_mut(id)
    }

    pub fn remove_cursor(&mut self, id: &str) -> Option<CursorState> {
        self.cursors.remove(id)
    }

    pub fn insert_cursor(&mut self, id: CursorId, cursor: CursorState) {
        self.cursors.insert(id, cursor);
    }

    pub fn close_cursor(&mut self, id: &str) -> bool {
        self.cursors.remove(id).is_some()
    }

    pub fn open_stream(&mut self, id: StreamId, stream: ExportStreamState) {
        self.streams.insert(id, stream);
    }

    pub fn get_stream_mut(&mut self, id: &str) -> Option<&mut ExportStreamState> {
        self.streams.get_mut(id)
    }

    pub fn get_stream(&self, id: &str) -> Option<&ExportStreamState> {
        self.streams.get(id)
    }

    pub fn close_stream(&mut self, id: &str) -> bool {
        self.streams.remove(id).is_some()
    }

    pub fn open_import(&mut self, import_state: ImportState) -> ImportId {
        let id = format!("i-{}", self.next_import_id);
        self.next_import_id += 1;
        self.imports.insert(id.clone(), import_state);
        id
    }

    pub fn remove_import(&mut self, id: &str) -> Option<ImportState> {
        self.imports.remove(id)
    }

    pub fn insert_import(&mut self, id: ImportId, import_state: ImportState) {
        self.imports.insert(id, import_state);
    }

    pub fn begin_tx(&mut self, conn_id: ConnId) -> Option<TxId> {
        if self.active_tx_by_conn.contains_key(&conn_id) {
            return None;
        }
        let tx_id = format!("tx-{}", self.next_tx_id);
        self.next_tx_id += 1;
        self.txs.insert(tx_id.clone(), conn_id);
        self.active_tx_by_conn.insert(conn_id, tx_id.clone());
        Some(tx_id)
    }

    pub fn tx_conn(&self, tx_id: &str) -> Option<ConnId> {
        self.txs.get(tx_id).copied()
    }

    pub fn tx_matches_conn(&self, tx_id: &str, conn_id: ConnId) -> bool {
        self.tx_conn(tx_id) == Some(conn_id)
    }

    pub fn has_active_tx(&self, conn_id: ConnId) -> bool {
        self.active_tx_by_conn.contains_key(&conn_id)
    }

    pub fn close_tx(&mut self, tx_id: &str) -> Option<ConnId> {
        let conn_id = self.txs.remove(tx_id)?;
        if self.active_tx_by_conn.get(&conn_id).map(String::as_str) == Some(tx_id) {
            self.active_tx_by_conn.remove(&conn_id);
        }
        Some(conn_id)
    }

    pub fn conn_count(&self) -> usize {
        self.conns.len()
    }

    pub fn cursor_count(&self) -> usize {
        self.cursors.len()
    }

    pub fn stream_count(&self) -> usize {
        self.streams.len()
    }

    pub fn import_count(&self) -> usize {
        self.imports.len()
    }

    pub fn tx_count(&self) -> usize {
        self.txs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_protocol::row::ColumnTypeKind;

    const CONN_ID: ConnId = 17;

    fn make_cols() -> Vec<ColumnSpec> {
        vec![ColumnSpec::new("x", "INT", ColumnTypeKind::I64)]
    }

    fn make_cursor() -> CursorState {
        CursorState::new(
            CONN_ID,
            make_cols(),
            "SELECT 1".into(),
            Vec::new(),
            None,
            None,
        )
    }

    #[test]
    fn cursor_state_tracks_fetch_offset() {
        let mut c = make_cursor();
        assert_eq!(c.offset(), 0);

        c.advance(4, 4);
        assert_eq!(c.offset(), 4);
        assert!(!c.is_done());

        c.advance(0, 4);
        assert!(c.is_done());
    }

    #[test]
    fn cursor_state_honors_max_rows() {
        let mut c = CursorState::new(
            CONN_ID,
            make_cols(),
            "SELECT 1".into(),
            Vec::new(),
            None,
            Some(3),
        );
        assert_eq!(c.remaining_max_rows(), Some(3));

        c.advance(2, 2);
        assert_eq!(c.remaining_max_rows(), Some(1));
        assert!(!c.is_done());

        c.advance(1, 1);
        assert!(c.is_done());
    }

    #[test]
    fn cursor_state_cancel_marks_done() {
        let mut c = make_cursor();
        c.cancel();
        assert!(c.is_done());
    }

    #[test]
    fn connection_state_assigns_unique_conn_ids() {
        let mut s = ConnectionState::new();
        let a = s.open_conn(DuckDbSession::new());
        let b = s.open_conn(DuckDbSession::new());
        let c = s.open_conn(DuckDbSession::new());
        assert_eq!(a, 1);
        assert_eq!(b, 2);
        assert_eq!(c, 3);
        assert_eq!(s.conn_count(), 3);
    }

    #[test]
    fn close_conn_returns_true_on_existing() {
        let mut s = ConnectionState::new();
        let id = s.open_conn(DuckDbSession::new());
        assert!(s.close_conn(id));
        assert!(!s.close_conn(id));
        assert_eq!(s.conn_count(), 0);
    }

    #[test]
    fn cursor_ids_are_prefixed_and_unique() {
        let mut s = ConnectionState::new();
        let id1 = s.open_cursor(make_cursor());
        let id2 = s.open_cursor(make_cursor());
        assert!(id1.starts_with("c-"));
        assert!(id2.starts_with("c-"));
        assert_ne!(id1, id2);
        assert_eq!(s.cursor_count(), 2);
    }

    #[test]
    fn close_cursor_removes_state() {
        let mut s = ConnectionState::new();
        let id = s.open_cursor(make_cursor());
        assert!(s.close_cursor(&id));
        assert!(!s.close_cursor(&id));
        assert!(s.get_cursor_mut(&id).is_none());
    }

    #[test]
    fn stream_state_drains_pending_by_limit() {
        let mut stream = ExportStreamState::new(
            CONN_ID,
            "SELECT 1".into(),
            vec!["x".into()],
            ExportStreamFormat::Ndjson,
            None,
        );

        stream.append_pending(b"abcdef".to_vec());

        assert_eq!(stream.drain_pending(2), b"ab");
        assert_eq!(stream.drain_pending(10), b"cdef");
        assert!(stream.pending().is_empty());
    }

    #[test]
    fn close_conn_removes_streams_for_connection() {
        let mut s = ConnectionState::new();
        let conn_id = s.open_conn(DuckDbSession::new());
        s.open_stream(
            "s-1".into(),
            ExportStreamState::new(
                conn_id,
                "SELECT 1".into(),
                vec!["x".into()],
                ExportStreamFormat::Ndjson,
                None,
            ),
        );

        assert_eq!(s.stream_count(), 1);
        assert!(s.close_conn(conn_id));
        assert_eq!(s.stream_count(), 0);
    }

    #[test]
    fn close_conn_removes_imports_for_connection() {
        let mut s = ConnectionState::new();
        let conn_id = s.open_conn(DuckDbSession::new());
        s.open_import(ImportState::new(
            conn_id,
            "\"target\"".into(),
            vec!["id".into()],
            ImportOptions::default(),
        ));

        assert_eq!(s.import_count(), 1);
        assert!(s.close_conn(conn_id));
        assert_eq!(s.import_count(), 0);
    }

    #[test]
    fn get_conn_mut_returns_session() {
        let mut s = ConnectionState::new();
        let id = s.open_conn(DuckDbSession::new());
        assert!(s.get_conn_mut(id).is_some());
        assert!(s.get_conn_mut(999).is_none());
    }

    #[test]
    fn tx_state_allows_one_active_tx_per_conn() {
        let mut s = ConnectionState::new();
        let conn_id = s.open_conn(DuckDbSession::new());

        let tx_id = s.begin_tx(conn_id).unwrap();

        assert!(s.tx_matches_conn(&tx_id, conn_id));
        assert!(s.begin_tx(conn_id).is_none());
        assert_eq!(s.tx_count(), 1);
    }

    #[test]
    fn close_tx_and_conn_cleanup_tx_state() {
        let mut s = ConnectionState::new();
        let conn_id = s.open_conn(DuckDbSession::new());
        let tx_id = s.begin_tx(conn_id).unwrap();

        assert_eq!(s.close_tx(&tx_id), Some(conn_id));
        assert_eq!(s.tx_count(), 0);
        assert!(s.begin_tx(conn_id).is_some());

        assert!(s.close_conn(conn_id));
        assert_eq!(s.tx_count(), 0);
    }
}
