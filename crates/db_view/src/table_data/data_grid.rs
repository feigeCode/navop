use gpui::prelude::*;
use gpui::{
    Anchor, AnyElement, App, AsyncApp, ClickEvent, Context, Entity, EventEmitter, FocusHandle,
    Focusable, Image, ImageFormat, IntoElement, ObjectFit, ParentElement, PathPromptOptions,
    SharedString, Styled, Subscription, Window, actions, div, img, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, Size, WindowExt,
    button::Button,
    h_flex,
    input::{Input, InputEvent, InputState},
    notification::Notification,
    v_flex,
};
use one_ui::edit_table::{Column, EditTable, EditTableEvent, EditTableState};
use one_ui::{create_large_text_editor_with_content, large_text_values_equivalent};
use rust_i18n::t;
use rust_xlsxwriter::Workbook;
use tracing::{error, log::trace};

use crate::import_export::table_export_view::DataExportView;
use crate::search_shortcut::{
    DB_SEARCH_CONTEXT, FocusSearchInput, OpenSelectedTableQuery, OpenTableDesigner,
    focus_search_input,
};
use crate::sidebar::execution_history::ExecutionContext;
use crate::sidebar::execution_history_panel::ExecutionHistoryPanel;
use crate::sql_editor::SqlEditor;
use crate::table_data::copy_format::{CopyFormat, CopyFormatContext, CopyFormatter, TableMetadata};
use crate::table_data::filter_editor::{FilterEditorEvent, TableFilterEditor, TableSchema};
use crate::table_data::results_delegate::{EditorTableDelegate, RowChange};
use chrono::Local;
use db::{
    BinaryCell, ColumnInfo, DatabasePlugin, DbManager, ExecOptions, GlobalDbState, IndexInfo,
    QueryColumnMeta, QueryResult, SqlFormatOptions, SqlResult, TableCellChange, TableCellValue,
    TableDataRequest, TableRowChange, TableSaveRequest, binary_value::format_binary_input,
};
use gpui_component::button::ButtonVariants;
use gpui_component::dialog::DialogButtonProps;
use gpui_component::menu::{DropdownMenu, PopupMenuItem};
use one_core::gpui_tokio::Tokio;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use one_core::settings::{AppSettings, LargeTextCellEditorOpenMode};
use one_core::storage::DatabaseType;
use one_core::tab_container::TabContainer;
use one_ui::edit_table::ColumnSort;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

actions!(
    data_grid,
    [Page500, Page1000, Page2000, Page10000, Page100000]
);

fn build_header_order_by_clause(
    db_manager: &DbManager,
    database_type: DatabaseType,
    column_name: &str,
    sort: ColumnSort,
) -> Result<Option<String>, String> {
    let direction = match sort {
        ColumnSort::Ascending => "ASC",
        ColumnSort::Descending => "DESC",
        ColumnSort::Default => return Ok(None),
    };

    let plugin = db_manager
        .get_plugin(&database_type)
        .map_err(|_| t!("TableDataGrid.plugin_unavailable").to_string())?;

    Ok(Some(format!(
        "{} {}",
        plugin.quote_identifier(column_name),
        direction
    )))
}

fn collect_delete_row_indices<I>(selected_rows: I, fallback_row: Option<usize>) -> Vec<usize>
where
    I: IntoIterator<Item = usize>,
{
    let mut row_indices: Vec<usize> = selected_rows.into_iter().collect();

    if row_indices.is_empty() {
        if let Some(row_ix) = fallback_row {
            row_indices.push(row_ix);
        }
    } else {
        row_indices.sort_unstable();
        row_indices.dedup();
    }

    row_indices.sort_unstable_by(|left, right| right.cmp(left));
    row_indices
}

fn resolve_copy_row_indices(
    delegate: &EditorTableDelegate,
    display_row_indices: &[usize],
) -> Vec<usize> {
    display_row_indices
        .iter()
        .filter_map(|&display_row| delegate.resolve_display_row(display_row))
        .filter(|&actual_row| !delegate.is_deleted_row(actual_row))
        .collect()
}

fn build_large_text_editor_title(column_name: &str, display_row_ix: usize) -> String {
    t!(
        "TableDataGrid.edit_cell_title",
        column = column_name,
        row = display_row_ix + 1
    )
    .to_string()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LargeTextCellTarget {
    pub display_row_ix: usize,
    pub actual_row_ix: usize,
    pub col_ix: usize,
    pub column_name: String,
    pub value: String,
    pub editable: bool,
}

struct TextEditorDialogRequest {
    initial_text: String,
    title: String,
    row_ix: usize,
    col_ix: usize,
    editable: bool,
}

#[derive(Clone, Debug)]
pub enum DataGridEvent {
    LargeTextSelectionChanged,
    ToggleLargeTextEditorRequested,
    SaveChangesRequested,
    OpenTableDesignerRequested,
    OpenTableQueryRequested,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LargeTextEditorRoute {
    PagePreview,
    Dialog,
}

fn resolve_large_text_editor_route(mode: LargeTextCellEditorOpenMode) -> LargeTextEditorRoute {
    match mode {
        LargeTextCellEditorOpenMode::SidebarPreview => LargeTextEditorRoute::PagePreview,
        LargeTextCellEditorOpenMode::Dialog => LargeTextEditorRoute::Dialog,
    }
}

/// 数据表格使用场景
#[derive(Clone, Debug, PartialEq)]
pub enum DataGridUsage {
    /// 在表格数据页签中使用（编辑器高度较低）
    TableData,
    /// 在SQL结果页签中使用（编辑器高度较高）
    SqlResult,
}

#[derive(Debug)]
struct ExportPayload {
    columns: Vec<SharedString>,
    rows: Vec<Vec<Option<String>>>,
    binary_cells: Vec<BinaryCell>,
}

struct SqlResultExportRequest {
    connection_id: String,
    database: Option<String>,
    schema: Option<String>,
    session_id: Option<String>,
    sql: String,
}

fn result_set_export_exec_options() -> ExecOptions {
    ExecOptions {
        max_rows: None,
        ..Default::default()
    }
}

async fn list_columns_direct_on_runtime(
    cx: &mut AsyncApp,
    global_state: GlobalDbState,
    connection_id: String,
    database_name: String,
    schema_name: Option<String>,
    table_name: String,
) -> anyhow::Result<Vec<ColumnInfo>> {
    Tokio::spawn_result(cx, async move {
        global_state
            .list_columns_direct(&connection_id, &database_name, schema_name, &table_name)
            .await
    })
    .await
}

async fn list_indexes_direct_on_runtime(
    cx: &mut AsyncApp,
    global_state: GlobalDbState,
    connection_id: String,
    database_name: String,
    schema_name: Option<String>,
    table_name: String,
) -> anyhow::Result<Vec<IndexInfo>> {
    Tokio::spawn_result(cx, async move {
        global_state
            .list_indexes_direct(&connection_id, &database_name, schema_name, &table_name)
            .await
    })
    .await
}

async fn execute_sql_result_export(
    global_state: GlobalDbState,
    request: SqlResultExportRequest,
    cx: &mut AsyncApp,
) -> Result<ExportPayload, String> {
    let SqlResultExportRequest {
        connection_id,
        database,
        schema,
        session_id,
        sql,
    } = request;
    let options = Some(result_set_export_exec_options());
    let results = match session_id {
        Some(session_id) => {
            Tokio::spawn_result(cx, async move {
                global_state.execute_session(session_id, sql, options).await
            })
            .await
        }
        None => {
            global_state
                .execute_script(cx, connection_id, sql, database, schema, options)
                .await
        }
    }
    .map_err(|error| error.to_string())?;

    query_result_for_export(results)
}

fn query_result_for_export(results: Vec<SqlResult>) -> Result<ExportPayload, String> {
    let mut results = results.into_iter();
    let result = results
        .next()
        .ok_or_else(|| "query returned no results".to_string())?;
    if results.next().is_some() {
        return Err("query returned multiple results".to_string());
    }

    match result {
        SqlResult::Query(result) => Ok(DataGrid::normalize_query_result(result)),
        SqlResult::Exec(_) => Err("statement did not return a result set".to_string()),
        SqlResult::Error(error) => Err(error.message),
    }
}

/// 将结果集自带的列元数据转换为表格列元数据。
///
/// 结果集在驱动执行时已携带每列的 `db_type`，列序天然与结果列一致，
/// 无需额外查询表结构即可完成字段类型（日期、数值等）识别。
fn column_infos_from_query_meta(query_meta: &[QueryColumnMeta]) -> Vec<ColumnInfo> {
    query_meta
        .iter()
        .map(|meta| ColumnInfo {
            name: meta.name.clone(),
            data_type: meta.db_type.clone(),
            is_nullable: meta.nullable,
            is_primary_key: false,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
        })
        .collect()
}

fn notify_export_failure(
    cx: &mut AsyncApp,
    window_id: Option<gpui::AnyWindowHandle>,
    error: String,
) {
    let Some(window_id) = window_id else {
        return;
    };
    let _ = cx.update_window(window_id, |_entity, window, cx| {
        window.push_notification(
            t!("TableDataGrid.export_failed", error = error).to_string(),
            cx,
        );
    });
}

/// 数据表格配置
#[derive(Clone, Debug, PartialEq)]
pub struct DataGridConfig {
    /// 数据库名称
    pub database_name: String,
    /// Schema名称（用于支持schema的数据库如PostgreSQL、MSSQL、Oracle）
    pub schema_name: Option<String>,
    /// 表名称
    pub table_name: String,
    /// 数据库连接ID
    pub connection_id: String,
    /// 数据库类型
    pub database_type: DatabaseType,
    /// 是否允许编辑
    pub editable: bool,
    /// Whether the SQL result has a reliable direct-table schema identity.
    pub schema_metadata_safe: bool,
    /// 是否显示工具栏
    pub show_toolbar: bool,
    /// 使用场景
    pub usage: DataGridUsage,
    /// 原始 SQL（SqlResult 场景使用）
    pub sql: String,
    /// Existing execution session for manual transaction results.
    pub session_id: Option<String>,
    /// 执行时间（SqlResult 场景使用）
    execution_time: u128,
    /// 数据行数（SqlResult 场景使用）
    rows_count: usize,
}

impl DataGridConfig {
    pub fn new(
        database_name: impl Into<String>,
        table_name: impl Into<String>,
        connection_id: impl Into<String>,
        database_type: DatabaseType,
    ) -> Self {
        Self {
            database_name: database_name.into(),
            schema_name: None,
            table_name: table_name.into(),
            connection_id: connection_id.into(),
            database_type,
            editable: true,
            schema_metadata_safe: false,
            show_toolbar: true,
            usage: DataGridUsage::TableData,
            sql: "".to_string(),
            session_id: None,
            execution_time: 0,
            rows_count: 0,
        }
    }

    pub fn with_schema(mut self, schema: impl Into<String>) -> Self {
        self.schema_name = Some(schema.into());
        self
    }

    pub fn editable(mut self, editable: bool) -> Self {
        self.editable = editable;
        self
    }

    pub fn schema_metadata_safe(mut self, safe: bool) -> Self {
        self.schema_metadata_safe = safe;
        self
    }

    pub fn show_toolbar(mut self, show: bool) -> Self {
        self.show_toolbar = show;
        self
    }

    pub fn usage(mut self, usage: DataGridUsage) -> Self {
        self.usage = usage;
        self
    }

    pub fn sql(mut self, sql: impl Into<String>) -> Self {
        self.sql = sql.into();
        self
    }
    pub fn with_session_id(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }
    pub fn execution_time(mut self, execution_time: u128) -> Self {
        self.execution_time = execution_time;
        self
    }
    pub fn rows_count(mut self, rows_count: usize) -> Self {
        self.rows_count = rows_count;
        self
    }
}

#[derive(Clone)]
pub struct TableDataInfo {
    pub current_page: usize,
    pub page_size: usize,
    pub total_count: usize,
    pub duration: u128,
    pub current_sql: String,
    pub columns: Vec<ColumnInfo>,
    pub index_infos: Vec<IndexInfo>,
    pub error_message: Option<String>,
}

impl Default for TableDataInfo {
    fn default() -> Self {
        Self {
            current_page: 1,
            page_size: 500,
            total_count: 0,
            duration: 0,
            current_sql: String::new(),
            columns: vec![],
            index_infos: vec![],
            error_message: None,
        }
    }
}

#[derive(Clone, Copy)]
enum ExportScope {
    All,
    CurrentPage,
}

#[derive(Clone, Copy)]
enum ExportFormat {
    Xlsx,
    Csv,
    InsertSql,
}

impl ExportFormat {
    fn extension(self) -> &'static str {
        match self {
            ExportFormat::Xlsx => "xlsx",
            ExportFormat::Csv => "csv",
            ExportFormat::InsertSql => "sql",
        }
    }

    fn copy_format(self) -> CopyFormat {
        match self {
            ExportFormat::Xlsx | ExportFormat::Csv => CopyFormat::Csv,
            ExportFormat::InsertSql => CopyFormat::SqlInsert,
        }
    }

    fn include_header(self) -> bool {
        matches!(self, ExportFormat::Xlsx | ExportFormat::Csv)
    }
}

struct ExportBuildInput {
    rows: Vec<Vec<Option<String>>>,
    binary_cells: Vec<BinaryCell>,
    columns: Vec<SharedString>,
    metadata: TableMetadata,
    plugin: Option<Arc<dyn DatabasePlugin>>,
}

fn build_export_bytes(
    format: ExportFormat,
    input: ExportBuildInput,
) -> Result<Option<Vec<u8>>, String> {
    let ExportBuildInput {
        rows,
        mut binary_cells,
        columns,
        metadata,
        plugin,
    } = input;
    if rows.is_empty() {
        return Ok(None);
    }

    let mut export_rows = rows;

    if format.include_header() {
        let header = columns
            .iter()
            .map(|column| Some(column.as_ref().to_string()))
            .collect();
        export_rows.insert(0, header);
        for cell in &mut binary_cells {
            cell.row_index += 1;
        }
    }

    let metadata = metadata.for_columns(&columns);

    match format {
        ExportFormat::Xlsx => build_xlsx_bytes(&export_rows, &binary_cells).map(Some),
        _ => {
            let context = CopyFormatContext::new(&export_rows, &columns, &metadata)
                .with_binary_cells(&binary_cells);
            let context = plugin
                .as_deref()
                .map_or(context, |plugin| context.with_plugin(plugin));
            Ok(Some(
                CopyFormatter::format(format.copy_format(), context).into_bytes(),
            ))
        }
    }
}

fn build_xlsx_bytes(
    rows: &[Vec<Option<String>>],
    binary_cells: &[BinaryCell],
) -> Result<Vec<u8>, String> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    let mut binary_index = HashMap::with_capacity(binary_cells.len());
    for cell in binary_cells {
        binary_index
            .entry((cell.row_index, cell.column_index))
            .or_insert_with(|| cell.bytes.as_slice());
    }

    for (row_index, row) in rows.iter().enumerate() {
        for (col_index, cell) in row.iter().enumerate() {
            if let Some(bytes) = binary_index.get(&(row_index, col_index)) {
                worksheet
                    .write_string(
                        row_index as u32,
                        col_index as u16,
                        format_binary_input(bytes),
                    )
                    .map_err(|error| error.to_string())?;
            } else if let Some(cell) = cell {
                worksheet
                    .write_string(row_index as u32, col_index as u16, cell)
                    .map_err(|error| error.to_string())?;
            }
        }
    }

    workbook.save_to_buffer().map_err(|error| error.to_string())
}

fn table_has_unsaved_changes(editing_cell: Option<(usize, usize)>, change_count: usize) -> bool {
    editing_cell.is_some() || change_count > 0
}

fn first_execution_error(results: &[SqlResult]) -> Option<&str> {
    results.iter().find_map(|result| match result {
        SqlResult::Error(error) => Some(error.message.as_str()),
        _ => None,
    })
}

fn data_generation_is_current(generation: &AtomicU64, expected: u64) -> bool {
    generation.load(Ordering::Acquire) == expected
}

/// 数据表格组件
pub struct DataGrid {
    /// 组件配置
    config: DataGridConfig,
    /// 内部表格状态
    pub(crate) table: Entity<EditTableState<EditorTableDelegate>>,
    /// 表格事件订阅
    _table_sub: Option<Subscription>,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 表格数据信息（分页、总数等）
    table_data_info: Entity<TableDataInfo>,
    /// 过滤器编辑器
    filter_editor: Entity<TableFilterEditor>,
    /// 过滤器事件订阅
    _filter_sub: Option<Subscription>,
    /// 当前页本地搜索输入框
    search_input: Entity<InputState>,
    /// 搜索输入框事件订阅
    _search_sub: Option<Subscription>,
    /// 当前数据库 tab 的共享 SQL 执行记录
    execution_history: Option<Entity<ExecutionHistoryPanel>>,
    /// 侧边栏大文本编辑器是否已为当前表格打开
    is_large_text_editor_sidebar_open: bool,
    /// Invalidates stale asynchronous data and metadata callbacks.
    data_generation: Arc<AtomicU64>,
}

impl DataGrid {
    pub fn new(
        config: DataGridConfig,
        execution_history: Option<Entity<ExecutionHistoryPanel>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let editable = config.editable;
        let is_table_data = config.usage == DataGridUsage::TableData;
        let database_type = config.database_type.clone();
        let table_name = config.table_name.clone();
        let data_grid_handle = cx.entity().downgrade();
        let table = cx.new(|cx| {
            let mut delegate =
                EditorTableDelegate::new(vec![], vec![], editable, database_type, window, cx);
            // 设置表名，用于 SQL 生成
            delegate.set_table_name(table_name);
            EditTableState::new(delegate, window, cx)
        });
        table.update(cx, |state, _| {
            state.delegate_mut().set_data_grid(data_grid_handle.clone());
        });
        let focus_handle = cx.focus_handle();
        let filter_editor = cx.new(|cx| TableFilterEditor::new(window, cx));
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("TableDataGrid.search_placeholder").to_string())
                .clean_on_escape()
        });
        let table_data_info = cx.new(|_| TableDataInfo::default());
        let mut result = Self {
            config,
            table,
            _table_sub: None,
            focus_handle,
            table_data_info,
            filter_editor,
            _filter_sub: None,
            search_input,
            _search_sub: None,
            execution_history,
            is_large_text_editor_sidebar_open: false,
            data_generation: Arc::new(AtomicU64::new(0)),
        };
        result.bind_table_event(window, cx);
        if is_table_data {
            result.bind_filter_event(window, cx);
            result.bind_search_event(window, cx);
            result.load_data_with_clauses(1, cx);
        }
        result
    }

    fn bind_table_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sub = cx.subscribe_in(
            &self.table,
            window,
            |_this, _, evt: &EditTableEvent, _window, cx| match evt {
                EditTableEvent::SelectCell(row, col) => {
                    trace!("select cell: {:?}", (row, col));
                    cx.emit(DataGridEvent::LargeTextSelectionChanged);
                }
                EditTableEvent::SelectionChanged(_) | EditTableEvent::CellEdited(_, _) => {
                    cx.emit(DataGridEvent::LargeTextSelectionChanged);
                }
                _ => {}
            },
        );
        self._table_sub = Some(sub);
    }

    fn bind_filter_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sub = cx.subscribe_in(
            &self.filter_editor,
            window,
            |this: &mut DataGrid, _, evt: &FilterEditorEvent, _, cx| match evt {
                FilterEditorEvent::QueryApply => {
                    this.load_data_with_clauses(1, cx);
                    cx.notify()
                }
            },
        );
        self._filter_sub = Some(sub);
    }

    fn bind_search_event(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sub = cx.subscribe_in(
            &self.search_input,
            window,
            |this: &mut DataGrid, input, evt: &InputEvent, _window, cx| {
                if let InputEvent::Change = evt {
                    let query = input.read(cx).text().to_string();
                    this.apply_row_search_query(query, cx);
                }
            },
        );
        self._search_sub = Some(sub);
    }

    fn apply_row_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| {
            state.delegate_mut().set_row_search_query(query);
            cx.notify();
        });
        cx.notify();
    }

    fn execution_context(&self) -> ExecutionContext {
        ExecutionContext {
            connection_id: self.config.connection_id.clone(),
            database: Some(self.config.database_name.clone()),
            schema: self.config.schema_name.clone(),
        }
    }

    fn record_execution_results(&self, sql: String, results: &[SqlResult], cx: &mut App) {
        let Some(execution_history) = &self.execution_history else {
            return;
        };
        execution_history.update(cx, |history, cx| {
            history.record_table_data_results(self.execution_context(), sql, results, cx);
        });
    }

    fn record_execution_failure(&self, error: String, sql: Option<String>, cx: &mut App) {
        let Some(execution_history) = &self.execution_history else {
            return;
        };
        execution_history.update(cx, |history, cx| {
            history.record_transport_error(
                self.execution_context(),
                sql.unwrap_or_default(),
                error,
                cx,
            );
        });
    }

    fn on_action_focus_search(
        &mut self,
        _: &FocusSearchInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.config.usage == DataGridUsage::TableData {
            focus_search_input(&self.search_input, window, cx);
        }
    }

    // ========== 公共访问器 ==========

    pub fn table(&self) -> &Entity<EditTableState<EditorTableDelegate>> {
        &self.table
    }

    pub fn update_data(
        &self,
        columns: Vec<Column>,
        rows: Vec<Vec<Option<String>>>,
        rowids: Vec<String>,
        cx: &mut App,
    ) {
        self.update_data_with_binary_cells(columns, rows, rowids, vec![], cx);
    }

    pub fn update_data_with_binary_cells(
        &self,
        columns: Vec<Column>,
        rows: Vec<Vec<Option<String>>>,
        rowids: Vec<String>,
        binary_cells: Vec<BinaryCell>,
        cx: &mut App,
    ) {
        self.data_generation.fetch_add(1, Ordering::AcqRel);
        self.table.update(cx, |state, cx| {
            state.delegate_mut().update_data_with_binary_cells(
                columns,
                rows,
                rowids,
                binary_cells,
                cx,
            );
            state.refresh(cx);
        });
    }

    pub(crate) fn current_data_generation(&self) -> u64 {
        self.data_generation.load(Ordering::Acquire)
    }

    pub(crate) fn is_data_generation_current(&self, expected: u64) -> bool {
        data_generation_is_current(&self.data_generation, expected)
    }

    pub(crate) fn download_binary_value(
        &self,
        bytes: Vec<u8>,
        file_name: String,
        cx: &mut Context<Self>,
    ) {
        let window_id = cx.active_window();
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            multiple: false,
            directories: true,
            prompt: Some(t!("TableData.select_binary_download_directory").into()),
        });

        cx.spawn(async move |_this, cx| {
            let output_dir = match future.await {
                Ok(Ok(Some(paths))) => match paths.into_iter().next() {
                    Some(path) => path,
                    None => return,
                },
                _ => return,
            };
            let output_path = output_dir.join(file_name);
            let path_for_write = output_path.clone();
            let write_result = cx
                .background_spawn(async move { std::fs::write(path_for_write, bytes) })
                .await;

            let _ = cx.update(|cx| {
                if let Some(window_id) = window_id {
                    let _ = cx.update_window(window_id, |_entity, window, cx| match write_result {
                        Ok(()) => window.push_notification(
                            Notification::success(
                                t!(
                                    "TableData.binary_download_complete",
                                    path = output_path.display()
                                )
                                .to_string(),
                            )
                            .autohide(true),
                            cx,
                        ),
                        Err(error) => window.push_notification(
                            Notification::error(
                                t!("TableData.binary_download_failed", error = error).to_string(),
                            )
                            .autohide(true),
                            cx,
                        ),
                    });
                }
            });
        })
        .detach();
    }

    pub(crate) fn show_binary_image_preview(
        &self,
        bytes: Arc<Vec<u8>>,
        format: ImageFormat,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let image = Arc::new(Image::from_bytes(format, bytes.as_ref().clone()));
        window.open_dialog(cx, move |dialog, _window, _cx| {
            dialog
                .title(SharedString::from(title.clone()))
                .w(px(900.))
                .h(px(680.))
                .child(
                    div()
                        .size_full()
                        .min_w_0()
                        .min_h_0()
                        .overflow_hidden()
                        .child(
                            img(image.clone())
                                .size_full()
                                .object_fit(ObjectFit::Contain),
                        ),
                )
                .close_button(true)
                .overlay(false)
                .content_center()
        });
    }

    /// 快路径：SQL 结果页签直接使用结果集自带的列元数据。
    ///
    /// 结果集在驱动执行时已携带每列的 `db_type`，无需额外查询表结构即可完成
    /// 日期/数值等字段类型识别。仅当列类型需要在结果列上重算时（见
    /// [`DataGrid::load_column_meta_for_sql_result`]）才需要慢路径的表结构查询。
    pub fn set_sql_result_column_meta(&self, query_meta: Vec<QueryColumnMeta>, cx: &mut App) {
        let meta = column_infos_from_query_meta(&query_meta);
        self.table.update(cx, |state, cx| {
            state.delegate_mut().set_column_meta(meta);
            cx.notify();
        });
    }

    pub fn load_column_meta_for_sql_result(&self, cx: &mut App) {
        if self.config.usage != DataGridUsage::SqlResult
            || self.config.sql.is_empty()
            || self.config.database_type != DatabaseType::MySQL
            || !self.config.schema_metadata_safe
            || self.config.table_name.is_empty()
        {
            return;
        }

        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let table_name = self.config.table_name.clone();
        let table = self.table.clone();
        let table_info = self.table_data_info.clone();
        let data_generation = self.data_generation.clone();
        let expected_generation = data_generation.load(Ordering::Acquire);
        cx.spawn(async move |cx: &mut AsyncApp| {
            let result = list_columns_direct_on_runtime(
                cx,
                global_state,
                connection_id,
                database_name,
                schema_name,
                table_name,
            )
            .await;

            if let Ok(cols) = result {
                cx.update(|cx| {
                    if !data_generation_is_current(&data_generation, expected_generation) {
                        return;
                    }

                    let reconcile_result = table.update(cx, |state, cx| {
                        let result = state
                            .delegate_mut()
                            .reconcile_binary_cells_with_column_meta(cols.clone());
                        if result.is_ok() {
                            state.refresh(cx);
                            cx.notify();
                        }
                        result
                    });
                    match reconcile_result {
                        Ok(()) => {
                            table_info.update(cx, |info, cx| {
                                info.columns = cols;
                                cx.notify();
                            });
                        }
                        Err(error) => {
                            tracing::warn!(
                                "failed to reconcile SQL result binary cells with metadata: {error}"
                            );
                        }
                    }
                })
            }
        })
        .detach();
    }

    pub fn set_editable(&mut self, editable: bool, cx: &mut Context<Self>) {
        if self.config.editable == editable {
            return;
        }

        self.config.editable = editable;
        self.table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            delegate.set_editable(editable);
            if !editable {
                delegate.clear_changes();
            }
            state.refresh(cx);
            cx.notify();
        });
        cx.notify();
    }

    pub(crate) fn apply_column_sort(
        &mut self,
        column_name: &str,
        sort: ColumnSort,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.config.usage != DataGridUsage::TableData {
            return;
        }

        let global_state = cx.global::<GlobalDbState>().clone();
        let order_by_clause = match build_header_order_by_clause(
            &global_state.db_manager,
            self.config.database_type.clone(),
            column_name,
            sort,
        ) {
            Ok(Some(clause)) => clause,
            Ok(None) => String::new(),
            Err(error) => {
                window.push_notification(error, cx);
                return;
            }
        };

        self.filter_editor.update(cx, |editor, cx| {
            editor.set_order_by_clause(order_by_clause.clone(), window, cx);
        });

        self.load_data_with_clauses(1, cx);
    }

    // ========== 数据加载 ==========

    fn load_data_with_clauses(&self, page: usize, cx: &mut App) {
        let expected_generation = self.data_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let data_generation = self.data_generation.clone();
        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let table_name = self.config.table_name.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let table = self.table.clone();
        let table_data_info = self.table_data_info.clone();
        let where_clause = self.filter_editor.read(cx).get_where_clause(cx);
        let order_by_clause = self.filter_editor.read(cx).get_order_by_clause(cx);
        let filter_editor = self.filter_editor.clone();
        let page_size = self.table_data_info.read(cx).page_size;

        tracing::info!(
            "load_data_with_clauses: connection_id={}, database={}, table={}",
            connection_id,
            database_name,
            table_name
        );

        self.table.update(cx, |state, cx| {
            state.delegate_mut().set_loading(true);
            cx.notify();
        });

        cx.spawn(async move |cx: &mut AsyncApp| {
            let mut request = TableDataRequest::new(&database_name, &table_name)
                .with_page(page, page_size)
                .with_where_clause(where_clause.clone())
                .with_order_by_clause(order_by_clause.clone());

            if let Some(schema) = schema_name.clone() {
                request = request.with_schema(schema);
            }

            let result = global_state
                .query_table_data(cx, connection_id.clone(), request)
                .await;

            match result {
                Err(err) => {
                    error!("load_data_with_clauses failed: {}", err);
                    cx.update(|cx| {
                        if !data_generation_is_current(&data_generation, expected_generation) {
                            return;
                        }
                        table_data_info.update(cx, |info, cx| {
                            info.error_message =
                                Some(t!("TableDataGrid.load_data_failed", error = err).to_string());
                            cx.notify();
                        });
                        table.update(cx, |state, cx| {
                            state.delegate_mut().set_loading(false);
                            state.delegate_mut().update_data(vec![], vec![], vec![], cx);
                            state.refresh(cx);
                        });
                    })
                }
                Ok(response) => {
                    if !data_generation_is_current(&data_generation, expected_generation) {
                        return;
                    }
                    let query_result = response.query_result;

                    let (columns, rows, rowids, binary_cells) =
                        if query_result.columns.first().map(|c| c.as_str()) == Some("__rowid__") {
                            let columns: Vec<Column> = query_result
                                .columns
                                .iter()
                                .skip(1)
                                .map(|col| Column::new(col.clone(), col.clone()))
                                .collect();
                            let mut rowids = Vec::new();
                            let rows: Vec<Vec<Option<String>>> = query_result
                                .rows
                                .iter()
                                .map(|row| {
                                    if let Some(first) = row.first() {
                                        rowids.push(first.clone().unwrap_or_default());
                                    }
                                    row.iter().skip(1).cloned().collect()
                                })
                                .collect();
                            let binary_cells = query_result
                                .binary_cells
                                .iter()
                                .filter_map(|cell| {
                                    cell.column_index.checked_sub(1).map(|column_index| {
                                        BinaryCell {
                                            row_index: cell.row_index,
                                            column_index,
                                            bytes: cell.bytes.clone(),
                                        }
                                    })
                                })
                                .collect();
                            (columns, rows, rowids, binary_cells)
                        } else {
                            let columns: Vec<Column> = query_result
                                .columns
                                .iter()
                                .map(|col| Column::new(col.clone(), col.clone()))
                                .collect();
                            let rows: Vec<Vec<Option<String>>> = query_result
                                .rows
                                .iter()
                                .map(|row| row.iter().cloned().collect())
                                .collect();
                            (columns, rows, Vec::new(), query_result.binary_cells.clone())
                        };

                    let column_meta: Vec<ColumnInfo> =
                        column_infos_from_query_meta(&query_result.column_meta)
                            .into_iter()
                            .filter(|col| col.name != "__rowid__")
                            .collect();

                    cx.update(|cx| {
                        if !data_generation_is_current(&data_generation, expected_generation) {
                            return;
                        }
                        table_data_info.update(cx, |info, cx| {
                            info.total_count = response.total_count;
                            info.current_sql = query_result.sql.clone();
                            info.duration = response.duration;
                            info.current_page = response.page;
                            info.columns = column_meta.clone();
                            info.index_infos = vec![];
                            info.error_message = None;
                            cx.notify();
                        });
                    });

                    cx.update(|cx| {
                        if !data_generation_is_current(&data_generation, expected_generation) {
                            return;
                        }
                        filter_editor.update(cx, |editor, cx| {
                            editor.set_schema(
                                TableSchema {
                                    columns: column_meta.clone(),
                                },
                                cx,
                            );
                        });

                        table.update(cx, |state, cx| {
                            state.delegate_mut().set_loading(false);
                            state.delegate_mut().set_column_meta(column_meta);
                            state.delegate_mut().update_data_with_binary_cells(
                                columns,
                                rows,
                                rowids,
                                binary_cells,
                                cx,
                            );
                            state.delegate_mut().apply_order_by_clause(&order_by_clause);
                            state.refresh(cx);
                        });
                    });

                    match list_columns_direct_on_runtime(
                        cx,
                        global_state,
                        connection_id,
                        database_name,
                        schema_name,
                        table_name,
                    )
                    .await
                    {
                        Ok(column_meta) => {
                            cx.update(|cx| {
                                if !data_generation_is_current(
                                    &data_generation,
                                    expected_generation,
                                ) {
                                    return;
                                }

                                filter_editor.update(cx, |editor, cx| {
                                    editor.set_schema(
                                        TableSchema {
                                            columns: column_meta.clone(),
                                        },
                                        cx,
                                    );
                                });

                                let reconcile_result = table.update(cx, |state, cx| {
                                    let result = state
                                        .delegate_mut()
                                        .reconcile_binary_cells_with_column_meta(
                                            column_meta.clone(),
                                        );
                                    if result.is_ok() {
                                        state.refresh(cx);
                                    }
                                    result
                                });
                                match reconcile_result {
                                    Ok(()) => {
                                        table_data_info.update(cx, |info, cx| {
                                            info.columns = column_meta;
                                            cx.notify();
                                        });
                                    }
                                    Err(error) => tracing::warn!(
                                        "failed to reconcile table data binary cells with metadata: {error}"
                                    ),
                                }
                            });
                        }
                        Err(err) => {
                            trace!("load_data_with_clauses list_columns failed: {}", err);
                        }
                    }
                }
            }
        })
        .detach();
    }

    fn handle_refresh(&self, cx: &mut App) {
        match self.config.usage {
            DataGridUsage::TableData => {
                let page = self.table_data_info.read(cx).current_page;
                self.load_data_with_clauses(page, cx);
            }
            DataGridUsage::SqlResult => {
                self.load_data_with_sql(self.config.sql.clone(), cx);
            }
        }
    }

    pub fn refresh_data(&self, cx: &mut App) {
        if self.table.read(cx).delegate().is_loading() {
            return;
        }
        self.handle_refresh(cx);
    }

    pub fn open_large_text_editor(&self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = cx.global::<AppSettings>();
        match resolve_large_text_editor_route(settings.large_text_cell_editor_open_mode) {
            LargeTextEditorRoute::PagePreview => {
                cx.emit(DataGridEvent::ToggleLargeTextEditorRequested);
            }
            LargeTextEditorRoute::Dialog => {
                self.show_large_text_editor(window, cx);
            }
        }
    }

    pub fn set_large_text_editor_sidebar_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.is_large_text_editor_sidebar_open == open {
            return;
        }

        self.is_large_text_editor_sidebar_open = open;
        cx.notify();
    }

    pub fn open_export_view(&self, window: &mut Window, cx: &mut App) {
        if self.config.usage != DataGridUsage::TableData {
            window.push_notification(t!("TableDataGrid.export_not_supported").to_string(), cx);
            return;
        }

        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let table_name = self.config.table_name.clone();

        let export_view = cx.new(|cx| {
            DataExportView::new(
                connection_id.clone(),
                database_name.clone(),
                schema_name.clone(),
                table_name.clone(),
                window,
                cx,
            )
        });

        let existing_columns = self.table_data_info.read(cx).columns.clone();
        if !existing_columns.is_empty() {
            export_view.update(cx, |view, cx| {
                view.update_column_list(existing_columns, cx);
            });
        } else {
            let global_state = cx.global::<GlobalDbState>().clone();
            let export_view_handle = export_view.clone();
            let connection_id_for_columns = self.config.connection_id.clone();
            let database_name_for_columns = self.config.database_name.clone();
            let schema_name_for_columns = self.config.schema_name.clone();
            let table_name_for_columns = self.config.table_name.clone();

            cx.spawn(async move |cx: &mut AsyncApp| {
                let columns_result = list_columns_direct_on_runtime(
                    cx,
                    global_state,
                    connection_id_for_columns,
                    database_name_for_columns,
                    schema_name_for_columns,
                    table_name_for_columns,
                )
                .await;

                match columns_result {
                    Ok(columns) => {
                        cx.update(|cx| {
                            export_view_handle.update(cx, |view, cx| {
                                view.update_column_list(columns, cx);
                            });
                        });
                    }
                    Err(error) => {
                        error!("Failed to fetch export columns: {}", error);
                    }
                }
            })
            .detach();
        }

        open_popup_window(
            PopupWindowOptions::new(t!("TableDataGrid.export_table").to_string())
                .size(800.0, 600.0),
            move |_window, _cx| export_view.clone(),
            Some(window),
            cx,
        );
    }

    fn export_result_set(
        &self,
        scope: ExportScope,
        format: ExportFormat,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.table.read(cx).delegate().is_loading() {
            window.push_notification(t!("TableDataGrid.export_loading").to_string(), cx);
            return;
        }

        let usage = self.config.usage.clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let session_id = self.config.session_id.clone();
        let sql = self.config.sql.clone();
        let table_name = self.config.table_name.clone();
        let current_page = self.table_data_info.read(cx).current_page;
        let total_count = self.table_data_info.read(cx).total_count;
        let where_clause = self.filter_editor.read(cx).get_where_clause(cx);
        let order_by_clause = self.filter_editor.read(cx).get_order_by_clause(cx);
        let table = self.table.clone();
        let metadata = self.table.read(cx).delegate().get_table_metadata();
        let window_id = cx.active_window();
        let global_state = cx.global::<GlobalDbState>().clone();
        let plugin = global_state.get_plugin(&self.config.database_type).ok();
        let prompt_future = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            multiple: false,
            directories: true,
            prompt: Some(t!("TableDataGrid.select_export_directory").into()),
        });

        cx.spawn(async move |cx: &mut AsyncApp| {
            let output_dir: PathBuf = match prompt_future.await {
                Ok(Ok(Some(paths))) => match paths.first() {
                    Some(path) => path.clone(),
                    None => return,
                },
                _ => return,
            };

            let export_payload = match scope {
                ExportScope::CurrentPage => match Self::collect_visible_rows(&table, cx) {
                    Ok(payload) => payload,
                    Err(error) => {
                        notify_export_failure(cx, window_id, error);
                        return;
                    }
                },
                ExportScope::All => match usage {
                    DataGridUsage::TableData => {
                        if total_count == 0 {
                            None
                        } else {
                            let mut request = TableDataRequest::new(&database_name, &table_name)
                                .with_page(1, total_count)
                                .with_where_clause(where_clause)
                                .with_order_by_clause(order_by_clause);

                            if let Some(schema) = schema_name.clone() {
                                request = request.with_schema(schema);
                            }

                            match global_state
                                .query_table_data(cx, connection_id.clone(), request)
                                .await
                            {
                                Ok(response) => {
                                    let query_result = response.query_result;
                                    Some(Self::normalize_query_result(query_result))
                                }
                                Err(error) => {
                                    notify_export_failure(cx, window_id, error.to_string());
                                    return;
                                }
                            }
                        }
                    }
                    DataGridUsage::SqlResult => {
                        let request = SqlResultExportRequest {
                            connection_id: connection_id.clone(),
                            database: (!database_name.is_empty()).then_some(database_name.clone()),
                            schema: schema_name.clone(),
                            session_id: session_id.clone(),
                            sql: sql.clone(),
                        };
                        match execute_sql_result_export(global_state.clone(), request, cx).await {
                            Ok(payload) => Some(payload),
                            Err(error) => {
                                notify_export_failure(cx, window_id, error);
                                return;
                            }
                        }
                    }
                },
            };

            let Some(ExportPayload {
                columns,
                rows,
                binary_cells,
            }) = export_payload
            else {
                let _ = cx.update(|cx| {
                    if let Some(window_id) = window_id {
                        let _ = cx.update_window(window_id, |_entity, window, cx| {
                            window.push_notification(
                                t!("TableDataGrid.no_data_to_export").to_string(),
                                cx,
                            );
                        });
                    }
                });
                return;
            };

            let input = ExportBuildInput {
                rows,
                binary_cells,
                columns: columns.clone(),
                metadata: metadata.clone(),
                plugin: plugin.clone(),
            };
            let bytes = match build_export_bytes(format, input) {
                Ok(Some(bytes)) => bytes,
                Ok(None) => {
                    let _ = cx.update(|cx| {
                        if let Some(window_id) = window_id {
                            let _ = cx.update_window(window_id, |_entity, window, cx| {
                                window.push_notification(
                                    t!("TableDataGrid.no_data_to_export").to_string(),
                                    cx,
                                );
                            });
                        }
                    });
                    return;
                }
                Err(error) => {
                    let _ = cx.update(|cx| {
                        if let Some(window_id) = window_id {
                            let _ = cx.update_window(window_id, |_entity, window, cx| {
                                window.push_notification(
                                    t!("TableDataGrid.export_failed", error = error).to_string(),
                                    cx,
                                );
                            });
                        }
                    });
                    return;
                }
            };

            let base_name = if table_name.is_empty() {
                "result_set"
            } else {
                table_name.as_str()
            };
            let scope_suffix = match scope {
                ExportScope::All => "all".to_string(),
                ExportScope::CurrentPage => format!("page-{}", current_page),
            };
            let now = Local::now();
            let datetime_str = now.format("%Y-%m-%d_%H-%M-%S").to_string();
            let prefix = if database_name.is_empty() {
                base_name.to_string()
            } else {
                format!("{}_{}", database_name, base_name)
            };
            let filename = format!(
                "{}_{}_{}.{}",
                prefix,
                scope_suffix,
                datetime_str,
                format.extension()
            );
            let full_path = output_dir.join(filename);
            let full_path_for_write = full_path.clone();

            let write_result = cx
                .background_spawn(async move { std::fs::write(&full_path_for_write, bytes) })
                .await;

            let _ = cx.update(|cx| {
                if let Some(window_id) = window_id {
                    let _ = cx.update_window(window_id, |_entity, window, cx| match write_result {
                        Ok(()) => {
                            window.push_notification(
                                t!("TableDataGrid.export_complete", path = full_path.display())
                                    .to_string(),
                                cx,
                            );
                        }
                        Err(error) => {
                            window.push_notification(
                                t!("TableDataGrid.export_failed", error = error).to_string(),
                                cx,
                            );
                        }
                    });
                }
            });
        })
        .detach();
    }

    fn collect_visible_rows(
        table: &Entity<EditTableState<EditorTableDelegate>>,
        cx: &AsyncApp,
    ) -> Result<Option<ExportPayload>, String> {
        table.read_with(cx, |table_state, _cx| {
            let delegate = table_state.delegate();
            let row_count = delegate.filtered_row_count();
            if row_count == 0 {
                return Ok(None);
            }

            let mut row_indices = Vec::with_capacity(row_count);
            for display_row in 0..row_count {
                let Some(actual_row) = delegate.resolve_display_row(display_row) else {
                    continue;
                };
                if delegate.is_deleted_row(actual_row) {
                    continue;
                }
                row_indices.push(actual_row);
            }

            let (rows, binary_cells) = delegate
                .get_rows_with_binary_cells(&row_indices)
                .map_err(|error| error.to_string())?;
            if rows.is_empty() {
                return Ok(None);
            }

            let columns = delegate
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect();

            Ok(Some(ExportPayload {
                columns,
                rows,
                binary_cells,
            }))
        })
    }

    fn normalize_query_result(query_result: QueryResult) -> ExportPayload {
        if query_result.columns.first().map(|name| name.as_str()) == Some("__rowid__") {
            let columns = query_result
                .columns
                .into_iter()
                .skip(1)
                .map(SharedString::from)
                .collect();
            let rows = query_result
                .rows
                .into_iter()
                .map(|row| row.into_iter().skip(1).collect())
                .collect();
            let binary_cells = query_result
                .binary_cells
                .into_iter()
                .filter_map(|cell| {
                    cell.column_index
                        .checked_sub(1)
                        .map(|column_index| BinaryCell {
                            row_index: cell.row_index,
                            column_index,
                            bytes: cell.bytes,
                        })
                })
                .collect();
            ExportPayload {
                columns,
                rows,
                binary_cells,
            }
        } else {
            let columns = query_result
                .columns
                .into_iter()
                .map(SharedString::from)
                .collect();
            ExportPayload {
                columns,
                rows: query_result.rows,
                binary_cells: query_result.binary_cells,
            }
        }
    }

    fn load_data_with_sql(&self, sql: String, cx: &mut App) {
        let expected_generation = self.data_generation.fetch_add(1, Ordering::AcqRel) + 1;
        let data_generation = self.data_generation.clone();
        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let table = self.table.clone();
        let table_name = self.config.table_name.clone();
        let editable = self.config.editable;
        let need_authoritative_meta = self.config.database_type == DatabaseType::MySQL
            && self.config.schema_metadata_safe
            && !self.config.table_name.is_empty();

        cx.spawn(async move |cx: &mut AsyncApp| {
            let result: Result<(SqlResult, Option<Vec<ColumnInfo>>), anyhow::Error> = async {
                let sql_result = global_state
                    .execute_single(
                        cx,
                        connection_id.clone(),
                        sql.clone(),
                        Some(database_name.clone()),
                        None,
                    )
                    .await?;
                if editable && need_authoritative_meta {
                    let column_meta = list_columns_direct_on_runtime(
                        cx,
                        global_state.clone(),
                        connection_id.clone(),
                        database_name.clone(),
                        schema_name.clone(),
                        table_name.clone(),
                    )
                    .await?;
                    return Ok((sql_result, Some(column_meta)));
                }
                Ok((sql_result, None))
            }
            .await;

            match result {
                Err(err) => cx.update(|cx| {
                    if !data_generation_is_current(&data_generation, expected_generation) {
                        return;
                    }
                    notification(
                        cx,
                        t!("TableDataGrid.execute_sql_failed", error = err.to_string()).to_string(),
                    );
                }),
                Ok(results) => {
                    if !data_generation_is_current(&data_generation, expected_generation) {
                        return;
                    }
                    let (result, column_meta) = results;
                    if let SqlResult::Query(query_result) = result {
                        let binary_cells = query_result.binary_cells.clone();
                        let columns: Vec<Column> = query_result
                            .columns
                            .iter()
                            .map(|col| Column::new(col.clone(), col.clone()))
                            .collect();
                        let rows: Vec<Vec<Option<String>>> = query_result
                            .rows
                            .iter()
                            .map(|row| row.iter().cloned().collect())
                            .collect();
                        cx.update(|cx| {
                            if !data_generation_is_current(
                                &data_generation,
                                expected_generation,
                            ) {
                                return;
                            }
                            table.update(cx, |state, cx| {
                                state.delegate_mut().update_data_with_binary_cells(
                                    columns,
                                    rows,
                                    vec![],
                                    binary_cells,
                                    cx,
                                );
                                // 快路径：结果集自带的列类型直接可用
                                state
                                    .delegate_mut()
                                    .set_column_meta(column_infos_from_query_meta(
                                        &query_result.column_meta,
                                    ));
                                if let Some(columns) = column_meta
                                    && let Err(error) = state
                                        .delegate_mut()
                                        .reconcile_binary_cells_with_column_meta(columns)
                                {
                                    tracing::warn!(
                                        "failed to reconcile refreshed SQL result binary cells with metadata: {error}"
                                    );
                                }
                                state.refresh(cx);
                            });
                        })
                    }
                }
            }
        })
        .detach();
    }

    fn handle_prev_page(&self, cx: &mut App) {
        let page = self.table_data_info.read(cx).current_page;
        if page > 1 {
            self.load_data_with_clauses(page - 1, cx);
        }
    }

    fn handle_next_page(&self, cx: &mut App) {
        let info = self.table_data_info.read(cx);
        let page = info.current_page;
        let total = info.total_count;
        let page_size = info.page_size;

        if page_size == 0 {
            return;
        }
        let total_pages = total.div_ceil(page_size);
        if page < total_pages {
            self.load_data_with_clauses(page + 1, cx);
        }
    }

    fn handle_page_size_change(&self, new_size: usize, cx: &mut App) {
        self.table_data_info.update(cx, |info, cx| {
            info.page_size = new_size;
            cx.notify();
        });
        self.load_data_with_clauses(1, cx);
    }

    fn handle_page_change_500(&mut self, _: &Page500, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_page_size_change(500, cx)
    }

    fn handle_page_change_1000(&mut self, _: &Page1000, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_page_size_change(1000, cx)
    }

    fn handle_page_change_2000(&mut self, _: &Page2000, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_page_size_change(2000, cx)
    }

    fn handle_page_change_10000(&mut self, _: &Page10000, _: &mut Window, cx: &mut Context<Self>) {
        self.handle_page_size_change(10000, cx)
    }

    fn handle_page_change_100000(
        &mut self,
        _: &Page100000,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_page_size_change(100000, cx)
    }

    fn handle_add_row(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| state.add_row(window, cx));
    }

    fn handle_delete_row(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| {
            // 优先使用多选区对应的行；没有多选时回退到旧的单选兼容状态。
            let row_indices = collect_delete_row_indices(
                state
                    .selection()
                    .all_cells()
                    .into_iter()
                    .map(|(row_ix, _)| row_ix),
                state
                    .selected_row()
                    .or_else(|| state.selected_cell().map(|(row_ix, _)| row_ix)),
            );

            for row_ix in row_indices {
                state.delete_row(row_ix, window, cx);
            }
        });
    }

    fn handle_revert_changes(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.revert_changes(cx);
    }

    fn handle_sql_preview(&mut self, _: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.table.update(cx, |state, cx| {
            state.commit_cell_edit(window, cx);
        });
        self.show_sql_preview(window, cx);
    }

    fn handle_commit_changes(&mut self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DataGridEvent::SaveChangesRequested);
    }

    fn handle_large_text_editor(
        &mut self,
        _: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_large_text_editor(window, cx);
    }

    fn handle_open_table_designer(
        &mut self,
        _: &ClickEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(DataGridEvent::OpenTableDesignerRequested);
    }

    fn on_action_open_table_designer(
        &mut self,
        _: &OpenTableDesigner,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(DataGridEvent::OpenTableDesignerRequested);
    }

    fn on_action_open_table_query(
        &mut self,
        _: &OpenSelectedTableQuery,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.emit(DataGridEvent::OpenTableQueryRequested);
    }

    fn handle_toolbar_refresh(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_refresh(cx);
    }

    fn handle_prev_page_click(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_prev_page(cx);
    }

    fn handle_next_page_click(
        &mut self,
        _: &ClickEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_next_page(cx);
    }

    // ========== 大文本编辑器 ==========

    fn show_large_text_editor(&self, window: &mut Window, cx: &mut App) {
        let Some(target) = self.selected_large_text_target(cx) else {
            window.push_notification(t!("TableData.select_cell").to_string(), cx);
            return;
        };

        self.show_text_editor_dialog(
            TextEditorDialogRequest {
                initial_text: target.value,
                title: build_large_text_editor_title(&target.column_name, target.display_row_ix),
                row_ix: target.display_row_ix,
                col_ix: target.col_ix + 1,
                editable: target.editable,
            },
            window,
            cx,
        );
    }

    pub fn selected_large_text_target(&self, cx: &App) -> Option<LargeTextCellTarget> {
        let table = self.table.read(cx);
        let (display_row_ix, selected_col_ix) = table.selected_cell()?;
        let col_ix = selected_col_ix.checked_sub(1)?;
        let delegate = table.delegate();
        let actual_row_ix = delegate.resolve_display_row(display_row_ix)?;
        let value = delegate.editable_cell_text(actual_row_ix, col_ix);
        let column_name = delegate
            .columns
            .get(col_ix)
            .map(|col| col.name.to_string())
            .unwrap_or_else(|| {
                t!("TableDataGrid.column_label", index = selected_col_ix).to_string()
            });

        Some(LargeTextCellTarget {
            display_row_ix,
            actual_row_ix,
            col_ix,
            column_name,
            value,
            editable: self.config.editable,
        })
    }

    pub fn apply_large_text_target_value(
        &self,
        target: &LargeTextCellTarget,
        value: String,
        cx: &mut App,
    ) -> bool {
        self.table.update(cx, |state, cx| {
            let delegate = state.delegate_mut();
            let changed = delegate.record_cell_change(target.actual_row_ix, target.col_ix, value);

            if changed {
                state.refresh(cx);
            }

            changed
        })
    }

    fn show_text_editor_dialog(
        &self,
        request: TextEditorDialogRequest,
        window: &mut Window,
        cx: &mut App,
    ) {
        let dialog_text_editor =
            create_large_text_editor_with_content(Some(request.initial_text.clone()), window, cx);
        let data_grid = self.clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let editor = dialog_text_editor.clone();
            let data_grid = data_grid.clone();
            let original_text = request.initial_text.clone();

            let mut d = dialog
                .title(SharedString::from(request.title.clone()))
                .w(px(800.0))
                .h(px(600.0))
                .child(v_flex().w_full().h_full().child(editor.clone()))
                .close_button(true)
                .overlay(false)
                .content_center()
                .confirm()
                .button_props(DialogButtonProps::default().show_cancel(true));

            if request.editable {
                d = d.on_ok(move |_, window, cx| {
                    if !editor.read(cx).has_pending_writeback() {
                        return true;
                    }
                    let content = editor.read(cx).get_writeback_text(cx);
                    return match content {
                        Ok(val) => {
                            if large_text_values_equivalent(&original_text, &val) {
                                editor.update(cx, |editor, _| {
                                    editor.mark_writeback_clean();
                                });
                                return true;
                            }
                            editor.update(cx, |editor, _| {
                                editor.mark_writeback_clean();
                            });
                            data_grid.table.update(cx, |state, cx| {
                                let delegate = state.delegate_mut();
                                let Some(actual_row_ix) =
                                    delegate.resolve_display_row(request.row_ix)
                                else {
                                    return false;
                                };

                                let col_index = request.col_ix.saturating_sub(1);
                                let changed =
                                    delegate.record_cell_change(actual_row_ix, col_index, val);

                                if changed {
                                    state.refresh(cx);
                                }

                                changed
                            });
                            true
                        }
                        Err(err) => {
                            window.push_notification(
                                t!("TableDataGrid.error_message", error = err).to_string(),
                                cx,
                            );
                            false
                        }
                    };
                });
            }

            d
        });
    }

    // ========== 数据变更 ==========

    pub fn get_changes(&self, cx: &App) -> Vec<RowChange> {
        self.table.read(cx).delegate().get_changes()
    }

    pub fn column_names(&self, cx: &App) -> Vec<String> {
        self.table.read(cx).delegate().column_names()
    }

    pub fn clear_changes(&self, cx: &mut App) {
        self.table.update(cx, |state, cx| {
            state.delegate_mut().clear_changes();
            cx.notify();
        });
    }

    fn clear_changes_and_refresh(&self, cx: &mut App) {
        self.clear_changes(cx);
        self.handle_refresh(cx);
    }

    pub fn revert_changes(&self, cx: &mut App) {
        self.table.update(cx, |state, cx| {
            state.delegate_mut().revert_all_changes();
            state.refresh(cx);
            cx.notify();
        });
    }

    pub fn has_unsaved_changes(&self, cx: &App) -> bool {
        let table = self.table.read(cx);
        table_has_unsaved_changes(table.editing_cell(), table.delegate().get_changes().len())
    }

    // ========== 复制为 SQL 语句 ==========

    /// 复制选中行为 INSERT 语句
    pub fn copy_as_insert(&self, row_indices: &[usize], cx: &App) -> String {
        use db::CopySqlRequest;

        let table = self.table.read(cx);
        let delegate = table.delegate();

        let row_indices = resolve_copy_row_indices(delegate, row_indices);
        let Ok(rows_data) = delegate.get_typed_rows_data(&row_indices) else {
            return String::new();
        };
        if rows_data.is_empty() {
            return String::new();
        }

        let column_names = delegate.column_names();
        let columns_meta = delegate.column_meta().to_vec();

        let global_state = cx.global::<GlobalDbState>().clone();
        match global_state
            .db_manager
            .get_plugin(&self.config.database_type)
        {
            Ok(plugin) => {
                let mut request = CopySqlRequest::new(&self.config.table_name, columns_meta)
                    .with_typed_rows(rows_data)
                    .with_column_names(column_names);
                if let Some(schema) = &self.config.schema_name {
                    request = request.with_schema(schema);
                }
                plugin.generate_copy_insert_sql(&request)
            }
            Err(_) => String::new(),
        }
    }

    /// 复制选中行为 INSERT 语句（带字段注释）
    pub fn copy_as_insert_with_comments(&self, row_indices: &[usize], cx: &App) -> String {
        use db::CopySqlRequest;

        let table = self.table.read(cx);
        let delegate = table.delegate();

        let row_indices = resolve_copy_row_indices(delegate, row_indices);
        let Ok(rows_data) = delegate.get_typed_rows_data(&row_indices) else {
            return String::new();
        };
        if rows_data.is_empty() {
            return String::new();
        }

        let column_names = delegate.column_names();
        let columns_meta = delegate.column_meta().to_vec();

        let global_state = cx.global::<GlobalDbState>().clone();
        match global_state
            .db_manager
            .get_plugin(&self.config.database_type)
        {
            Ok(plugin) => {
                let mut request = CopySqlRequest::new(&self.config.table_name, columns_meta)
                    .with_typed_rows(rows_data)
                    .with_column_names(column_names);
                if let Some(schema) = &self.config.schema_name {
                    request = request.with_schema(schema);
                }
                plugin.generate_copy_insert_with_comments_sql(&request)
            }
            Err(_) => String::new(),
        }
    }

    /// 复制选中行为 UPDATE 语句
    pub fn copy_as_update(&self, row_indices: &[usize], cx: &App) -> String {
        use db::CopySqlRequest;

        let table = self.table.read(cx);
        let delegate = table.delegate();

        let row_indices = resolve_copy_row_indices(delegate, row_indices);
        let Ok(rows_data) = delegate.get_typed_rows_data(&row_indices) else {
            return String::new();
        };
        let Ok(original_rows) = delegate.get_typed_original_rows_data(&row_indices) else {
            return String::new();
        };
        if rows_data.is_empty() {
            return String::new();
        }

        let column_names = delegate.column_names();
        let columns_meta = delegate.column_meta().to_vec();

        let global_state = cx.global::<GlobalDbState>().clone();
        match global_state
            .db_manager
            .get_plugin(&self.config.database_type)
        {
            Ok(plugin) => {
                let mut request = CopySqlRequest::new(&self.config.table_name, columns_meta)
                    .with_typed_rows(rows_data)
                    .with_typed_original_rows(original_rows)
                    .with_column_names(column_names);
                if let Some(schema) = &self.config.schema_name {
                    request = request.with_schema(schema);
                }
                plugin.generate_copy_update_sql(&request)
            }
            Err(_) => String::new(),
        }
    }

    /// 复制选中行为 DELETE 语句
    pub fn copy_as_delete(&self, row_indices: &[usize], cx: &App) -> String {
        use db::CopySqlRequest;

        let table = self.table.read(cx);
        let delegate = table.delegate();

        let row_indices = resolve_copy_row_indices(delegate, row_indices);
        let Ok(original_rows) = delegate.get_typed_original_rows_data(&row_indices) else {
            return String::new();
        };
        if original_rows.is_empty() {
            return String::new();
        }

        let column_names = delegate.column_names();
        let columns_meta = delegate.column_meta().to_vec();

        let global_state = cx.global::<GlobalDbState>().clone();
        match global_state
            .db_manager
            .get_plugin(&self.config.database_type)
        {
            Ok(plugin) => {
                let mut request = CopySqlRequest::new(&self.config.table_name, columns_meta)
                    .with_typed_original_rows(original_rows)
                    .with_column_names(column_names);
                if let Some(schema) = &self.config.schema_name {
                    request = request.with_schema(schema);
                }
                plugin.generate_copy_delete_sql(&request)
            }
            Err(_) => String::new(),
        }
    }

    /// 获取当前选中的行索引列表
    pub fn selected_row_indices(&self, cx: &App) -> Vec<usize> {
        let table = self.table.read(cx);
        let selection = table.selection();

        if selection.is_empty() {
            // 如果没有多选，检查单选的行
            if let Some(row_ix) = table.selected_row() {
                return vec![row_ix];
            }
            return vec![];
        }

        // 从选区获取所有行
        let mut rows: Vec<usize> = selection.all_cells().iter().map(|(row, _)| *row).collect();
        rows.sort();
        rows.dedup();
        rows
    }

    /// 复制当前选中行为指定类型的 SQL
    pub fn copy_selection_as_sql(&self, sql_type: db::CopyAsSqlType, cx: &App) -> String {
        use db::CopyAsSqlType;

        let row_indices = self.selected_row_indices(cx);
        if row_indices.is_empty() {
            return String::new();
        }

        match sql_type {
            CopyAsSqlType::Insert => self.copy_as_insert(&row_indices, cx),
            CopyAsSqlType::InsertWithComments => {
                self.copy_as_insert_with_comments(&row_indices, cx)
            }
            CopyAsSqlType::Update => self.copy_as_update(&row_indices, cx),
            CopyAsSqlType::Delete => self.copy_as_delete(&row_indices, cx),
        }
    }

    pub fn convert_row_changes(
        changes: Vec<RowChange>,
        column_names: &[String],
    ) -> Vec<TableRowChange> {
        changes
            .into_iter()
            .filter_map(|change| match change {
                RowChange::Added { data } => Some(TableRowChange::Added {
                    data: data.into_iter().map(TableCellValue::from).collect(),
                }),
                RowChange::Updated {
                    original_data,
                    changes,
                    rowid,
                } => {
                    let converted: Vec<TableCellChange> = changes
                        .into_iter()
                        .map(|c| TableCellChange {
                            column_index: c.col_ix,
                            column_name: if c.col_name.is_empty() {
                                column_names.get(c.col_ix).cloned().unwrap_or_default()
                            } else {
                                c.col_name
                            },
                            old_value: TableCellValue::from(c.old_value),
                            new_value: TableCellValue::from(c.new_value),
                        })
                        .collect();

                    if converted.is_empty() {
                        None
                    } else {
                        Some(TableRowChange::Updated {
                            original_data: original_data
                                .into_iter()
                                .map(TableCellValue::from)
                                .collect(),
                            changes: converted,
                            rowid,
                        })
                    }
                }
                RowChange::Deleted {
                    original_data,
                    rowid,
                } => Some(TableRowChange::Deleted {
                    original_data: original_data
                        .into_iter()
                        .map(TableCellValue::from)
                        .collect(),
                    rowid,
                }),
            })
            .collect()
    }

    pub fn create_save_request(
        &self,
        columns: Vec<ColumnInfo>,
        index_infos: Vec<IndexInfo>,
        cx: &App,
    ) -> Option<TableSaveRequest> {
        let changes = self.get_changes(cx);
        if changes.is_empty() {
            return None;
        }

        let column_names = self.column_names(cx);
        let table_changes = Self::convert_row_changes(changes, &column_names);

        if table_changes.is_empty() {
            return None;
        }

        Some(TableSaveRequest {
            database: self.config.database_name.clone(),
            schema: self.config.schema_name.clone(),
            table: self.config.table_name.clone(),
            columns,
            index_infos,
            changes: table_changes,
        })
    }

    pub fn save_changes(&self, window: &mut Window, cx: &mut App) {
        self.table.update(cx, |state, cx| {
            state.commit_cell_edit(window, cx);
        });
        self.handle_save_changes(&ClickEvent::default(), window, cx);
    }

    pub fn save_and_close(
        &self,
        tab_container: Entity<TabContainer>,
        tab_id: String,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.table.update(cx, |state, cx| {
            state.commit_cell_edit(window, cx);
        });
        let changes = self.get_changes(cx);
        let window_handle = window.window_handle();
        if changes.is_empty() {
            let _ = cx.update_window(window_handle, |_, window, cx| {
                tab_container.update(cx, |container, cx| {
                    container.force_close_tab_by_id(&tab_id, window, cx);
                });
            });
            return;
        }

        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let table_name = self.config.table_name.clone();
        let database_type = self.config.database_type.clone();
        let this = self.clone();
        let delegate = self.table.read(cx).delegate();
        let columns = delegate.column_meta.clone();
        let has_primary_key = columns.iter().any(|c| c.is_primary_key);
        let has_rowids = delegate.has_rowids();
        let need_index_infos = !has_primary_key && !has_rowids;

        cx.spawn(async move |cx: &mut AsyncApp| {
            let mut index_infos = vec![];
            if need_index_infos {
                let index_infos_result = list_indexes_direct_on_runtime(
                    cx,
                    global_state.clone(),
                    connection_id.clone(),
                    database_name.clone(),
                    schema_name.clone(),
                    table_name.clone(),
                )
                .await;
                index_infos = match index_infos_result {
                    Ok(infos) => infos,
                    Err(err) => {
                        cx.update(|cx| {
                            let error = err.to_string();
                            let summary = t!("TableDataGrid.get_table_keys_failed", error = error)
                                .to_string();
                            notification(cx, summary);
                        });
                        return;
                    }
                };
            }

            let save_result = cx.update(|cx| {
                let Some(save_request) = this.create_save_request(columns, index_infos, cx) else {
                    return Err(t!("TableDataGrid.no_changes").to_string());
                };
                let change_count = save_request.changes.len();

                let global_state = cx.global::<GlobalDbState>().clone();
                match global_state.db_manager.get_plugin(&database_type) {
                    Ok(plugin) => {
                        let sql = plugin.generate_table_changes_sql(&save_request);
                        let trimmed = sql.trim();
                        if trimmed.is_empty()
                            || trimmed == t!("TableDataGrid.no_changes_sql_marker")
                        {
                            Err(t!("TableDataGrid.no_changes").to_string())
                        } else {
                            Ok((sql, change_count))
                        }
                    }
                    Err(_) => Err(t!("TableDataGrid.plugin_unavailable").to_string()),
                }
            });

            let (sql_content, change_count) = match save_result {
                Ok((sql, count)) => (sql, count),
                Err(msg) => {
                    cx.update(|cx| {
                        notification(cx, msg);
                    });
                    return;
                }
            };

            let exec_options = ExecOptions {
                stop_on_error: true,
                transactional: true,
                max_rows: None,
                streaming: false,
            };

            let result = global_state
                .execute_script(
                    cx,
                    connection_id.clone(),
                    sql_content.clone(),
                    Some(database_name.clone()),
                    None,
                    Some(exec_options),
                )
                .await;

            cx.update(|cx| match result {
                Ok(results) => {
                    let sql = sql_content.clone();
                    let first_error = first_execution_error(&results).map(str::to_owned);
                    let succeeded = first_error.is_none();
                    this.record_execution_results(sql, &results, cx);
                    let summary = first_error
                        .map(|error| {
                            t!("TableDataGrid.save_changes_failed", error = error).to_string()
                        })
                        .unwrap_or_else(|| {
                            t!("TableDataGrid.save_changes_success", count = change_count)
                                .to_string()
                        });

                    if succeeded {
                        this.clear_changes(cx);
                        let _ = cx.update_window(window_handle, |_, window, cx| {
                            tab_container.update(cx, |container, cx| {
                                container.force_close_tab_by_id(&tab_id, window, cx);
                            });
                        });
                    }
                    notification(cx, summary);
                }
                Err(error) => {
                    let error = error.to_string();
                    let summary =
                        t!("TableDataGrid.save_changes_failed", error = error).to_string();
                    this.record_execution_failure(error, Some(sql_content), cx);
                    notification(cx, summary);
                }
            });
        })
        .detach();
    }

    fn handle_save_changes(&self, _: &ClickEvent, _window: &mut Window, cx: &mut App) {
        let changes = self.get_changes(cx);
        if changes.is_empty() {
            return;
        }

        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let table_name = self.config.table_name.clone();
        let database_type = self.config.database_type.clone();
        let this = self.clone();
        let delegate = self.table.read(cx).delegate();
        let columns = delegate.column_meta.clone();
        let has_primary_key = columns.iter().any(|c| c.is_primary_key);
        let has_rowids = delegate.has_rowids();
        let need_index_infos = !has_primary_key && !has_rowids;

        cx.spawn(async move |cx: &mut AsyncApp| {
            let mut index_infos = vec![];
            if need_index_infos {
                let index_infos_result = list_indexes_direct_on_runtime(
                    cx,
                    global_state.clone(),
                    connection_id.clone(),
                    database_name.clone(),
                    schema_name.clone(),
                    table_name.clone(),
                )
                .await;
                index_infos = match index_infos_result {
                    Ok(infos) => infos,
                    Err(err) => {
                        cx.update(|cx| {
                            let error = err.to_string();
                            let summary = t!("TableDataGrid.get_table_keys_failed", error = error)
                                .to_string();
                            notification(cx, summary);
                        });
                        return;
                    }
                };
            }

            let save_result = cx.update(|cx| {
                let Some(save_request) = this.create_save_request(columns, index_infos, cx) else {
                    return Err(t!("TableDataGrid.no_changes").to_string());
                };
                let change_count = save_request.changes.len();

                let global_state = cx.global::<GlobalDbState>().clone();
                match global_state.db_manager.get_plugin(&database_type) {
                    Ok(plugin) => {
                        let sql = plugin.generate_table_changes_sql(&save_request);
                        let trimmed = sql.trim();
                        if trimmed.is_empty()
                            || trimmed == t!("TableDataGrid.no_changes_sql_marker")
                        {
                            Err(t!("TableDataGrid.no_changes").to_string())
                        } else {
                            Ok((sql, change_count))
                        }
                    }
                    Err(_) => Err(t!("TableDataGrid.plugin_unavailable").to_string()),
                }
            });

            let (sql_content, change_count) = match save_result {
                Ok((sql, count)) => (sql, count),
                Err(msg) => {
                    cx.update(|cx| {
                        notification(cx, msg);
                    });
                    return;
                }
            };

            let exec_options = ExecOptions {
                stop_on_error: true,
                transactional: true,
                max_rows: None,
                streaming: false,
            };

            let result = global_state
                .execute_script(
                    cx,
                    connection_id.clone(),
                    sql_content.clone(),
                    Some(database_name.clone()),
                    None,
                    Some(exec_options),
                )
                .await;

            cx.update(|cx| match result {
                Ok(results) => {
                    let sql = sql_content.clone();
                    let first_error = first_execution_error(&results).map(str::to_owned);
                    let succeeded = first_error.is_none();
                    this.record_execution_results(sql, &results, cx);
                    let summary = first_error
                        .map(|error| {
                            t!("TableDataGrid.save_changes_failed", error = error).to_string()
                        })
                        .unwrap_or_else(|| {
                            t!("TableDataGrid.save_changes_success", count = change_count)
                                .to_string()
                        });

                    if succeeded {
                        this.clear_changes_and_refresh(cx);
                    }
                    notification(cx, summary);
                }
                Err(error) => {
                    let error = error.to_string();
                    let summary =
                        t!("TableDataGrid.save_changes_failed", error = error).to_string();
                    this.record_execution_failure(error, Some(sql_content), cx);
                    notification(cx, summary);
                }
            });
        })
        .detach();
    }

    pub fn show_sql_preview(&self, window: &mut Window, cx: &mut App) {
        let changes = self.get_changes(cx);
        if changes.is_empty() {
            window.push_notification(t!("TableDataGrid.no_changes").to_string(), cx);
            return;
        }

        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let table_name = self.config.table_name.clone();
        let this = self.clone();
        let delegate = self.table.read(cx).delegate();
        let columns = delegate.column_meta.clone();
        let has_primary_key = columns.iter().any(|c| c.is_primary_key);
        let has_rowids = delegate.has_rowids();
        let need_index_infos = !has_primary_key && !has_rowids;

        cx.spawn(async move |cx: &mut AsyncApp| {
            let mut index_infos = vec![];
            if need_index_infos {
                let index_infos_result = list_indexes_direct_on_runtime(
                    cx,
                    global_state,
                    connection_id,
                    database_name,
                    schema_name,
                    table_name,
                )
                .await;
                index_infos = match index_infos_result {
                    Ok(index_infos) => index_infos,
                    Err(err) => {
                        cx.update(|cx| {
                            let error = err.to_string();
                            let summary = t!("TableDataGrid.get_table_keys_failed", error = error)
                                .to_string();
                            notification(cx, summary);
                        });
                        return;
                    }
                };
            }

            cx.update(|cx| {
                if let Some(window_id) = cx.active_window() {
                    let _ = cx.update_window(window_id, |_entity, window, cx| {
                        let Some(save_request) =
                            this.create_save_request(columns.clone(), index_infos.clone(), cx)
                        else {
                            window
                                .push_notification(t!("TableDataGrid.no_changes").to_string(), cx);
                            return;
                        };

                        let sql_content = match this.build_changes_sql(&save_request, cx) {
                            Ok(sql) => sql,
                            Err(message) => {
                                window.push_notification(message, cx);
                                return;
                            }
                        };

                        this.show_sql_editor_dialog(
                            sql_content,
                            t!("TableDataGrid.change_sql_preview").as_ref(),
                            window,
                            cx,
                        );
                    });
                }
            });
        })
        .detach();
    }

    pub fn show_sql_editor_dialog(
        &self,
        initial_sql: String,
        title: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        let sql_editor = cx.new(|cx| SqlEditor::new(window, cx));
        sql_editor.update(cx, |editor, cx| {
            editor.set_value(initial_sql, window, cx);
        });

        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let this = self.clone();
        let title = title.to_string();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let editor = sql_editor.clone();
            let execute_state = global_state.clone();
            let execute_connection = connection_id.clone();
            let execute_database = database_name.clone();
            let data_grid = this.clone();

            dialog
                .title(SharedString::from(title.clone()))
                .w(px(800.0))
                .h(px(600.0))
                .child(v_flex().w_full().h_full().child(editor.clone()))
                .close_button(true)
                .overlay(false)
                .content_center()
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("TableDataGrid.execute_sql").to_string())
                        .show_cancel(true),
                )
                .on_ok(move |_, window, cx| {
                    let sql_text = editor.read(cx).get_text(cx);
                    if sql_text.trim().is_empty() {
                        window.push_notification(t!("TableDataGrid.sql_empty").to_string(), cx);
                        return false;
                    }
                    data_grid.execute_sql_and_refresh(
                        sql_text,
                        execute_state.clone(),
                        execute_connection.clone(),
                        execute_database.clone(),
                        window,
                        cx,
                    );
                    false
                })
        });
    }

    async fn execute_sql_and_refresh_async(
        sql: String,
        global_state: GlobalDbState,
        connection_id: String,
        database_name: String,
        cx: &mut AsyncApp,
    ) -> Result<Vec<SqlResult>, String> {
        let exec_options = ExecOptions {
            stop_on_error: true,
            transactional: true,
            max_rows: None,
            streaming: false,
        };

        global_state
            .execute_script(
                cx,
                connection_id,
                sql,
                Some(database_name),
                None,
                Some(exec_options),
            )
            .await
            .map_err(|error| error.to_string())
    }

    fn execute_sql_and_refresh(
        &self,
        sql: String,
        global_state: GlobalDbState,
        connection_id: String,
        database_name: String,
        _window: &mut Window,
        cx: &mut App,
    ) {
        let data_grid = self.clone();

        cx.spawn(async move |cx: &mut AsyncApp| {
            let executed_sql = sql.clone();
            match Self::execute_sql_and_refresh_async(
                sql,
                global_state,
                connection_id,
                database_name,
                cx,
            )
            .await
            {
                Ok(results) => {
                    cx.update(|cx| {
                        let first_error = first_execution_error(&results).map(str::to_owned);
                        let succeeded = first_error.is_none();
                        data_grid.record_execution_results(executed_sql.clone(), &results, cx);
                        let summary = first_error
                            .map(|error| {
                                t!("TableDataGrid.execute_failed", error = error).to_string()
                            })
                            .unwrap_or_else(|| t!("TableDataGrid.execute_success").to_string());

                        if succeeded {
                            data_grid.clear_changes_and_refresh(cx);
                            if let Some(window_id) = cx.active_window() {
                                let _ = cx.update_window(window_id, |_entity, window, cx| {
                                    window.close_dialog(cx);
                                });
                            }
                        }
                        notification(cx, summary);
                    });
                }
                Err(error) => {
                    cx.update(|cx| {
                        let summary = t!("TableDataGrid.execute_failed", error = error).to_string();
                        data_grid.record_execution_failure(error, Some(executed_sql), cx);
                        notification(cx, summary);
                    });
                }
            }
        })
        .detach();
    }

    fn build_changes_sql(&self, request: &TableSaveRequest, cx: &App) -> Result<String, String> {
        let global_state = cx.global::<GlobalDbState>().clone();
        match global_state
            .db_manager
            .get_plugin(&self.config.database_type)
        {
            Ok(plugin) => {
                let sql = plugin.generate_table_changes_sql(request);
                let trimmed = sql.trim();
                if trimmed.is_empty() || trimmed == t!("TableDataGrid.no_changes_sql_marker") {
                    Err(t!("TableDataGrid.no_changes").to_string())
                } else {
                    let format_options =
                        SqlFormatOptions::from_settings(&cx.global::<AppSettings>().sql_format);
                    Ok(plugin.format_sql_with_options(&sql, format_options))
                }
            }
            Err(_) => Err(t!("TableDataGrid.plugin_unavailable").to_string()),
        }
    }

    // ========== 渲染辅助方法 ==========

    pub fn render_toolbar(&self, _window: &mut Window, cx: &Context<Self>) -> AnyElement {
        let editable = self.config.editable;
        let loading = self.table.read(cx).delegate().is_loading();
        let data_grid = cx.entity().clone();
        let settings = cx.global::<AppSettings>();
        let large_text_button_selected = settings.large_text_cell_editor_open_mode
            == LargeTextCellEditorOpenMode::SidebarPreview
            && self.is_large_text_editor_sidebar_open;

        h_flex()
            .gap_1()
            .items_center()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                Button::new("refresh-data")
                    .with_size(Size::Medium)
                    .icon(IconName::Refresh)
                    .tooltip(t!("TableDataGrid.refresh").to_string())
                    .disabled(loading)
                    .on_click(cx.listener(Self::handle_toolbar_refresh)),
            )
            .when(editable, |this| {
                this.child(
                    Button::new("add-row")
                        .with_size(Size::Medium)
                        .icon(IconName::Plus)
                        .tooltip(t!("TableDataGrid.add_row").to_string())
                        .disabled(loading)
                        .on_click(cx.listener(Self::handle_add_row)),
                )
            })
            .when(editable, |this| {
                this.child(
                    Button::new("delete-row")
                        .with_size(Size::Medium)
                        .icon(IconName::Minus)
                        .tooltip(t!("TableDataGrid.delete_row").to_string())
                        .disabled(loading)
                        .on_click(cx.listener(Self::handle_delete_row)),
                )
            })
            .when(editable, |this| {
                this.child(
                    Button::new("undo-changes")
                        .with_size(Size::Medium)
                        .icon(IconName::Undo)
                        .tooltip(t!("TableDataGrid.undo").to_string())
                        .disabled(loading)
                        .on_click(cx.listener(Self::handle_revert_changes)),
                )
            })
            .when(editable, |this| {
                this.child(
                    Button::new("sql-preview")
                        .with_size(Size::Medium)
                        .icon(IconName::Eye)
                        .tooltip(t!("TableDataGrid.sql_preview").to_string())
                        .disabled(loading)
                        .on_click(cx.listener(Self::handle_sql_preview)),
                )
            })
            .when(editable, |this| {
                this.child(
                    Button::new("commit-changes")
                        .with_size(Size::Medium)
                        .icon(IconName::ArrowUp)
                        .tooltip(t!("TableDataGrid.commit_changes").to_string())
                        .disabled(loading)
                        .on_click(cx.listener(Self::handle_commit_changes)),
                )
            })
            .child(div().flex_1())
            .when(self.config.usage == DataGridUsage::TableData, |this| {
                this.child(
                    div().w(px(220.)).child(
                        Input::new(&self.search_input)
                            .prefix(
                                Icon::new(IconName::Search).text_color(cx.theme().muted_foreground),
                            )
                            .cleanable(true)
                            .small()
                            .w_full(),
                    ),
                )
            })
            .when(
                self.config.usage == DataGridUsage::TableData && editable,
                |this| {
                    this.child(
                        Button::new("open-table-designer")
                            .with_size(Size::Medium)
                            .icon(IconName::TableDesignTool)
                            .tooltip(t!("TableDataGrid.open_table_designer").to_string())
                            .disabled(loading)
                            .on_click(cx.listener(Self::handle_open_table_designer)),
                    )
                },
            )
            .child(
                Button::new("toggle-editor")
                    .with_size(Size::Medium)
                    .icon(IconName::EditBorder)
                    .when(large_text_button_selected, |this| this.primary())
                    .tooltip(t!("TableDataGrid.large_text_editor").to_string())
                    .disabled(loading)
                    .on_click(cx.listener(Self::handle_large_text_editor)),
            )
            .child(
                Button::new("export-data")
                    .with_size(Size::Medium)
                    .icon(IconName::Export)
                    .tooltip(t!("TableDataGrid.export").to_string())
                    .disabled(loading)
                    .dropdown_menu(move |menu, window, _cx| {
                        menu.item(
                            PopupMenuItem::new(t!("TableDataGrid.export_result_xlsx").to_string())
                                .on_click(window.listener_for(
                                    &data_grid,
                                    |this, _, window, cx| {
                                        this.export_result_set(
                                            ExportScope::All,
                                            ExportFormat::Xlsx,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                        )
                        .item(
                            PopupMenuItem::new(t!("TableDataGrid.export_result_csv").to_string())
                                .on_click(window.listener_for(
                                    &data_grid,
                                    |this, _, window, cx| {
                                        this.export_result_set(
                                            ExportScope::All,
                                            ExportFormat::Csv,
                                            window,
                                            cx,
                                        );
                                    },
                                )),
                        )
                        .item(
                            PopupMenuItem::new(
                                t!("TableDataGrid.export_result_insert_sql").to_string(),
                            )
                            .on_click(window.listener_for(
                                &data_grid,
                                |this, _, window, cx| {
                                    this.export_result_set(
                                        ExportScope::All,
                                        ExportFormat::InsertSql,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .separator()
                        .item(
                            PopupMenuItem::new(
                                t!("TableDataGrid.export_current_page_xlsx").to_string(),
                            )
                            .on_click(window.listener_for(
                                &data_grid,
                                |this, _, window, cx| {
                                    this.export_result_set(
                                        ExportScope::CurrentPage,
                                        ExportFormat::Xlsx,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .item(
                            PopupMenuItem::new(
                                t!("TableDataGrid.export_current_page_csv").to_string(),
                            )
                            .on_click(window.listener_for(
                                &data_grid,
                                |this, _, window, cx| {
                                    this.export_result_set(
                                        ExportScope::CurrentPage,
                                        ExportFormat::Csv,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                        .item(
                            PopupMenuItem::new(
                                t!("TableDataGrid.export_current_page_insert_sql").to_string(),
                            )
                            .on_click(window.listener_for(
                                &data_grid,
                                |this, _, window, cx| {
                                    this.export_result_set(
                                        ExportScope::CurrentPage,
                                        ExportFormat::InsertSql,
                                        window,
                                        cx,
                                    );
                                },
                            )),
                        )
                    }),
            )
            .into_any_element()
    }

    pub fn render_table_area(&self, _window: &mut Window, cx: &App) -> AnyElement {
        let error_message = self.table_data_info.read(cx).error_message.clone();

        if let Some(error) = error_message {
            return div()
                .flex_1()
                .w_full()
                .h_full()
                .bg(cx.theme().background)
                .border_1()
                .border_color(cx.theme().border)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_color(cx.theme().danger_foreground)
                        .text_sm()
                        .child(error),
                )
                .into_any_element();
        }

        let table_view = EditTable::new(&self.table);
        div()
            .flex_1()
            .w_full()
            .h_full()
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .child(table_view)
            .into_any_element()
    }

    fn render_status_bar(&self, cx: &Context<Self>) -> AnyElement {
        let table_data_info = self.table_data_info.read(cx);
        let table = self.table.read(cx);

        let filtered_count = table.delegate().filtered_row_count();
        let total_rows = table.delegate().rows.len();
        let current_page_size = table_data_info.page_size;

        h_flex()
            .gap_3()
            .items_center()
            .px_2()
            .py_1()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(div().text_sm().text_color(cx.theme().foreground).child({
                if filtered_count < total_rows {
                    t!(
                        "TableDataGrid.page_info",
                        page_size = filtered_count,
                        page_count = total_rows,
                        total_count = table_data_info.total_count
                    )
                    .to_string()
                } else {
                    t!(
                        "TableDataGrid.page_number",
                        page = table_data_info.current_page,
                        total = table_data_info.total_count
                    )
                    .to_string()
                }
            }))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        t!(
                            "TableDataGrid.query_elapsed",
                            duration = table_data_info.duration
                        )
                        .to_string(),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(table_data_info.current_sql.clone()),
            )
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .child(
                        Button::new("prev-page")
                            .with_size(Size::Small)
                            .icon(IconName::ChevronLeft)
                            .on_click(cx.listener(Self::handle_prev_page_click)),
                    )
                    .child({
                        let label = match current_page_size {
                            0 => t!("TableDataGrid.all").to_string(),
                            n => format!("{}", n),
                        };

                        Button::new("page-size-selector")
                            .with_size(Size::Small)
                            .label(label)
                            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                                menu.menu("500", Box::new(Page500))
                                    .menu("1000", Box::new(Page1000))
                                    .menu("2000", Box::new(Page2000))
                                    .menu("10000", Box::new(Page10000))
                                    .menu("100000", Box::new(Page100000))
                            })
                    })
                    .child(
                        Button::new("next-page")
                            .with_size(Size::Small)
                            .icon(IconName::ChevronRight)
                            .on_click(cx.listener(Self::handle_next_page_click)),
                    ),
            )
            .into_any_element()
    }

    fn render_simple_status_bar(&self, cx: &App) -> AnyElement {
        let row_count = self.config.rows_count;
        // 将SQL中的换行符替换为空格，保持单行显示
        let sql = self.config.sql.replace(['\n', '\r'], " ");
        let execution_time = self.config.execution_time;

        h_flex()
            .gap_3()
            .items_center()
            .px_2()
            .py_1()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(t!("TableDataGrid.total_records", count = row_count).to_string()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        t!("TableDataGrid.query_elapsed", duration = execution_time).to_string(),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .flex_1()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(sql),
            )
            .into_any_element()
    }
}

impl Render for DataGrid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_table_data = self.config.usage == DataGridUsage::TableData;

        v_flex()
            .when(is_table_data, |this| {
                this.on_action(cx.listener(Self::handle_page_change_500))
                    .on_action(cx.listener(Self::handle_page_change_1000))
                    .on_action(cx.listener(Self::handle_page_change_2000))
                    .on_action(cx.listener(Self::handle_page_change_10000))
                    .on_action(cx.listener(Self::handle_page_change_100000))
                    .on_action(cx.listener(Self::on_action_focus_search))
                    .on_action(cx.listener(Self::on_action_open_table_designer))
                    .on_action(cx.listener(Self::on_action_open_table_query))
                    .key_context(DB_SEARCH_CONTEXT)
            })
            .size_full()
            .gap_0()
            .track_focus(&self.focus_handle)
            .child(self.render_toolbar(window, cx))
            .when(is_table_data, |this| {
                this.child(
                    h_flex()
                        .items_center()
                        .w_full()
                        .px_2()
                        .py_1()
                        .child(self.filter_editor.clone()),
                )
            })
            .child(
                div()
                    .flex_1()
                    .w_full()
                    .h_full()
                    .overflow_hidden()
                    .child(self.render_table_area(window, cx)),
            )
            .child(if is_table_data {
                self.render_status_bar(cx)
            } else {
                self.render_simple_status_bar(cx)
            })
    }
}

impl Clone for DataGrid {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            table: self.table.clone(),
            _table_sub: None,
            focus_handle: self.focus_handle.clone(),
            table_data_info: self.table_data_info.clone(),
            filter_editor: self.filter_editor.clone(),
            _filter_sub: None,
            search_input: self.search_input.clone(),
            _search_sub: None,
            execution_history: self.execution_history.clone(),
            is_large_text_editor_sidebar_open: self.is_large_text_editor_sidebar_open,
            data_generation: self.data_generation.clone(),
        }
    }
}

impl EventEmitter<DataGridEvent> for DataGrid {}

impl Focusable for DataGrid {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[inline]
pub fn notification(cx: &mut App, error: String) {
    if let Some(window) = cx.active_window() {
        _ = window.update(cx, |_, w, cx| w.push_notification(error, cx));
    };
}

#[cfg(test)]
mod tests {
    use super::{
        DataGrid, ExportFormat, LargeTextEditorRoute, TableMetadata, build_header_order_by_clause,
        build_large_text_editor_title, collect_delete_row_indices, column_infos_from_query_meta,
        data_generation_is_current, query_result_for_export, resolve_large_text_editor_route,
        result_set_export_exec_options, table_has_unsaved_changes,
    };
    use crate::table_data::results_delegate::{CellChange, RowChange};
    use db::{
        BinaryCell, ColumnInfo, DbManager, ExecResult, QueryColumnMeta, QueryResult, SqlErrorInfo,
        SqlResult, TableCellValue, TableRowChange,
    };
    use gpui::SharedString;
    use one_core::settings::LargeTextCellEditorOpenMode;
    use one_core::storage::DatabaseType;
    use one_ui::edit_table::ColumnSort;
    use rust_i18n::t;
    use std::io::{Cursor, Read};
    use std::sync::atomic::{AtomicU64, Ordering};
    use zip::ZipArchive;

    #[test]
    fn data_generation_rejects_stale_async_callbacks() {
        let generation = AtomicU64::new(7);

        assert!(data_generation_is_current(&generation, 7));
        generation.fetch_add(1, Ordering::AcqRel);
        assert!(!data_generation_is_current(&generation, 7));
        assert!(data_generation_is_current(&generation, 8));
    }

    #[test]
    fn column_infos_from_query_meta_preserves_result_order_and_types() {
        // 结果集自带列元数据必须按结果列序映射，并保留 db_type 以识别时间列。
        let column_infos = column_infos_from_query_meta(&[
            QueryColumnMeta::new("name", "VARCHAR(100)"),
            QueryColumnMeta::new("created_at", "DATE"),
            QueryColumnMeta::new("updated_at", "DATETIME"),
            QueryColumnMeta::new("started_at", "TIME"),
            QueryColumnMeta::new("amount", "DECIMAL(10,2)"),
        ]);

        assert_eq!(
            column_infos
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["name", "created_at", "updated_at", "started_at", "amount"]
        );
        assert_eq!(column_infos[1].data_type, "DATE");
        assert_eq!(column_infos[2].data_type, "DATETIME");
        assert_eq!(column_infos[3].data_type, "TIME");
        assert!(!column_infos[0].is_primary_key);
        assert_eq!(column_infos[0].is_nullable, true);
        assert_eq!(column_infos[0].charset, None);
    }

    fn sample_export_input() -> (Vec<Vec<Option<String>>>, Vec<SharedString>, TableMetadata) {
        let rows = vec![vec![
            Some("1".to_string()),
            Some("Line 1\nLine 2".to_string()),
        ]];
        let columns = vec![SharedString::from("id"), SharedString::from("body")];
        let metadata = TableMetadata::new("news").with_columns(vec!["id", "body"]);
        (rows, columns, metadata)
    }

    fn sample_query_result(row_count: usize) -> QueryResult {
        QueryResult {
            sql: "select id from users".to_string(),
            columns: vec!["id".to_string()],
            column_meta: vec![],
            rows: (0..row_count)
                .map(|row| vec![Some(row.to_string())])
                .collect(),
            binary_cells: vec![],
            elapsed_ms: 0,
        }
    }

    #[test]
    fn result_set_export_disables_query_row_limit() {
        assert_eq!(None, result_set_export_exec_options().max_rows);
    }

    #[test]
    fn query_result_export_preserves_all_rows() {
        let payload =
            query_result_for_export(vec![SqlResult::Query(sample_query_result(1001))]).unwrap();
        assert_eq!(1001, payload.rows.len());
        assert!(payload.binary_cells.is_empty());
    }

    #[test]
    fn query_result_export_preserves_binary_sidecar() {
        let mut result = sample_query_result(1);
        result.rows[0][0] = Some("wrong".to_string());
        result.binary_cells = vec![BinaryCell {
            row_index: 0,
            column_index: 0,
            bytes: vec![1, 2, 3],
        }];

        let payload = query_result_for_export(vec![SqlResult::Query(result)]).unwrap();

        assert_eq!(payload.rows, vec![vec![Some("wrong".to_string())]]);
        assert_eq!(payload.binary_cells.len(), 1);
        assert_eq!(payload.binary_cells[0].bytes, vec![1, 2, 3]);
    }

    #[test]
    fn normalize_query_result_remaps_binary_sidecar_after_rowid_column() {
        let result = QueryResult {
            sql: "select __rowid__, payload from sample".to_string(),
            columns: vec!["__rowid__".to_string(), "payload".to_string()],
            column_meta: vec![],
            rows: vec![vec![Some("row-1".to_string()), Some("wrong".to_string())]],
            binary_cells: vec![
                BinaryCell {
                    row_index: 0,
                    column_index: 0,
                    bytes: vec![0xaa],
                },
                BinaryCell {
                    row_index: 0,
                    column_index: 1,
                    bytes: vec![1, 2, 3],
                },
            ],
            elapsed_ms: 0,
        };

        let payload = DataGrid::normalize_query_result(result);

        assert_eq!(payload.columns, vec![SharedString::from("payload")]);
        assert_eq!(payload.rows, vec![vec![Some("wrong".to_string())]]);
        assert_eq!(payload.binary_cells.len(), 1);
        assert_eq!(payload.binary_cells[0].row_index, 0);
        assert_eq!(payload.binary_cells[0].column_index, 0);
        assert_eq!(payload.binary_cells[0].bytes, vec![1, 2, 3]);
    }

    #[test]
    fn query_result_export_rejects_non_query_and_ambiguous_results() {
        let exec = SqlResult::Exec(ExecResult {
            sql: "update users set active = true".to_string(),
            rows_affected: 1,
            elapsed_ms: 0,
            message: None,
        });
        assert!(query_result_for_export(vec![exec]).is_err());

        let database_error = "database unavailable";
        let error = SqlResult::Error(SqlErrorInfo {
            sql: "select id from users".to_string(),
            message: database_error.to_string(),
        });
        assert!(
            query_result_for_export(vec![error])
                .unwrap_err()
                .contains(database_error)
        );

        assert!(query_result_for_export(vec![]).is_err());
        assert!(
            query_result_for_export(vec![
                SqlResult::Query(sample_query_result(1)),
                SqlResult::Query(sample_query_result(1)),
            ])
            .is_err()
        );
    }

    #[test]
    fn table_has_unsaved_changes_when_cell_is_still_editing() {
        assert!(table_has_unsaved_changes(Some((0, 1)), 0));
    }

    #[test]
    fn table_has_unsaved_changes_when_delegate_has_pending_changes() {
        assert!(table_has_unsaved_changes(None, 1));
    }

    #[test]
    fn table_has_no_unsaved_changes_without_editing_or_pending_changes() {
        assert!(!table_has_unsaved_changes(None, 0));
    }

    #[test]
    fn convert_row_changes_preserves_null_separately_from_empty_string() {
        let changes = vec![RowChange::Updated {
            original_data: vec![
                Some("42".to_string()),
                None,
                Some(String::new()),
                Some("NULL".to_string()),
            ],
            changes: vec![
                CellChange {
                    col_ix: 1,
                    col_name: "score".to_string(),
                    old_value: Some("7".to_string()),
                    new_value: None,
                },
                CellChange {
                    col_ix: 3,
                    col_name: "note".to_string(),
                    old_value: Some("old".to_string()),
                    new_value: Some("NULL".to_string()),
                },
            ],
            rowid: None,
        }];

        let converted =
            DataGrid::convert_row_changes(changes, &["id".to_string(), "score".to_string()]);

        let TableRowChange::Updated {
            original_data,
            changes,
            ..
        } = &converted[0]
        else {
            panic!("expected an updated row");
        };
        assert_eq!(
            original_data,
            &[
                TableCellValue::Text("42".to_string()),
                TableCellValue::Null,
                TableCellValue::Text(String::new()),
                TableCellValue::Text("NULL".to_string()),
            ]
        );
        assert_eq!(changes[0].old_value, TableCellValue::Text("7".to_string()));
        assert_eq!(changes[0].new_value, TableCellValue::Null);
        assert_eq!(
            changes[1].new_value,
            TableCellValue::Text("NULL".to_string())
        );
    }

    #[test]
    fn convert_added_and_deleted_rows_preserve_all_cell_value_states() {
        let values = vec![None, Some(String::new()), Some("NULL".to_string())];
        let converted = DataGrid::convert_row_changes(
            vec![
                RowChange::Added {
                    data: values.clone(),
                },
                RowChange::Deleted {
                    original_data: values,
                    rowid: None,
                },
            ],
            &[],
        );
        let expected = vec![
            TableCellValue::Null,
            TableCellValue::Text(String::new()),
            TableCellValue::Text("NULL".to_string()),
        ];

        let TableRowChange::Added { data } = &converted[0] else {
            panic!("expected an added row");
        };
        assert_eq!(data, &expected);

        let TableRowChange::Deleted { original_data, .. } = &converted[1] else {
            panic!("expected a deleted row");
        };
        assert_eq!(original_data, &expected);
    }

    fn read_xlsx_entry(bytes: &[u8], entry_name: &str) -> String {
        let mut archive =
            ZipArchive::new(Cursor::new(bytes)).expect("xlsx bytes should be a valid zip archive");
        let mut entry = archive
            .by_name(entry_name)
            .unwrap_or_else(|_| panic!("missing xlsx entry: {entry_name}"));
        let mut content = String::new();
        entry
            .read_to_string(&mut content)
            .expect("xlsx entry should be valid utf-8 xml");
        content
    }

    #[test]
    fn build_header_order_by_clause_quotes_mysql_identifier() {
        let clause = build_header_order_by_clause(
            &DbManager::default(),
            DatabaseType::MySQL,
            "order",
            ColumnSort::Descending,
        )
        .expect("should build order by clause");

        assert_eq!(clause.as_deref(), Some("`order` DESC"));
    }

    #[test]
    fn build_header_order_by_clause_quotes_postgresql_identifier() {
        let clause = build_header_order_by_clause(
            &DbManager::default(),
            DatabaseType::PostgreSQL,
            "created_at",
            ColumnSort::Ascending,
        )
        .expect("should build order by clause");

        assert_eq!(clause.as_deref(), Some("\"created_at\" ASC"));
    }

    #[test]
    fn build_header_order_by_clause_clears_default_sort() {
        let clause = build_header_order_by_clause(
            &DbManager::default(),
            DatabaseType::SQLite,
            "id",
            ColumnSort::Default,
        )
        .expect("default sort should not require a clause");

        assert!(clause.is_none());
    }

    #[test]
    fn collect_delete_row_indices_dedups_and_sorts_descending() {
        let rows = collect_delete_row_indices(vec![2, 4, 2, 1, 4], None);

        assert_eq!(rows, vec![4, 2, 1]);
    }

    #[test]
    fn collect_delete_row_indices_uses_fallback_when_selection_is_empty() {
        let rows = collect_delete_row_indices(Vec::<usize>::new(), Some(3));

        assert_eq!(rows, vec![3]);
    }

    #[test]
    fn collect_delete_row_indices_prefers_selection_over_fallback() {
        let rows = collect_delete_row_indices(vec![1, 0], Some(5));

        assert_eq!(rows, vec![1, 0]);
    }

    #[test]
    fn build_large_text_editor_title_formats_row_and_column() {
        let title = build_large_text_editor_title("payload", 2);

        assert_eq!(
            title,
            t!("TableDataGrid.edit_cell_title", column = "payload", row = 3).to_string()
        );
    }

    #[test]
    fn resolve_large_text_editor_route_uses_page_preview_for_sidebar_mode() {
        let route = resolve_large_text_editor_route(LargeTextCellEditorOpenMode::SidebarPreview);

        assert_eq!(route, LargeTextEditorRoute::PagePreview);
    }

    #[test]
    fn resolve_large_text_editor_route_uses_dialog_for_dialog_mode() {
        let route = resolve_large_text_editor_route(LargeTextCellEditorOpenMode::Dialog);

        assert_eq!(route, LargeTextEditorRoute::Dialog);
    }

    #[test]
    fn dialogs_bind_callbacks_and_button_props() {
        let source = include_str!("data_grid.rs");
        assert_dialog_callback_and_buttons(source, "fn show_text_editor_dialog(");
        assert_dialog_callback_and_buttons(source, "fn show_sql_editor_dialog(");
    }

    fn assert_dialog_callback_and_buttons(source: &str, marker: &str) {
        let start = source.find(marker).expect("function exists");
        let rest = &source[start..];
        let end = rest.find("\n    fn ").unwrap_or(rest.len());
        let body = &rest[..end];
        let on_ok = body.find(".on_ok(").expect("function binds ok callback");
        let buttons = body
            .find(".button_props(")
            .expect("function configures dialog buttons");
        assert_ne!(on_ok, buttons);
    }

    #[test]
    fn build_export_bytes_creates_zip_based_xlsx_payload() {
        let (rows, columns, metadata) = sample_export_input();

        let bytes = super::build_export_bytes(
            ExportFormat::Xlsx,
            super::ExportBuildInput {
                rows,
                binary_cells: vec![],
                columns,
                metadata,
                plugin: None,
            },
        )
        .expect("xlsx export should build successfully")
        .expect("xlsx export should produce bytes");

        assert!(bytes.starts_with(b"PK"));
        let sheet_xml = read_xlsx_entry(&bytes, "xl/worksheets/sheet1.xml");
        assert!(sheet_xml.contains("<sheetData>"));
    }

    #[test]
    fn build_export_bytes_preserves_newlines_in_xlsx_cells() {
        let (rows, columns, metadata) = sample_export_input();

        let bytes = super::build_export_bytes(
            ExportFormat::Xlsx,
            super::ExportBuildInput {
                rows,
                binary_cells: vec![],
                columns,
                metadata,
                plugin: None,
            },
        )
        .expect("xlsx export should build successfully")
        .expect("xlsx export should produce bytes");

        let shared_strings = read_xlsx_entry(&bytes, "xl/sharedStrings.xml");
        assert!(
            shared_strings.contains("Line 1\nLine 2")
                || shared_strings.contains("Line 1&#10;Line 2")
                || shared_strings.contains("Line 1&#xA;Line 2")
        );
    }

    #[test]
    fn build_export_bytes_preserves_null_empty_and_literal_null_text() {
        let rows = vec![vec![None, Some(String::new()), Some("NULL".to_string())]];
        let columns = vec!["nullable".into(), "empty".into(), "literal".into()];
        let metadata = TableMetadata::new("sample");

        let csv = super::build_export_bytes(
            ExportFormat::Csv,
            super::ExportBuildInput {
                rows: rows.clone(),
                binary_cells: vec![],
                columns: columns.clone(),
                metadata: metadata.clone(),
                plugin: None,
            },
        )
        .expect("CSV export should build")
        .expect("CSV export should produce bytes");
        assert_eq!(
            String::from_utf8(csv).expect("CSV is UTF-8"),
            "nullable,empty,literal\n\\N,\"\",NULL"
        );

        let sql = super::build_export_bytes(
            ExportFormat::InsertSql,
            super::ExportBuildInput {
                rows,
                binary_cells: vec![],
                columns,
                metadata,
                plugin: None,
            },
        )
        .expect("SQL export should build")
        .expect("SQL export should produce bytes");
        assert!(
            String::from_utf8(sql)
                .expect("SQL is UTF-8")
                .contains("(NULL, '', 'NULL')")
        );
    }

    #[test]
    fn build_export_bytes_uses_mysql_bit_literal() {
        let plugin = DbManager::default()
            .get_plugin(&DatabaseType::MySQL)
            .expect("MySQL plugin should exist");
        let rows = vec![vec![Some("1".to_string()), Some("1".to_string())]];
        let columns = vec!["id".into(), "bit_name".into()];
        let metadata = TableMetadata::new("test_bit")
            .with_columns(columns.clone())
            .with_column_meta(vec![
                ColumnInfo {
                    name: "id".to_string(),
                    data_type: "INT".to_string(),
                    is_nullable: false,
                    is_primary_key: true,
                    default_value: None,
                    comment: None,
                    charset: None,
                    collation: None,
                },
                ColumnInfo {
                    name: "bit_name".to_string(),
                    data_type: "BIT(1)".to_string(),
                    is_nullable: false,
                    is_primary_key: false,
                    default_value: None,
                    comment: None,
                    charset: None,
                    collation: None,
                },
            ]);

        let sql = super::build_export_bytes(
            ExportFormat::InsertSql,
            super::ExportBuildInput {
                rows,
                binary_cells: vec![],
                columns,
                metadata,
                plugin: Some(plugin),
            },
        )
        .expect("SQL export should build")
        .expect("SQL export should produce bytes");

        assert_eq!(
            String::from_utf8(sql).expect("SQL is UTF-8"),
            "INSERT INTO `test_bit` (`id`, `bit_name`) VALUES\n(1, 1);"
        );
    }

    #[test]
    fn build_export_bytes_uses_binary_sidecar_for_csv_xlsx_and_sql() {
        let rows = vec![vec![Some("wrong".to_string()), None]];
        let columns = vec![
            SharedString::from("payload"),
            SharedString::from("empty_payload"),
        ];
        let metadata =
            TableMetadata::new("binary_values").with_columns(vec!["payload", "empty_payload"]);
        let binary_cells = vec![
            BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: vec![1, 2, 3],
            },
            BinaryCell {
                row_index: 0,
                column_index: 1,
                bytes: vec![],
            },
        ];

        let csv = super::build_export_bytes(
            ExportFormat::Csv,
            super::ExportBuildInput {
                rows: rows.clone(),
                binary_cells: binary_cells.clone(),
                columns: columns.clone(),
                metadata: metadata.clone(),
                plugin: None,
            },
        )
        .expect("CSV export should build")
        .expect("CSV export should produce bytes");
        assert_eq!(
            String::from_utf8(csv).expect("CSV is UTF-8"),
            "payload,empty_payload\nbase64:AQID,base64:"
        );

        let xlsx = super::build_export_bytes(
            ExportFormat::Xlsx,
            super::ExportBuildInput {
                rows: rows.clone(),
                binary_cells: binary_cells.clone(),
                columns: columns.clone(),
                metadata: metadata.clone(),
                plugin: None,
            },
        )
        .expect("XLSX export should build")
        .expect("XLSX export should produce bytes");
        let shared_strings = read_xlsx_entry(&xlsx, "xl/sharedStrings.xml");
        assert!(shared_strings.contains("base64:AQID"), "{shared_strings}");
        assert!(shared_strings.contains("base64:"), "{shared_strings}");
        assert!(!shared_strings.contains("wrong"), "{shared_strings}");

        let plugin = DbManager::default()
            .get_plugin(&DatabaseType::MySQL)
            .expect("MySQL plugin should exist");
        let sql = super::build_export_bytes(
            ExportFormat::InsertSql,
            super::ExportBuildInput {
                rows,
                binary_cells,
                columns,
                metadata,
                plugin: Some(plugin),
            },
        )
        .expect("SQL export should build")
        .expect("SQL export should produce bytes");
        assert_eq!(
            String::from_utf8(sql).expect("SQL is UTF-8"),
            "INSERT INTO `binary_values` (`payload`, `empty_payload`) VALUES\n(X'010203', X'');"
        );
    }
}
