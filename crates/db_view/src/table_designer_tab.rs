use crate::search_shortcut::{
    DB_SEARCH_CONTEXT, FocusSearchInput, RefreshDatabasePanel, SaveDatabaseChanges,
    focus_search_input,
};
use futures::channel::oneshot;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, AsyncApp, ClipboardItem, Context, Div, DragMoveEvent, Entity, EntityId,
    EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement,
    ListSizingBehavior, MouseButton, ParentElement, Pixels, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Subscription, Task, UniformListScrollHandle, Window, div,
    px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IndexPath, Sizable, Size, Theme, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    dialog::{DialogButtonProps, DialogFooter},
    form::{field, h_form},
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState, Textarea, TextareaState},
    scroll::Scrollbar,
    select::{Select, SelectEvent, SelectItem, SelectState},
    spinner::Spinner,
    tab::{Tab, TabBar},
    v_flex,
};
use one_assets::IconName;
use one_ui::{IconButton, IconSize as OneIconSize};
use std::collections::HashSet;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use crate::database_view_plugin::{
    ColumnEditorCapabilities, get_column_editor_capabilities_for, get_engines_for,
    get_table_designer_capabilities_for,
};
use db::GlobalDbState;
#[cfg(all(test, feature = "builtin-duckdb"))]
use db::duckdb::DuckDbPlugin;
#[cfg(all(test, not(feature = "builtin-duckdb")))]
use db::ipc::ExternalDatabasePlugin;
use db::plugin::DatabasePlugin;
use db::types::{
    CharsetInfo, CollationInfo, ColumnDefinition, ColumnInfo, IndexDefinition, IndexInfo,
    ParsedColumnType, TableDesign, TableInfo, TableOptions,
};
use gpui_component::select::SearchableVec;
use one_core::gpui_tokio::Tokio;
use one_core::storage::DatabaseType;
use one_core::tab_container::{TabContainer, TabContent, TabContentEvent};
use rust_i18n::t;

/// 列宽抓取区在列边界两侧各自的宽度。
const COLUMN_EDITOR_RESIZE_GRAB_PADDING: Pixels = px(4.0);
/// 分隔线本身的宽度。
const COLUMN_EDITOR_RESIZE_LINE_WIDTH: Pixels = px(1.0);
const COLUMN_EDITOR_COLUMN_COUNT: usize = 8;
const COLUMN_NAME_COL: usize = 0;
const COLUMN_TYPE_COL: usize = 1;
const COLUMN_LENGTH_COL: usize = 2;
const COLUMN_SCALE_COL: usize = 3;
const COLUMN_NULLABLE_COL: usize = 4;
const COLUMN_PRIMARY_KEY_COL: usize = 5;
const COLUMN_AUTO_INCREMENT_COL: usize = 6;
/// 默认值列紧跟在自增之后、注释之前，与各家设计器的习惯顺序一致。
const COLUMN_DEFAULT_COL: usize = 7;
const COLUMN_DRAG_HANDLE_WIDTH: Pixels = px(24.0);
const COLUMN_EDITOR_DEFAULT_WIDTHS: [Pixels; COLUMN_EDITOR_COLUMN_COUNT] = [
    px(160.0),
    px(140.0),
    px(60.0),
    px(60.0),
    px(50.0),
    px(50.0),
    px(50.0),
    px(120.0),
];
const COLUMN_EDITOR_MIN_WIDTHS: [Pixels; COLUMN_EDITOR_COLUMN_COUNT] = [
    px(96.0),
    px(96.0),
    px(48.0),
    px(48.0),
    px(42.0),
    px(42.0),
    px(42.0),
    px(80.0),
];
const COLUMN_EDITOR_MAX_WIDTHS: [Pixels; COLUMN_EDITOR_COLUMN_COUNT] = [
    px(360.0),
    px(320.0),
    px(160.0),
    px(160.0),
    px(120.0),
    px(120.0),
    px(140.0),
    px(260.0),
];

/// 列/索引行的选中底色透明度。
const SELECTED_ROW_BACKGROUND_OPACITY: f32 = 0.1;
/// 列/索引行的悬停底色透明度。
const HOVERED_ROW_BACKGROUND_OPACITY: f32 = 0.3;

/// 行悬停时叠加的底色。
///
/// 选中的行返回 `None`：悬停样式是在基础样式之后 refine 的，一旦叠加就会把选中
/// 底色整个盖掉，看起来就像「鼠标移到这一行，高亮没了」。树视图同样只在未选中
/// 的行上叠加悬停底色。
fn row_hover_background(is_selected: bool, theme: &Theme) -> Option<Hsla> {
    (!is_selected).then(|| theme.muted.opacity(HOVERED_ROW_BACKGROUND_OPACITY))
}

#[derive(Clone, Debug, PartialEq)]
pub enum DesignerTab {
    Columns,
    Indexes,
    Options,
    SqlPreview,
    Ddl,
}

#[derive(Clone, Debug)]
pub enum TableDesignerEvent {
    Saved {
        connection_id: String,
        database_name: String,
        schema_name: Option<String>,
        table_name: String,
        is_new_table: bool,
        tab_id: Option<String>,
    },
}

#[derive(Clone)]
pub struct TableDesignerConfig {
    pub connection_id: String,
    pub database_name: String,
    pub schema_name: Option<String>,
    pub database_type: DatabaseType,
    pub table_name: Option<String>,
    pub tab_id: Option<String>,
}

impl TableDesignerConfig {
    pub fn new(
        connection_id: impl Into<String>,
        database_name: impl Into<String>,
        database_type: DatabaseType,
    ) -> Self {
        Self {
            connection_id: connection_id.into(),
            database_name: database_name.into(),
            schema_name: None,
            database_type,
            table_name: None,
            tab_id: None,
        }
    }

    pub fn with_schema_name(mut self, name: impl Into<String>) -> Self {
        self.schema_name = Some(name.into());
        self
    }

    pub fn with_table_name(mut self, name: impl Into<String>) -> Self {
        self.table_name = Some(name.into());
        self
    }

    pub fn with_tab_id(mut self, id: impl Into<String>) -> Self {
        self.tab_id = Some(id.into());
        self
    }
}

pub(crate) fn build_table_design_from_metadata(
    database_type: DatabaseType,
    database_name: String,
    table_name: String,
    columns: &[ColumnInfo],
    indexes: &[IndexInfo],
    table_info: Option<&TableInfo>,
    plugin: Option<&dyn DatabasePlugin>,
) -> TableDesign {
    // IPC 驱动的 schema/columns 不一定回列级主键（共享 handleSchemaColumns
    // 不查约束），但 schema/indexes 会标记 PK 索引——从它的成员反推列主键。
    // 原生插件两条路都有，先到先得，不覆盖已有 true。
    // Oracle 家族的字典列名全大写，列名可能小写——不区分大小写匹配。
    let pk_members: std::collections::HashSet<String> = indexes
        .iter()
        .filter(|idx| idx.is_primary || idx.name.eq_ignore_ascii_case("PRIMARY"))
        .flat_map(|idx| idx.columns.iter())
        .map(|name| name.to_ascii_uppercase())
        .collect();
    let columns: Vec<ColumnInfo> = columns
        .iter()
        .map(|col| {
            let mut col = col.clone();
            if !col.is_primary_key && pk_members.contains(&col.name.to_ascii_uppercase()) {
                col.is_primary_key = true;
            }
            col
        })
        .collect();
    let primary_key_count = columns.iter().filter(|col| col.is_primary_key).count();
    let column_defs: Vec<ColumnDefinition> = columns
        .iter()
        .map(|col| {
            let parsed = plugin
                .map(|plugin| plugin.parse_column_type(&col.data_type))
                .unwrap_or_else(|| fallback_parse_column_type(&col.data_type));
            column_info_to_definition(database_type.clone(), col, parsed, primary_key_count)
        })
        .collect();

    let index_defs: Vec<IndexDefinition> = indexes
        .iter()
        .filter(|idx| !idx.is_primary && idx.name.to_uppercase() != "PRIMARY")
        .map(|idx| IndexDefinition {
            name: idx.name.clone(),
            columns: idx.columns.clone(),
            is_unique: idx.is_unique,
            is_primary: false,
            index_type: idx.index_type.clone(),
            comment: String::new(),
        })
        .collect();

    let mut options = TableOptions::default();
    if let Some(table_info) = table_info {
        options.comment = table_info.comment.clone().unwrap_or_default();
        // issue #586：既有表的引擎/字符集/排序规则必须一起带进原始设计，
        // 否则设计器只能看到默认值，保存时也无从判断表设置是否被改动。
        options.engine = table_info.engine.clone();
        options.charset = table_info.charset.clone();
        options.collation = table_info.collation.clone();
    }

    TableDesign {
        database_name,
        table_name,
        columns: column_defs,
        indexes: index_defs,
        foreign_keys: vec![],
        options,
    }
}

fn column_info_to_definition(
    database_type: DatabaseType,
    col: &ColumnInfo,
    parsed: ParsedColumnType,
    primary_key_count: usize,
) -> ColumnDefinition {
    let is_auto_increment =
        column_is_auto_increment(&database_type, col, &parsed, primary_key_count);
    let base_type = parsed.base_type;
    let data_type = if let Some(enum_values) = parsed.enum_values {
        format!("{}({})", base_type, enum_values)
    } else {
        base_type.clone()
    };

    ColumnDefinition {
        name: col.name.clone(),
        data_type,
        length: parsed.length,
        precision: None,
        scale: parsed.scale,
        is_nullable: col.is_nullable,
        is_primary_key: col.is_primary_key,
        is_auto_increment,
        is_unsigned: parsed.is_unsigned,
        default_value: col.default_value.clone(),
        comment: col.comment.clone().unwrap_or_default(),
        charset: col.charset.clone(),
        collation: col.collation.clone(),
    }
}

/// 某列在设计器里是否算自增列。
///
/// 优先信元数据：各插件已按引擎语义填好 `is_auto_increment`（MSSQL IDENTITY、MySQL EXTRA、
/// PG serial/identity）；元数据没给时，SQLite 按「单列 INTEGER 主键 = rowid 别名」兜底
/// （联合主键、WITHOUT ROWID 表都不算），其它引擎退回类型字符串解析。
fn column_is_auto_increment(
    database_type: &DatabaseType,
    col: &ColumnInfo,
    parsed: &ParsedColumnType,
    primary_key_count: usize,
) -> bool {
    if col.is_auto_increment {
        return true;
    }
    if matches!(database_type, DatabaseType::SQLite) {
        return col.is_primary_key
            && parsed.base_type.eq_ignore_ascii_case("INTEGER")
            && primary_key_count == 1;
    }
    parsed.is_auto_increment
}

fn fallback_parse_column_type(data_type: &str) -> ParsedColumnType {
    let (base_type, length) = parse_data_type(data_type);
    ParsedColumnType {
        base_type,
        length,
        scale: extract_scale_from_type_str(data_type),
        enum_values: None,
        is_unsigned: data_type.to_uppercase().contains("UNSIGNED"),
        is_auto_increment: data_type.to_uppercase().contains("AUTO_INCREMENT"),
    }
}

fn parse_data_type(data_type: &str) -> (String, Option<u32>) {
    if let Some(start) = data_type.find('(') {
        if let Some(end) = data_type.find(')') {
            let base_type = data_type[..start].trim().to_string();
            let len_str = &data_type[start + 1..end];
            if let Some(comma) = len_str.find(',') {
                let length = len_str[..comma].trim().parse().ok();
                return (base_type, length);
            }
            let length = len_str.trim().parse().ok();
            return (base_type, length);
        }
    }
    (data_type.to_string(), None)
}

fn extract_scale_from_type_str(data_type: &str) -> Option<u32> {
    if let Some(start) = data_type.find('(') {
        if let Some(end) = data_type.find(')') {
            let len_str = &data_type[start + 1..end];
            if let Some(comma) = len_str.find(',') {
                return len_str[comma + 1..].trim().parse().ok();
            }
        }
    }
    None
}

pub(crate) fn find_loaded_table_info(
    tables: Vec<TableInfo>,
    table_name: &str,
    schema_name: Option<&str>,
) -> Option<TableInfo> {
    tables.into_iter().find(|table| {
        if table.name != table_name {
            return false;
        }
        match (schema_name, table.schema.as_deref()) {
            // 驱动没有返回 schema 信息时,退化按表名匹配。
            // 宿主调用 list_tables 时已带上 schema 过滤条件,所以同名表
            // 在结果集中通常唯一。
            (Some(_), None) => true,
            (Some(schema), Some(table_schema)) => schema == table_schema,
            (None, _) => true,
        }
    })
}

fn column_order_snapshot(design: &TableDesign) -> Vec<&str> {
    design.columns.iter().map(|col| col.name.as_str()).collect()
}

/// 复制预览页签内容的载荷：空白内容返回 `None`，避免把空串写进剪贴板。
fn preview_sql_to_copy(sql: &str) -> Option<ClipboardItem> {
    (!sql.trim().is_empty()).then(|| ClipboardItem::new_string(sql.to_string()))
}

pub struct TableDesigner {
    title: SharedString,
    focus_handle: FocusHandle,
    config: TableDesignerConfig,
    active_tab: DesignerTab,
    table_name_input: Entity<InputState>,
    table_comment_input: Entity<InputState>,
    engine_select: Entity<SelectState<Vec<EngineSelectItem>>>,
    charset_select: Entity<SelectState<Vec<CharsetSelectItem>>>,
    collation_select: Entity<SelectState<Vec<CollationSelectItem>>>,
    auto_increment_input: Entity<InputState>,
    columns_editor: Entity<ColumnsEditor>,
    indexes_editor: Entity<IndexesEditor>,
    engines: Vec<EngineSelectItem>,
    charsets: Vec<CharsetInfo>,
    sql_preview_input: Entity<EditorState>,
    ddl_preview_input: Entity<EditorState>,
    preview_refresh_state: PreviewRefreshScheduleState,
    metadata_load_seq: usize,
    structure_load: StructureLoadState,
    preview_generation: usize,
    sql_preview_loading: bool,
    executing: bool,
    original_design: Option<TableDesign>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
enum ExecuteSuccessBehavior {
    StayOpen {
        tab_id: Option<String>,
    },
    CloseTab {
        tab_container: Entity<TabContainer>,
        tab_id: String,
        emitted_tab_id: Option<String>,
    },
}

#[derive(Clone)]
struct TableDesignerExecutionRequest {
    connection_id: String,
    database_name: String,
    schema_name: Option<String>,
    sql: String,
    table_name: String,
    is_new_table: bool,
    success_behavior: ExecuteSuccessBehavior,
}

#[derive(Default)]
struct PreviewRefreshScheduleState {
    refresh_pending: bool,
}

impl PreviewRefreshScheduleState {
    fn request_refresh(&mut self) -> bool {
        if self.refresh_pending {
            return false;
        }
        self.refresh_pending = true;
        true
    }

    fn finish_refresh(&mut self) {
        self.refresh_pending = false;
    }
}

/// 表结构加载的触发来源。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StructureLoadReason {
    /// 打开既有表设计器时的首次加载。
    InitialOpen,
    /// 保存成功后按最新结构重载。
    AfterSave,
    /// 用户在列/索引工具栏主动刷新结构。
    ManualRefresh,
}

/// 表结构加载态。
///
/// 首次打开和手动刷新都会把整块面板换成加载中：前者还没有结构可显示，
/// 后者需要明确表达正在从数据库重建当前表单。保存后的自动重载留在原地，
/// 避免表单闪烁。
#[derive(Default)]
struct StructureLoadState {
    loading: bool,
    refreshing: bool,
}

impl StructureLoadState {
    fn begin(&mut self, reason: StructureLoadReason) {
        self.loading = matches!(
            reason,
            StructureLoadReason::InitialOpen | StructureLoadReason::ManualRefresh
        );
        self.refreshing = true;
    }

    /// 加载结束（成功或失败都要调，否则面板会一直停在加载中）。
    fn finish(&mut self) {
        self.loading = false;
        self.refreshing = false;
    }

    fn is_loading(&self) -> bool {
        self.loading
    }

    fn is_refreshing(&self) -> bool {
        self.refreshing
    }
}

impl TableDesigner {
    fn on_action_refresh_database_panel(
        &mut self,
        _: &RefreshDatabasePanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_structure(window, cx);
    }

    fn on_action_save_database_changes(
        &mut self,
        _: &SaveDatabaseChanges,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save(window, cx);
    }

    pub fn new(
        title: impl Into<SharedString>,
        config: TableDesignerConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let title = title.into();
        let focus_handle = cx.focus_handle();

        let table_name_input = cx.new(|cx| {
            let mut input =
                InputState::new(window, cx).placeholder(t!("Table.enter_table_name").to_string());
            if let Some(name) = &config.table_name {
                input.set_value(name.clone(), window, cx);
            }
            input
        });

        let table_comment_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Table.table_comment").to_string())
        });

        let auto_increment_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Table.auto_increment").to_string())
        });

        let (engines, column_editor_capabilities): (
            Vec<EngineSelectItem>,
            ColumnEditorCapabilities,
        ) = {
            let engines = get_engines_for(config.database_type.clone(), cx)
                .into_iter()
                .map(|name| EngineSelectItem { name })
                .collect();
            let capabilities = get_column_editor_capabilities_for(config.database_type.clone(), cx);
            (engines, capabilities)
        };

        let engine_select = cx.new(|cx| {
            if engines.is_empty() {
                SelectState::new(vec![], None, window, cx)
            } else {
                SelectState::new(engines.clone(), Some(IndexPath::new(0)), window, cx)
            }
        });

        let charsets = Self::get_charsets(&config.database_type, cx);
        let charset_items: Vec<CharsetSelectItem> = charsets
            .iter()
            .cloned()
            .map(|info| CharsetSelectItem { info })
            .collect();
        let charset_select =
            cx.new(|cx| SelectState::new(charset_items, Some(IndexPath::new(0)), window, cx));

        let default_charset = charsets
            .first()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "utf8mb4".to_string());
        let collations = Self::get_collations(&config.database_type, &default_charset, cx);
        let collation_items: Vec<CollationSelectItem> = collations
            .iter()
            .cloned()
            .map(|info| CollationSelectItem { info })
            .collect();
        let default_coll_idx = collation_items
            .iter()
            .position(|c| c.info.is_default)
            .unwrap_or(0);
        let collation_select = cx.new(|cx| {
            SelectState::new(
                collation_items,
                Some(IndexPath::new(default_coll_idx)),
                window,
                cx,
            )
        });

        let can_refresh_structure = config.table_name.is_some();
        let columns_editor = cx.new(|cx| {
            ColumnsEditor::new(
                config.database_type.clone(),
                charsets.clone(),
                column_editor_capabilities,
                can_refresh_structure,
                window,
                cx,
            )
        });
        let indexes_editor = cx.new(|cx| IndexesEditor::new(can_refresh_structure, window, cx));

        let sql_preview_input = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(false)
        });
        let ddl_preview_input = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("sql")
                .line_number(false)
        });

        let name_sub = cx.subscribe_in(
            &table_name_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    this.schedule_preview_refresh(window, cx);
                }
            },
        );

        let comment_sub = cx.subscribe_in(
            &table_comment_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    this.schedule_preview_refresh(window, cx);
                }
            },
        );

        let auto_inc_sub = cx.subscribe_in(
            &auto_increment_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::Change = event {
                    this.schedule_preview_refresh(window, cx);
                }
            },
        );

        let engine_sub = cx.observe_in(&engine_select, window, |this, _, window, cx| {
            this.schedule_preview_refresh(window, cx);
        });

        let charset_select_clone = charset_select.clone();
        let charset_sub = cx.observe_in(&charset_select, window, move |this, _, window, cx| {
            this.schedule_preview_refresh(window, cx);
            this.update_collations_for_charset(&charset_select_clone, window, cx);
        });

        let collation_sub = cx.observe_in(&collation_select, window, |this, _, window, cx| {
            this.schedule_preview_refresh(window, cx);
        });

        let cols_sub = cx.subscribe_in(
            &columns_editor,
            window,
            |this, _, event: &ColumnsEditorEvent, window, cx| match event {
                ColumnsEditorEvent::Changed => this.schedule_preview_refresh(window, cx),
                ColumnsEditorEvent::RefreshRequested => this.refresh_structure(window, cx),
            },
        );

        let idx_sub = cx.subscribe_in(
            &indexes_editor,
            window,
            |this, _, event: &IndexesEditorEvent, window, cx| match event {
                IndexesEditorEvent::Changed => this.schedule_preview_refresh(window, cx),
                IndexesEditorEvent::RefreshRequested => this.refresh_structure(window, cx),
            },
        );

        let mut designer = Self {
            title,
            focus_handle,
            config,
            active_tab: DesignerTab::Columns,
            table_name_input,
            table_comment_input,
            engine_select,
            charset_select,
            collation_select,
            auto_increment_input,
            columns_editor,
            indexes_editor,
            engines,
            charsets,
            sql_preview_input,
            ddl_preview_input,
            preview_refresh_state: PreviewRefreshScheduleState::default(),
            metadata_load_seq: 0,
            structure_load: StructureLoadState::default(),
            preview_generation: 0,
            sql_preview_loading: false,
            executing: false,
            original_design: None,
            _subscriptions: vec![
                name_sub,
                comment_sub,
                auto_inc_sub,
                engine_sub,
                charset_sub,
                collation_sub,
                cols_sub,
                idx_sub,
            ],
        };

        designer.update_previews(window, cx);

        if designer.config.table_name.is_some() {
            designer.load_table_structure(StructureLoadReason::InitialOpen, cx);
        }

        designer
    }

    fn get_charsets(database_type: &DatabaseType, cx: &App) -> Vec<CharsetInfo> {
        let global_state = cx.global::<GlobalDbState>();
        if let Ok(plugin) = global_state.db_manager.get_plugin(database_type) {
            plugin.get_charsets()
        } else {
            vec![CharsetInfo {
                name: "utf8mb4".to_string(),
                description: "UTF-8 Unicode".to_string(),
                default_collation: "utf8mb4_general_ci".to_string(),
            }]
        }
    }

    fn get_collations(database_type: &DatabaseType, charset: &str, cx: &App) -> Vec<CollationInfo> {
        let global_state = cx.global::<GlobalDbState>();
        if let Ok(plugin) = global_state.db_manager.get_plugin(database_type) {
            plugin.get_collations(charset)
        } else {
            vec![CollationInfo {
                name: "utf8mb4_general_ci".to_string(),
                charset: "utf8mb4".to_string(),
                is_default: true,
            }]
        }
    }

    fn update_collations_for_charset(
        &mut self,
        charset_select: &Entity<SelectState<Vec<CharsetSelectItem>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_charset = charset_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| "utf8mb4".to_string());

        let mut collations =
            Self::get_collations(&self.config.database_type, &selected_charset, cx);
        // issue #586：回填表的排序规则时要能在字符集切换后保住它。观察者晚于
        // 回填执行，若无条件选回字符集默认值，用户看到的仍是「默认」。
        // 只有当前排序规则自成一个字符集（与选中字符集不兼容）时才退到默认值，
        // 用户手动切换字符集正属于这种情况。
        let current_collation = self
            .collation_select
            .read(cx)
            .selected_value()
            .cloned()
            .filter(|value| !value.is_empty());
        if let Some(collation) = current_collation.as_deref() {
            if Self::collation_belongs_to_charset(collation, &selected_charset)
                && !collations.iter().any(|info| info.name == collation)
            {
                // 服务端扩展排序规则不在插件静态列表里，补进来才能保住它。
                collations.insert(
                    0,
                    CollationInfo {
                        name: collation.to_string(),
                        charset: selected_charset.clone(),
                        is_default: false,
                    },
                );
            }
        }
        let collation_items: Vec<CollationSelectItem> = collations
            .into_iter()
            .map(|info| CollationSelectItem { info })
            .collect();
        let default_idx = collation_items
            .iter()
            .position(|c| c.info.is_default)
            .unwrap_or(0);
        let selected_idx = current_collation
            .and_then(|collation| {
                collation_items
                    .iter()
                    .position(|item| item.info.name == collation)
            })
            .unwrap_or(default_idx);

        self.collation_select.update(cx, |state, inner_cx| {
            state.set_items(collation_items, window, inner_cx);
            state.set_selected_index(Some(IndexPath::new(selected_idx)), window, inner_cx);
        });
    }

    /// 排序规则名以字符集名为前缀（`utf8mb4_0900_ai_ci`），据此判断
    /// 它是否属于当前字符集，避免切换字符集后还挂着上一个字符集的排序规则。
    fn collation_belongs_to_charset(collation: &str, charset: &str) -> bool {
        collation
            .strip_prefix(charset)
            .is_some_and(|rest| rest.starts_with('_'))
    }

    /// issue #586：把既有表的引擎/字符集/排序规则回填到选项栏，
    /// 并让新增列默认继承这套表设置。
    fn apply_loaded_table_options(
        &mut self,
        options: &TableOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(engine) = options.engine.as_deref().filter(|value| !value.is_empty()) {
            // 引擎清单同样来自插件静态能力描述，服务端新引擎需要先补进列表。
            if !self.engines.iter().any(|item| item.name == engine) {
                self.engines.push(EngineSelectItem {
                    name: engine.to_string(),
                });
                let items = self.engines.clone();
                self.engine_select.update(cx, |state, cx| {
                    state.set_items(items, window, cx);
                });
            }
            if self
                .engine_select
                .read(cx)
                .selected_value()
                .map(String::as_str)
                != Some(engine)
            {
                let engine = engine.to_string();
                self.engine_select.update(cx, |state, cx| {
                    state.set_selected_value(&engine, window, cx);
                });
            }
        }

        // 个别驱动只回传排序规则、不回传字符集；MySQL 的排序规则名本身
        // 带字符集前缀，据此把字符集补回来，避免选项栏落回默认值。
        let loaded_charset = options
            .charset
            .clone()
            .filter(|value| !value.is_empty())
            .or_else(|| {
                let collation = options
                    .collation
                    .as_deref()
                    .filter(|value| !value.is_empty())?;
                let charset = collation.split('_').next()?;
                self.charsets
                    .iter()
                    .any(|info| info.name == charset)
                    .then(|| charset.to_string())
            });

        if let Some(charset) = loaded_charset.as_deref() {
            // 服务端可能返回插件静态字符集列表之外的字符集。此时必须补进
            // 列表再选中，否则会落回第一项，保存时误改表字符集。
            if !self.charsets.iter().any(|info| info.name == charset) {
                self.charsets.push(CharsetInfo {
                    name: charset.to_string(),
                    description: charset.to_string(),
                    default_collation: options.collation.clone().unwrap_or_default(),
                });
                let items: Vec<CharsetSelectItem> = self
                    .charsets
                    .iter()
                    .cloned()
                    .map(|info| CharsetSelectItem { info })
                    .collect();
                self.charset_select.update(cx, |state, cx| {
                    state.set_items(items, window, cx);
                });
            }
            let charset = charset.to_string();
            self.charset_select.update(cx, |state, cx| {
                state.set_selected_value(&charset, window, cx);
            });
            self.apply_loaded_charset_collations(
                &charset,
                options.collation.as_deref(),
                window,
                cx,
            );
        }

        self.columns_editor.update(cx, |editor, cx| {
            editor.set_table_defaults(loaded_charset, options.collation.clone());
            cx.notify();
        });
    }

    /// 按表字符集重建排序规则列表，并选中表自己的排序规则。
    fn apply_loaded_charset_collations(
        &mut self,
        charset: &str,
        collation: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut collations = Self::get_collations(&self.config.database_type, charset, cx);
        if let Some(collation) = collation.filter(|value| !value.is_empty()) {
            if !collations.iter().any(|info| info.name == collation) {
                // 元数据里的排序规则可能不在插件的静态列表中，补进来才能显示真实值。
                collations.insert(
                    0,
                    CollationInfo {
                        name: collation.to_string(),
                        charset: charset.to_string(),
                        is_default: false,
                    },
                );
            }
        }
        let items: Vec<CollationSelectItem> = collations
            .into_iter()
            .map(|info| CollationSelectItem { info })
            .collect();
        let selected_idx = collation
            .and_then(|collation| items.iter().position(|item| item.info.name == collation))
            .or_else(|| items.iter().position(|item| item.info.is_default))
            .unwrap_or(0);

        self.collation_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(Some(IndexPath::new(selected_idx)), window, cx);
        });
    }

    fn collect_design(&self, cx: &App) -> TableDesign {
        let table_name = self.table_name_input.read(cx).text().to_string();
        let table_comment = self.table_comment_input.read(cx).text().to_string();
        let columns = self.columns_editor.read(cx).get_columns(cx);
        let indexes = self.indexes_editor.read(cx).get_indexes(cx);

        let engine = self.engine_select.read(cx).selected_value().cloned();
        let charset = self.charset_select.read(cx).selected_value().cloned();
        let collation = self.collation_select.read(cx).selected_value().cloned();
        let auto_increment_str = self.auto_increment_input.read(cx).text().to_string();
        let auto_increment = auto_increment_str.parse::<u64>().ok();

        let options = TableOptions {
            engine,
            charset,
            collation,
            comment: table_comment,
            auto_increment,
        };

        TableDesign {
            database_name: self.config.database_name.clone(),
            table_name,
            columns,
            indexes,
            foreign_keys: vec![],
            options,
        }
    }

    fn collect_column_renames(&self, cx: &App) -> Vec<(String, String)> {
        self.columns_editor.read(cx).get_column_renames(cx)
    }

    fn normalize_column_renames(
        original: &TableDesign,
        design: &TableDesign,
        column_renames: &[(String, String)],
    ) -> Vec<(String, String)> {
        let original_names: HashSet<&str> = original
            .columns
            .iter()
            .map(|col| col.name.as_str())
            .collect();
        let current_names: HashSet<&str> =
            design.columns.iter().map(|col| col.name.as_str()).collect();
        let mut seen_old = HashSet::new();
        let mut seen_new = HashSet::new();

        column_renames
            .iter()
            .filter_map(|(old_name, new_name)| {
                if old_name.is_empty() || new_name.is_empty() || old_name == new_name {
                    return None;
                }
                if !original_names.contains(old_name.as_str())
                    || !current_names.contains(new_name.as_str())
                {
                    return None;
                }
                if seen_old.contains(old_name) || seen_new.contains(new_name) {
                    return None;
                }
                seen_old.insert(old_name.clone());
                seen_new.insert(new_name.clone());
                Some((old_name.clone(), new_name.clone()))
            })
            .collect()
    }

    fn update_previews(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let design = self.collect_design(cx);
        let column_renames = self.collect_column_renames(cx);
        let original = self.original_design.clone();
        let column_renames = match &original {
            Some(original) => Self::normalize_column_renames(original, &design, &column_renames),
            None => column_renames,
        };

        // 预览与保存共用同一条异步驱动路径（build_table_design_sql），由插件
        // （driver jar）生成方言正确的 DDL / COMMENT，其他数据库同样受益。
        self.preview_generation += 1;
        let generation = self.preview_generation;
        self.sql_preview_loading = true;
        cx.notify();
        let window_handle = window.window_handle();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let sql_result = global_state
                .build_table_design_sql(
                    cx,
                    connection_id.clone(),
                    database_name.clone(),
                    schema_name.clone(),
                    original.clone(),
                    design.clone(),
                    column_renames.clone(),
                )
                .await;

            // 完整 CREATE TABLE 预览：新建表时与 SQL 预览相同；编辑既有表时
            // 单独通过驱动生成完整建表 DDL。
            let ddl_result = if original.is_some() {
                Some(
                    global_state
                        .build_table_design_sql(
                            cx,
                            connection_id,
                            database_name,
                            schema_name,
                            None,
                            design.clone(),
                            Vec::new(),
                        )
                        .await,
                )
            } else {
                None
            };

            let _ = cx.update(|cx: &mut App| {
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    let _ = this.update(cx, |designer, cx| {
                        if designer.preview_generation != generation {
                            return;
                        }
                        designer.sql_preview_loading = false;
                        if let Ok(sql) = &sql_result {
                            if let Some(original) = &designer.original_design {
                                if Self::sql_has_changes(sql) {
                                    let original_order = column_order_snapshot(original);
                                    let current_order = column_order_snapshot(&design);
                                    tracing::warn!(
                                        target: "table_designer_diag",
                                        table = %design.table_name,
                                        database_type = ?designer.config.database_type,
                                        ?column_renames,
                                        ?original_order,
                                        ?current_order,
                                        original_columns = ?original.columns,
                                        current_columns = ?design.columns,
                                        sql = %sql,
                                        "[table_designer_diag] existing table preview detected diff"
                                    );
                                }
                            }
                            designer.sql_preview_input.update(cx, |state, cx| {
                                state.set_value(sql.clone(), window, cx);
                            });
                            if ddl_result.is_none() {
                                designer.ddl_preview_input.update(cx, |state, cx| {
                                    state.set_value(sql.clone(), window, cx);
                                });
                            }
                        }
                        if let Some(Ok(ddl)) = &ddl_result {
                            designer.ddl_preview_input.update(cx, |state, cx| {
                                state.set_value(ddl.clone(), window, cx);
                            });
                        }
                        cx.notify();
                    });
                });
            });
        })
        .detach();
    }

    fn schedule_preview_refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // 子编辑器事件会在同一个 effect cycle 内级联触发，延后到周期末再统一回读状态，
        // 避免预览读取到上一拍的聚合结果。
        if !self.preview_refresh_state.request_refresh() {
            return;
        }

        cx.defer_in(window, |this, window, cx| {
            this.preview_refresh_state.finish_refresh();
            this.update_previews(window, cx);
        });
    }

    fn sql_has_changes(sql: &str) -> bool {
        let trimmed = sql.trim();
        let no_changes_localized = t!("SqlEditor.no_changes").to_string();
        !trimmed.is_empty()
            && !trimmed.starts_with("-- No changes")
            && !trimmed.starts_with(no_changes_localized.as_str())
    }

    fn contains_destructive_sql(sql: &str) -> bool {
        if !Self::sql_has_changes(sql) {
            return false;
        }

        let normalized_sql = sql
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with("--"))
            .collect::<Vec<_>>()
            .join(" ")
            .to_uppercase();

        ["DROP COLUMN", "DROP INDEX", "DROP CONSTRAINT", "DROP TABLE"]
            .iter()
            .any(|keyword| normalized_sql.contains(keyword))
    }

    fn execute_request(
        &mut self,
        request: TableDesignerExecutionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let global_state = cx.global::<GlobalDbState>().clone();
        let window_handle = window.window_handle();

        self.executing = true;
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = global_state
                .execute_script(
                    cx,
                    request.connection_id.clone(),
                    request.sql.clone(),
                    Some(request.database_name.clone()),
                    request.schema_name.clone(),
                    None,
                )
                .await;

            let _ = cx.update(|cx: &mut App| {
                let _ = cx.update_window(window_handle, |_, window, cx| match &result {
                    Ok(results) => {
                        let _ = this.update(cx, |designer, cx| {
                            designer.executing = false;
                            cx.notify();
                        });
                        let has_error = results.iter().any(|r| r.is_error());
                        if has_error {
                            let error_msg = results
                                .iter()
                                .filter_map(|r| {
                                    if let db::executor::SqlResult::Error(err) = r {
                                        Some(err.message.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect::<Vec<_>>()
                                .join("; ");
                            window.push_notification(
                                format!("{}: {}", t!("Table.execute_failed"), error_msg),
                                cx,
                            );
                        } else {
                            let msg = if request.is_new_table {
                                t!("Table.create_success").to_string()
                            } else {
                                t!("Table.modify_success").to_string()
                            };
                            window.push_notification(msg, cx);
                            let _ =
                                this.update(cx, |designer, cx| match &request.success_behavior {
                                    ExecuteSuccessBehavior::StayOpen { tab_id } => {
                                        cx.emit(TableDesignerEvent::Saved {
                                            connection_id: request.connection_id.clone(),
                                            database_name: request.database_name.clone(),
                                            schema_name: request.schema_name.clone(),
                                            table_name: request.table_name.clone(),
                                            is_new_table: request.is_new_table,
                                            tab_id: tab_id.clone(),
                                        });
                                        if request.is_new_table {
                                            designer.config.table_name =
                                                Some(request.table_name.clone());
                                        }
                                        designer.load_table_structure(
                                            StructureLoadReason::AfterSave,
                                            cx,
                                        );
                                    }
                                    ExecuteSuccessBehavior::CloseTab {
                                        tab_container,
                                        tab_id,
                                        emitted_tab_id,
                                    } => {
                                        cx.emit(TableDesignerEvent::Saved {
                                            connection_id: request.connection_id.clone(),
                                            database_name: request.database_name.clone(),
                                            schema_name: request.schema_name.clone(),
                                            table_name: request.table_name.clone(),
                                            is_new_table: request.is_new_table,
                                            tab_id: emitted_tab_id.clone(),
                                        });
                                        tab_container.update(
                                            cx,
                                            |container: &mut TabContainer, cx| {
                                                container.force_close_tab_by_id(tab_id, window, cx);
                                            },
                                        );
                                    }
                                });
                        }
                    }
                    Err(e) => {
                        let msg = if request.is_new_table {
                            t!("Table.create_failed").to_string()
                        } else {
                            t!("Table.modify_failed").to_string()
                        };
                        window.push_notification(format!("{}: {}", msg, e), cx);
                        let _ = this.update(cx, |designer, cx| {
                            designer.executing = false;
                            cx.notify();
                        });
                    }
                });
            });
        })
        .detach();
    }

    fn maybe_confirm_and_execute(
        &mut self,
        request: TableDesignerExecutionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !Self::contains_destructive_sql(&request.sql) {
            self.execute_request(request, window, cx);
            return;
        }

        self.active_tab = DesignerTab::SqlPreview;
        self.update_previews(window, cx);

        // 确认对话框打开期间先取消保存按钮的 loading，
        // 确认执行时 execute_request 会重新置为 loading，取消则不会卡住。
        self.executing = false;
        cx.notify();

        let designer_entity = cx.entity().clone();
        window.open_dialog(cx, move |dialog, _window, _cx| {
            let request_for_ok = request.clone();
            let designer_entity = designer_entity.clone();

            dialog
                .title(t!("Table.destructive_sql_confirm_title").to_string())
                .confirm()
                .overlay(false)
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Table.destructive_sql_confirm_execute").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .child(
                    v_flex()
                        .gap_2()
                        .child(t!("Table.destructive_sql_confirm_message").to_string())
                        .child(t!("Table.destructive_sql_confirm_desc").to_string())
                        .child(t!("Common.irreversible").to_string()),
                )
                .on_ok(move |_, window, cx| {
                    let request = request_for_ok.clone();
                    designer_entity.update(cx, |designer, cx| {
                        designer.execute_request(request, window, cx);
                    });
                    true
                })
        });
    }

    fn build_and_maybe_execute(
        &mut self,
        design: TableDesign,
        column_renames: Vec<(String, String)>,
        success_behavior: ExecuteSuccessBehavior,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let original = self.original_design.clone();
        let is_new_table = original.is_none();
        let table_name = design.table_name.clone();
        let window_handle = window.window_handle();

        self.executing = true;
        cx.notify();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let sql_result = global_state
                .build_table_design_sql(
                    cx,
                    connection_id.clone(),
                    database_name.clone(),
                    schema_name.clone(),
                    original,
                    design,
                    column_renames,
                )
                .await;

            let _ = cx.update(|cx: &mut App| {
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    let _ = this.update(cx, |designer, cx| match sql_result {
                        Ok(sql) if !Self::sql_has_changes(&sql) => {
                            designer.executing = false;
                            cx.notify();
                            match &success_behavior {
                                ExecuteSuccessBehavior::StayOpen { .. } => {
                                    window
                                        .push_notification(t!("Table.no_changes").to_string(), cx);
                                }
                                ExecuteSuccessBehavior::CloseTab {
                                    tab_container,
                                    tab_id,
                                    ..
                                } => {
                                    tab_container.update(cx, |container: &mut TabContainer, cx| {
                                        container.force_close_tab_by_id(tab_id, window, cx);
                                    });
                                }
                            }
                        }
                        Ok(sql) => {
                            let request = TableDesignerExecutionRequest {
                                connection_id,
                                database_name,
                                schema_name,
                                sql,
                                table_name,
                                is_new_table,
                                success_behavior,
                            };
                            designer.maybe_confirm_and_execute(request, window, cx);
                        }
                        Err(error) => {
                            designer.executing = false;
                            cx.notify();
                            let msg = if is_new_table {
                                t!("Table.create_failed").to_string()
                            } else {
                                t!("Table.modify_failed").to_string()
                            };
                            window.push_notification(format!("{}: {}", msg, error), cx);
                        }
                    });
                });
            });
        })
        .detach();
    }

    pub fn has_unsaved_changes(&self, cx: &App) -> bool {
        let sql = self.sql_preview_input.read(cx).text().to_string();
        Self::sql_has_changes(&sql)
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.handle_execute(&gpui::ClickEvent::default(), window, cx);
    }

    pub fn save_and_close(
        &mut self,
        tab_container: Entity<TabContainer>,
        tab_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let design = self.collect_design(cx);
        if design.table_name.is_empty() {
            window.push_notification(t!("Table.please_enter_table_name").to_string(), cx);
            return;
        }
        if design.columns.is_empty() {
            window.push_notification(t!("Table.please_add_column").to_string(), cx);
            return;
        }

        let column_renames = self.collect_column_renames(cx);
        let success_behavior = ExecuteSuccessBehavior::CloseTab {
            tab_container,
            tab_id,
            emitted_tab_id: self.config.tab_id.clone(),
        };
        self.build_and_maybe_execute(design, column_renames, success_behavior, window, cx);
    }

    /// 从数据库重新读取当前表结构。
    ///
    /// 工具栏在列/索引子编辑器里，但元数据请求和加载状态归表设计器所有，
    /// 这样两个 tab 共享同一个 loading 生命周期和查询路径。
    fn refresh_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.config.table_name.is_none() || self.executing || self.structure_load.is_refreshing()
        {
            return;
        }

        if self.has_unsaved_changes(cx) {
            self.confirm_refresh_structure(window, cx);
            return;
        }

        self.load_table_structure(StructureLoadReason::ManualRefresh, cx);
    }

    fn confirm_refresh_structure(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let designer_entity = cx.entity().clone();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let refresh_entity = designer_entity.clone();
            let footer = DialogFooter::new().children(vec![
                Button::new("cancel")
                    .label(t!("Common.cancel").to_string())
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                    })
                    .into_any_element(),
                Button::new("discard-and-refresh")
                    .label(t!("Table.discard_and_refresh").to_string())
                    .danger()
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        refresh_entity.update(cx, |designer, cx| {
                            designer.load_table_structure(StructureLoadReason::ManualRefresh, cx);
                        });
                    })
                    .into_any_element(),
            ]);

            dialog
                .title(t!("Table.refresh_structure").to_string())
                .overlay_closable(false)
                .close_button(false)
                .footer(footer)
                .child(t!("Table.refresh_discard_changes_prompt").to_string())
        });
    }

    fn load_table_structure(&mut self, reason: StructureLoadReason, cx: &mut Context<Self>) {
        let Some(table_name) = self.config.table_name.clone() else {
            return;
        };

        self.metadata_load_seq += 1;
        let load_seq = self.metadata_load_seq;
        self.structure_load.begin(reason);
        self.set_editors_can_refresh(true, cx);
        cx.notify();
        tracing::warn!(
            target: "table_designer_diag",
            seq = load_seq,
            reason = ?reason,
            connection_id = %self.config.connection_id,
            database = %self.config.database_name,
            schema = ?self.config.schema_name,
            table = %table_name,
            "[table_designer_diag] load_table_structure triggered"
        );

        let global_state = cx.global::<GlobalDbState>().clone();
        let connection_id = self.config.connection_id.clone();
        let database_name = self.config.database_name.clone();
        let schema_name = self.config.schema_name.clone();
        let columns_editor = self.columns_editor.clone();
        let indexes_editor = self.indexes_editor.clone();
        let table_comment_input = self.table_comment_input.clone();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let query_result = Tokio::spawn_result(cx, async move {
                let columns_result = global_state
                    .list_columns_direct(
                        &connection_id,
                        &database_name,
                        schema_name.clone(),
                        &table_name,
                    )
                    .await;

                let indexes_result = global_state
                    .list_indexes_direct(
                        &connection_id,
                        &database_name,
                        schema_name.clone(),
                        &table_name,
                    )
                    .await;

                let tables_result = global_state
                    .list_tables_direct(&connection_id, &database_name, schema_name.clone())
                    .await;

                Ok((
                    columns_result,
                    indexes_result,
                    tables_result,
                    schema_name,
                    table_name,
                ))
            })
            .await;

            let (columns_result, indexes_result, tables_result, schema_name, table_name) =
                match query_result {
                    Ok(result) => result,
                    Err(error) => {
                        tracing::warn!(
                            target: "table_designer_diag",
                            seq = load_seq,
                            error = %error,
                            "[table_designer_diag] load_table_structure Tokio task failed"
                        );
                        let message = t!("Table.load_structure_failed", error = error.to_string())
                            .to_string();
                        let _ = cx.update(|cx| {
                            if let Some(window_id) = cx.active_window() {
                                cx.update_window(window_id, |_entity, window, cx| {
                                    window.push_notification(message.clone(), cx);
                                })
                                .ok();
                            }
                        });
                        let _ = this.update(cx, |designer, cx| {
                            designer.finish_structure_load(cx);
                        });
                        return;
                    }
                };

            tracing::warn!(
                target: "table_designer_diag",
                seq = load_seq,
                columns_ok = columns_result.is_ok(),
                columns_count = columns_result.as_ref().map(|cols| cols.len()).unwrap_or(0),
                indexes_ok = indexes_result.is_ok(),
                indexes_count = indexes_result.as_ref().map(|idxs| idxs.len()).unwrap_or(0),
                tables_ok = tables_result.is_ok(),
                tables_count = tables_result.as_ref().map(|tables| tables.len()).unwrap_or(0),
                "[table_designer_diag] load_table_structure query finished"
            );

            let _ = cx.update(|cx| {
                if let Some(window_id) = cx.active_window() {
                    cx.update_window(window_id, |_entity, window, cx| {
                        // 元数据读取失败时必须可见：否则设计器只剩表头，
                        // 用户看到空网格却不知道发生了错误。
                        let mut failures: Vec<String> = Vec::new();
                        let columns = match columns_result {
                            Ok(columns) => Some(columns),
                            Err(error) => {
                                failures.push(error.to_string());
                                None
                            }
                        };
                        let indexes = match indexes_result {
                            Ok(indexes) => Some(indexes),
                            Err(error) => {
                                failures.push(error.to_string());
                                None
                            }
                        };
                        if !failures.is_empty() {
                            window.push_notification(
                                t!("Table.load_structure_failed", error = failures.join("; "))
                                    .to_string(),
                                cx,
                            );
                        }
                        let table_info = tables_result.ok().and_then(|tables| {
                            find_loaded_table_info(tables, &table_name, schema_name.as_deref())
                        });

                        if let Some(ref cols) = columns {
                            columns_editor.update(cx, |editor, cx| {
                                editor.load_columns(cols.clone(), window, cx);
                            });
                        }

                        if let Some(ref idxs) = indexes {
                            indexes_editor.update(cx, |editor, cx| {
                                editor.load_indexes(idxs.clone(), window, cx);
                            });
                        }

                        if let Some(ref info) = table_info {
                            if let Some(comment) = &info.comment {
                                table_comment_input.update(cx, |input, cx| {
                                    input.set_value(comment.clone(), window, cx);
                                });
                            }
                        }

                        let _ = this.update(cx, |designer, cx| {
                            let original_design = designer.build_original_design(
                                columns.unwrap_or_default(),
                                indexes.unwrap_or_default(),
                                table_info.as_ref(),
                                cx,
                            );
                            // issue #586：表自己的引擎/字符集/排序规则要回填到
                            // 选项栏，新增列也默认继承它们；否则编辑既有表时只
                            // 能看到设计器的默认值。
                            let loaded_options = original_design.options.clone();
                            designer.original_design = Some(original_design);
                            designer.apply_loaded_table_options(&loaded_options, window, cx);
                            designer.update_previews(window, cx);
                            designer.finish_structure_load(cx);
                        });
                    })
                } else {
                    let _ = this.update(cx, |designer, cx| {
                        designer.finish_structure_load(cx);
                    });
                    Err(anyhow::anyhow!("No active window"))
                }
            });
        })
        .detach();
    }

    /// 收掉表结构加载态并刷新面板。
    fn finish_structure_load(&mut self, cx: &mut Context<Self>) {
        self.structure_load.finish();
        cx.notify();
    }

    fn set_editors_can_refresh(&self, can_refresh: bool, cx: &mut Context<Self>) {
        self.columns_editor.update(cx, |editor, cx| {
            editor.set_can_refresh(can_refresh, cx);
        });
        self.indexes_editor.update(cx, |editor, cx| {
            editor.set_can_refresh(can_refresh, cx);
        });
    }

    fn build_original_design(
        &self,
        columns: Vec<ColumnInfo>,
        indexes: Vec<IndexInfo>,
        table_info: Option<&TableInfo>,
        cx: &App,
    ) -> TableDesign {
        let global_state = cx.global::<GlobalDbState>();
        let plugin = global_state
            .db_manager
            .get_plugin(&self.config.database_type)
            .ok();
        build_table_design_from_metadata(
            self.config.database_type.clone(),
            self.config.database_name.clone(),
            self.config.table_name.clone().unwrap_or_default(),
            &columns,
            &indexes,
            table_info,
            plugin.as_deref(),
        )
    }

    fn handle_execute(
        &mut self,
        _: &gpui::ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let design = self.collect_design(cx);
        if design.table_name.is_empty() {
            window.push_notification(t!("Table.please_enter_table_name").to_string(), cx);
            return;
        }
        if design.columns.is_empty() {
            window.push_notification(t!("Table.please_add_column").to_string(), cx);
            return;
        }

        let column_renames = self.collect_column_renames(cx);
        let success_behavior = ExecuteSuccessBehavior::StayOpen {
            tab_id: self.config.tab_id.clone(),
        };
        self.build_and_maybe_execute(design, column_renames, success_behavior, window, cx);
    }

    fn render_toolbar(&self, cx: &Context<Self>) -> AnyElement {
        // 只有预览类和 DDL 页签有可复制的 SQL，其它页签不摆这个按钮。
        let preview_source = match self.active_tab {
            DesignerTab::SqlPreview => Some(&self.sql_preview_input),
            DesignerTab::Ddl => Some(&self.ddl_preview_input),
            _ => None,
        };

        h_flex()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .gap_4()
            .items_center()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Table.table_name").to_string()),
                    )
                    .child(Input::new(&self.table_name_input).w(px(200.))),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Table.comment").to_string()),
                    )
                    .child(Input::new(&self.table_comment_input).w(px(300.))),
            )
            .child(div().flex_1())
            .when_some(preview_source, |this, source| {
                this.child(self.render_copy_action(PREVIEW_COPY_SELECTOR, source, cx))
            })
            .child(
                Button::new("execute")
                    .primary()
                    .loading(self.executing)
                    .label(t!("Common.save").to_string())
                    .on_click(cx.listener(Self::handle_execute)),
            )
            .into_any_element()
    }

    fn render_tabs(&self, cx: &Context<Self>) -> AnyElement {
        let active_idx = match self.active_tab {
            DesignerTab::Columns => 0,
            DesignerTab::Indexes => 1,
            DesignerTab::Options => 2,
            DesignerTab::SqlPreview => 3,
            DesignerTab::Ddl => 4,
        };

        h_flex()
            .w_full()
            .justify_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                TabBar::new("designer-tabs")
                    .underline()
                    .with_size(Size::Small)
                    .selected_index(active_idx)
                    .on_click(cx.listener(|this, ix: &usize, _window, cx| {
                        this.active_tab = match ix {
                            0 => DesignerTab::Columns,
                            1 => DesignerTab::Indexes,
                            2 => DesignerTab::Options,
                            3 => DesignerTab::SqlPreview,
                            4 => DesignerTab::Ddl,
                            _ => DesignerTab::Columns,
                        };
                        cx.notify();
                    }))
                    .child(Tab::new().label(t!("Table.columns").to_string()))
                    .child(Tab::new().label(t!("Table.indexes").to_string()))
                    .child(Tab::new().label(t!("Table.options").to_string()))
                    .child(Tab::new().label(t!("Table.sql_preview").to_string()))
                    .child(Tab::new().label(t!("Table.ddl").to_string())),
            )
            .into_any_element()
    }

    fn render_active_tab(&self, _window: &mut Window, cx: &Context<Self>) -> AnyElement {
        match self.active_tab {
            DesignerTab::Columns => self.columns_editor.clone().into_any_element(),
            DesignerTab::Indexes => self.indexes_editor.clone().into_any_element(),
            DesignerTab::Options => self.render_options(cx),
            DesignerTab::SqlPreview => self.render_sql_preview(cx),
            DesignerTab::Ddl => self.render_ddl_preview(),
        }
    }

    fn render_options(&self, cx: &Context<Self>) -> AnyElement {
        let capabilities =
            get_table_designer_capabilities_for(self.config.database_type.clone(), cx);

        v_flex()
            .size_full()
            .p_4()
            .gap_4()
            .when(capabilities.supports_engine, |this| {
                this.child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .w(px(80.))
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Table.engine").to_string()),
                        )
                        .child(Select::new(&self.engine_select).w(px(200.))),
                )
            })
            .when(capabilities.supports_charset, |this| {
                this.child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .w(px(80.))
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Table.charset").to_string()),
                        )
                        .child(Select::new(&self.charset_select).w(px(200.))),
                )
            })
            .when(capabilities.supports_collation, |this| {
                this.child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .w(px(80.))
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Table.collation").to_string()),
                        )
                        .child(Select::new(&self.collation_select).w(px(200.))),
                )
            })
            .when(capabilities.supports_auto_increment, |this| {
                this.child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .w(px(80.))
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Table.auto_increment").to_string()),
                        )
                        .child(Input::new(&self.auto_increment_input).w(px(200.))),
                )
            })
            .into_any_element()
    }

    fn render_sql_preview(&self, cx: &Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .when(self.sql_preview_loading, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(Spinner::new().with_size(Size::Small))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Table.generating_sql_preview").to_string()),
                        ),
                )
            })
            .child(
                Editor::new(&self.sql_preview_input)
                    .size_full()
                    .readonly(true),
            )
            .into_any_element()
    }

    fn render_ddl_preview(&self) -> AnyElement {
        v_flex()
            .size_full()
            .p_4()
            .gap_3()
            .child(
                Editor::new(&self.ddl_preview_input)
                    .size_full()
                    .readonly(true),
            )
            .into_any_element()
    }

    /// 预览类页签的复制入口，摆在工具栏「保存」旁边。
    ///
    /// 编辑器虽然是只读的（可以选中、复制），但整段 SQL 一键取走更省事；
    /// 没有可复制的 SQL 时置灰，避免复制出空内容。
    fn render_copy_action(
        &self,
        selector: &'static str,
        source: &Entity<EditorState>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let source = source.clone();
        let can_copy = !source.read(cx).text().to_string().trim().is_empty();

        div()
            .debug_selector(move || selector.to_owned())
            .child(
                Button::new(selector)
                    .small()
                    .ghost()
                    .icon(IconName::Copy)
                    .label(t!("Table.copy_sql").to_string())
                    .tooltip(t!("Table.copy_sql").to_string())
                    .disabled(!can_copy)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.copy_preview_sql(&source, window, cx);
                    })),
            )
            .into_any_element()
    }

    /// 把预览页签里生成好的 SQL 写进剪贴板并给出提示；空内容不写。
    fn copy_preview_sql(
        &self,
        source: &Entity<EditorState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let sql = source.read(cx).text().to_string();
        let Some(item) = preview_sql_to_copy(&sql) else {
            return;
        };

        cx.write_to_clipboard(item);
        window.push_notification(t!("Table.sql_copied").to_string(), cx);
    }
}

impl Focusable for TableDesigner {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TableDesigner {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context(DB_SEARCH_CONTEXT)
            .on_action(cx.listener(Self::on_action_refresh_database_panel))
            .on_action(cx.listener(Self::on_action_save_database_changes))
            .child(self.render_toolbar(cx))
            .child(self.render_tabs(cx))
            .child(div().flex_1().w_full().overflow_hidden().child(
                if self.structure_load.is_loading() {
                    render_structure_loading(cx)
                } else {
                    self.render_active_tab(window, cx)
                },
            ))
    }
}

/// 工具栏复制按钮的调试选择器（供回归测试定位该按钮）。
///
/// 一个按钮服务两个页签：当前页签是 SQL 预览就复制预览，是 DDL 就复制 DDL。
const PREVIEW_COPY_SELECTOR: &str = "table-designer-copy-preview";

/// 表结构加载面板的调试选择器（供回归测试定位该面板）。
const STRUCTURE_LOADING_SELECTOR: &str = "table-designer-structure-loading";

/// 首次打开既有表时，列/索引/表信息还在路上：先给一个明确的等待反馈，
/// 而不是先渲染一个空白表单再被数据填上。
fn render_structure_loading(cx: &App) -> AnyElement {
    v_flex()
        .debug_selector(|| STRUCTURE_LOADING_SELECTOR.to_owned())
        .size_full()
        .items_center()
        .justify_center()
        .gap_3()
        .child(Spinner::new().with_size(Size::Large))
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("Table.loading_structure").to_string()),
        )
        .into_any_element()
}

impl EventEmitter<TableDesignerEvent> for TableDesigner {}

impl EventEmitter<TabContentEvent> for TableDesigner {}

impl TabContent for TableDesigner {
    fn content_key(&self) -> &'static str {
        "TableDesigner"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.title.clone()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::TableDesign.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let has_changes = self.has_unsaved_changes(cx);
        if !has_changes {
            return Task::ready(true);
        }

        let title = self.title.clone();
        let designer_entity = cx.entity().clone();

        let (tx, rx) = oneshot::channel::<bool>();
        let tx = Arc::new(Mutex::new(Some(tx)));

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let tx_save = tx.clone();
            let tx_discard = tx.clone();
            let tx_cancel = tx.clone();
            let designer_entity = designer_entity.clone();

            let footer = DialogFooter::new().children(vec![
                Button::new("cancel")
                    .label(t!("Common.cancel").to_string())
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        if let Some(tx) = tx_cancel.lock().ok().and_then(|mut g| g.take()) {
                            let _ = tx.send(false);
                        }
                    })
                    .into_any_element(),
                Button::new("discard")
                    .label(t!("Common.discard").to_string())
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        if let Some(tx) = tx_discard.lock().ok().and_then(|mut g| g.take()) {
                            let _ = tx.send(true);
                        }
                    })
                    .into_any_element(),
                Button::new("save")
                    .label(t!("Common.save").to_string())
                    .primary()
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        designer_entity.update(cx, |designer, cx| designer.save(window, cx));
                        if let Some(tx) = tx_save.lock().ok().and_then(|mut g| g.take()) {
                            let _ = tx.send(true);
                        }
                    })
                    .into_any_element(),
            ]);

            dialog
                .title(format!("{} {}", t!("Common.close"), title))
                .overlay_closable(false)
                .close_button(false)
                .footer(footer)
                .child(t!("Table.unsaved_changes_prompt").to_string())
        });

        cx.spawn(async move |_handle, _cx| rx.await.unwrap_or(false))
    }
}

/// 批量新增列的一条定义：`name` 必填，类型与长度可选。
#[derive(Clone, Debug, PartialEq, Eq)]
struct BatchColumnSpec {
    name: String,
    /// 用户输入的原始类型文本（可能带长度，如 `VARCHAR(64)`）。
    type_text: Option<String>,
    /// 类型基础名，如 `VARCHAR`。
    data_type: Option<String>,
    /// 括号里的长度/精度文本，如 `64` 或 `10,2`。
    type_args: Option<String>,
}

impl BatchColumnSpec {
    fn new(name: &str, type_text: Option<&str>) -> Self {
        let type_text = type_text.map(str::trim).filter(|text| !text.is_empty());
        let (data_type, type_args) = match type_text {
            Some(text) => match (text.find('('), text.rfind(')')) {
                (Some(start), Some(end)) if end > start => (
                    Some(text[..start].trim().to_string()),
                    Some(text[start + 1..end].trim().to_string()),
                ),
                _ => (Some(text.to_string()), None),
            },
            None => (None, None),
        };
        Self {
            name: name.to_string(),
            type_text: type_text.map(str::to_string),
            data_type,
            type_args,
        }
    }
}

/// 按换行或逗号切分批量列定义，但括号里的逗号（如 `DECIMAL(10,2)`）不切。
fn split_batch_column_lines(input: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (index, ch) in input.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '\n' | ',' | '，' if depth == 0 => {
                lines.push(&input[start..index]);
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    lines.push(&input[start..]);
    lines
}

/// 解析「批量新增列」的输入：一行一条（也接受逗号分隔），
/// 每条形如 `name` 或 `name TYPE(len)`，忽略空行，按列名大小写不敏感去重。
fn parse_batch_column_specs(input: &str) -> Vec<BatchColumnSpec> {
    let mut specs: Vec<BatchColumnSpec> = Vec::new();
    for line in split_batch_column_lines(input) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let (name, type_text) = match line.split_once(char::is_whitespace) {
            Some((name, rest)) => (name.trim(), Some(rest.trim())),
            None => (line, None),
        };
        if name.is_empty()
            || specs
                .iter()
                .any(|existing| existing.name.eq_ignore_ascii_case(name))
        {
            continue;
        }
        specs.push(BatchColumnSpec::new(name, type_text));
    }
    specs
}

/// 只取列名的便捷包装（目前仅测试使用）。
#[cfg(test)]
fn parse_batch_column_names(input: &str) -> Vec<String> {
    parse_batch_column_specs(input)
        .into_iter()
        .map(|spec| spec.name)
        .collect()
}

// === Columns Editor ===

pub enum ColumnsEditorEvent {
    Changed,
    RefreshRequested,
}

/// 拖拽列时的视觉反馈
#[derive(Clone)]
struct DragColumn {
    index: usize,
    name: String,
}

impl DragColumn {
    fn new(index: usize, name: String) -> Self {
        Self { index, name }
    }
}

impl Render for DragColumn {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("drag-column")
            .cursor_grabbing()
            .py_1()
            .px_2()
            .rounded_md()
            .bg(cx.theme().primary.opacity(0.9))
            .text_color(cx.theme().primary_foreground)
            .text_sm()
            .child(if self.name.is_empty() {
                t!("Table.column_number", index = self.index + 1).to_string()
            } else {
                self.name.clone()
            })
    }
}

#[derive(Clone)]
struct ResizeColumnEditorColumn {
    entity_id: EntityId,
    col_ix: usize,
}

impl Render for ResizeColumnEditorColumn {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.0))
    }
}

pub struct ColumnsEditor {
    focus_handle: FocusHandle,
    can_refresh: bool,
    columns: Vec<ColumnEditorRow>,
    selected_index: Option<usize>,
    column_widths: [Pixels; COLUMN_EDITOR_COLUMN_COUNT],
    data_types: Vec<String>,
    charsets: Vec<CharsetInfo>,
    /// 表级默认字符集/排序规则，新增列默认继承（issue #586）。
    table_charset: Option<String>,
    table_collation: Option<String>,
    database_type: DatabaseType,
    column_editor_capabilities: ColumnEditorCapabilities,
    scroll_handle: UniformListScrollHandle,
    search_input: Entity<InputState>,
    search_query: String,
    filtered_indices: Vec<usize>,
    _search_subscription: Subscription,
    _subscriptions: Vec<Subscription>,
}

struct ColumnEditorRow {
    source_name: Option<String>,
    name_input: Entity<InputState>,
    type_select: Entity<SelectState<SearchableVec<String>>>,
    length_input: Entity<InputState>,
    scale_input: Entity<InputState>,
    nullable: bool,
    is_pk: bool,
    auto_increment: bool,
    is_unsigned: bool,
    default_input: Entity<InputState>,
    comment_input: Entity<InputState>,
    charset_select: Entity<SelectState<Vec<CharsetSelectItem>>>,
    collation_select: Entity<SelectState<Vec<CollationSelectItem>>>,
    enum_values_input: Entity<InputState>,
}

impl ColumnsEditor {
    fn on_action_focus_search(
        &mut self,
        _: &FocusSearchInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        focus_search_input(&self.search_input, window, cx);
    }

    pub fn new(
        database_type: DatabaseType,
        charsets: Vec<CharsetInfo>,
        column_editor_capabilities: ColumnEditorCapabilities,
        can_refresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let data_types = Self::get_data_types(&database_type, cx);
        let search_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Table.search_column").to_string())
        });

        let search_sub = cx.subscribe_in(
            &search_input,
            window,
            |this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    this.update_filtered_indices(cx);
                }
            },
        );

        Self {
            focus_handle,
            can_refresh,
            columns: vec![],
            selected_index: None,
            column_widths: COLUMN_EDITOR_DEFAULT_WIDTHS,
            data_types,
            charsets,
            table_charset: None,
            table_collation: None,
            database_type,
            column_editor_capabilities,
            scroll_handle: UniformListScrollHandle::default(),
            search_input,
            search_query: String::new(),
            filtered_indices: vec![],
            _search_subscription: search_sub,
            _subscriptions: vec![],
        }
    }

    /// issue #586：记录表级字符集/排序规则，之后新增的列默认继承它们。
    fn set_table_defaults(&mut self, charset: Option<String>, collation: Option<String>) {
        let charset = charset.filter(|value| !value.is_empty());
        let collation = collation.filter(|value| !value.is_empty());
        // 服务端可能返回插件静态字符集列表之外的字符集；补进列表新增列才选得中。
        if let Some(charset) = charset.as_deref() {
            if !self.charsets.iter().any(|info| info.name == charset) {
                self.charsets.push(CharsetInfo {
                    name: charset.to_string(),
                    description: charset.to_string(),
                    default_collation: collation.clone().unwrap_or_default(),
                });
            }
        }
        self.table_charset = charset;
        self.table_collation = collation;
    }

    /// 新增列的字符集候选项：表默认字符集存在时直接选中它。
    fn new_column_charset_selection(&self) -> (Vec<CharsetSelectItem>, usize) {
        let items: Vec<CharsetSelectItem> = std::iter::once(CharsetSelectItem {
            info: CharsetInfo {
                name: "".to_string(),
                description: t!("Table.default").to_string(),
                default_collation: "".to_string(),
            },
        })
        .chain(
            self.charsets
                .iter()
                .cloned()
                .map(|info| CharsetSelectItem { info }),
        )
        .collect();
        let selected = self
            .table_charset
            .as_deref()
            .and_then(|charset| items.iter().position(|item| item.info.name == charset))
            .unwrap_or(0);
        (items, selected)
    }

    /// 新增列的排序规则候选项：跟随表默认字符集，并选中表自己的排序规则。
    fn new_column_collation_selection(&self, cx: &App) -> (Vec<CollationSelectItem>, usize) {
        let Some(charset) = self.table_charset.as_deref() else {
            return (
                vec![CollationSelectItem {
                    info: CollationInfo {
                        name: "".to_string(),
                        charset: "".to_string(),
                        is_default: true,
                    },
                }],
                0,
            );
        };
        let global_state = cx.global::<GlobalDbState>();
        let plugin = global_state.db_manager.get_plugin(&self.database_type).ok();
        Self::build_existing_collation_selection(
            plugin.as_deref(),
            charset,
            self.table_collation.as_deref(),
        )
    }

    fn resize_column_width(
        widths: &mut [Pixels; COLUMN_EDITOR_COLUMN_COUNT],
        col_ix: usize,
        width: Pixels,
    ) {
        let Some(column_width) = widths.get_mut(col_ix) else {
            return;
        };
        *column_width = width
            .max(COLUMN_EDITOR_MIN_WIDTHS[col_ix])
            .min(COLUMN_EDITOR_MAX_WIDTHS[col_ix]);
    }

    fn column_width(&self, col_ix: usize) -> Pixels {
        self.column_widths
            .get(col_ix)
            .copied()
            .unwrap_or(COLUMN_EDITOR_DEFAULT_WIDTHS[COLUMN_NAME_COL])
    }

    fn update_collation_for_charset(
        database_type: DatabaseType,
        charset_select: &Entity<SelectState<Vec<CharsetSelectItem>>>,
        collation_select: &Entity<SelectState<Vec<CollationSelectItem>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = charset_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        // 空字符串表示 "(default)"，此时排序规则也重置为默认
        if selected.is_empty() {
            collation_select.update(cx, |state, cx| {
                let items = vec![CollationSelectItem {
                    info: CollationInfo {
                        name: "".into(),
                        charset: "".into(),
                        is_default: true,
                    },
                }];
                state.set_items(items, window, cx);
                state.set_selected_index(Some(IndexPath::new(0)), window, cx);
            });
            return;
        }
        let global_state = cx.global::<GlobalDbState>();
        let collations = if let Ok(plugin) = global_state.db_manager.get_plugin(&database_type) {
            plugin.get_collations(&selected)
        } else {
            vec![]
        };
        let items: Vec<CollationSelectItem> = collations
            .into_iter()
            .map(|info| CollationSelectItem { info })
            .collect();
        let default_idx = items.iter().position(|c| c.info.is_default).unwrap_or(0);
        collation_select.update(cx, |state, cx| {
            state.set_items(items, window, cx);
            state.set_selected_index(Some(IndexPath::new(default_idx)), window, cx);
        });
    }

    fn update_filtered_indices(&mut self, cx: &mut Context<Self>) {
        let query = self.search_input.read(cx).text().to_string().to_lowercase();
        self.search_query = query.clone();

        if query.is_empty() {
            self.filtered_indices = (0..self.columns.len()).collect();
        } else {
            self.filtered_indices = self
                .columns
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    let name = row.name_input.read(cx).text().to_string().to_lowercase();
                    name.contains(&query)
                })
                .map(|(idx, _)| idx)
                .collect();

            if !self.filtered_indices.is_empty() {
                self.scroll_handle
                    .scroll_to_item(0, gpui::ScrollStrategy::Top);
                self.selected_index = Some(self.filtered_indices[0]);
            }
        }

        cx.notify();
    }

    fn set_can_refresh(&mut self, can_refresh: bool, cx: &mut Context<Self>) {
        if self.can_refresh == can_refresh {
            return;
        }
        self.can_refresh = can_refresh;
        cx.notify();
    }

    fn get_data_types(database_type: &DatabaseType, cx: &App) -> Vec<String> {
        let global_state = cx.global::<GlobalDbState>();
        if let Ok(plugin) = global_state.db_manager.get_plugin(database_type) {
            plugin
                .get_data_types()
                .iter()
                .map(|(name, _)| name.to_string())
                .collect()
        } else {
            vec![
                "INT".to_string(),
                "VARCHAR".to_string(),
                "TEXT".to_string(),
                "DATE".to_string(),
                "DATETIME".to_string(),
            ]
        }
    }

    fn ensure_loaded_type_option(data_types: &mut Vec<String>, base_type: &str) -> usize {
        if let Some(idx) = data_types
            .iter()
            .position(|t| t.eq_ignore_ascii_case(base_type))
        {
            return idx;
        }

        data_types.push(base_type.to_string());
        data_types.len() - 1
    }

    /// 批量新增列：逐行填列名，类型先在行内用默认值，后续再按需调整（issue #30 #7）。
    fn open_batch_add_columns_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let names_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(t!("Table.batch_add_columns_placeholder").to_string())
                .auto_grow(3, 10)
        });
        let designer = cx.entity();

        window.open_dialog(cx, move |dialog, _window, _cx| {
            let names_input = names_input.clone();
            let designer_ok = designer.clone();
            dialog
                .title(t!("Table.batch_add_columns").to_string())
                .overlay_closable(false)
                .child(
                    v_flex().gap_2().child(Textarea::new(&names_input)).child(
                        div()
                            .text_xs()
                            .child(t!("Table.batch_add_columns_hint").to_string()),
                    ),
                )
                .on_ok(move |_, window, cx| {
                    let specs = parse_batch_column_specs(&names_input.read(cx).text().to_string());
                    if specs.is_empty() {
                        window
                            .push_notification(t!("Table.batch_add_columns_empty").to_string(), cx);
                        return false;
                    }
                    let ignored = designer_ok.update(cx, |designer, cx| {
                        designer.add_columns_from_specs(&specs, window, cx)
                    });
                    if !ignored.is_empty() {
                        window.push_notification(
                            t!(
                                "Table.batch_add_columns_unknown_type",
                                types = ignored.join(", ")
                            )
                            .to_string(),
                            cx,
                        );
                    }
                    true
                })
        });
    }

    /// 按批量定义新增列，返回无法识别的类型文本（用于提示用户手动调整）。
    fn add_columns_from_specs(
        &mut self,
        specs: &[BatchColumnSpec],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<String> {
        let existing: Vec<String> = self
            .columns
            .iter()
            .map(|column| column.name_input.read(cx).value().to_string())
            .collect();
        let mut ignored: Vec<String> = Vec::new();
        for spec in specs {
            if existing
                .iter()
                .any(|current| current.eq_ignore_ascii_case(&spec.name))
            {
                continue;
            }
            let index = self.columns.len();
            self.add_column(window, cx);
            let Some(row) = self.columns.get(index) else {
                continue;
            };
            row.name_input.update(cx, |state, cx| {
                state.set_value(spec.name.as_str(), window, cx)
            });
            let Some(type_text) = spec.type_text.clone() else {
                continue;
            };
            let matched = spec.data_type.as_deref().and_then(|name| {
                self.data_types
                    .iter()
                    .find(|candidate| candidate.eq_ignore_ascii_case(name))
                    .cloned()
            });
            match matched {
                Some(data_type) => {
                    row.type_select.update(cx, |state, cx| {
                        state.set_selected_value(&data_type, window, cx)
                    });
                    if let Some(args) = spec.type_args.as_deref() {
                        let (length, scale) = match args.split_once(',') {
                            Some((length, scale)) => (length.trim(), Some(scale.trim())),
                            None => (args, None),
                        };
                        row.length_input
                            .update(cx, |state, cx| state.set_value(length, window, cx));
                        if let Some(scale) = scale {
                            row.scale_input
                                .update(cx, |state, cx| state.set_value(scale, window, cx));
                        }
                    }
                }
                None => ignored.push(type_text),
            }
        }
        ignored
    }

    fn add_column(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("Table.column_name").to_string()));
        let type_items = SearchableVec::new(self.data_types.clone());
        let type_select = cx.new(|cx| {
            SelectState::new(type_items, Some(IndexPath::new(0)), window, cx).searchable(true)
        });
        let length_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Table.length").to_string()));
        let scale_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Table.decimal_places").to_string())
        });
        let default_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Table.default_value").to_string())
        });
        let comment_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Table.comment").to_string()));

        // issue #586：新增列默认继承表字符集/排序规则，而不是停在「默认」。
        let (charset_items, charset_idx) = self.new_column_charset_selection();
        let charset_select = cx.new(|cx| {
            SelectState::new(charset_items, Some(IndexPath::new(charset_idx)), window, cx)
        });

        let (collation_items, collation_idx) = self.new_column_collation_selection(cx);
        let collation_select = cx.new(|cx| {
            SelectState::new(
                collation_items,
                Some(IndexPath::new(collation_idx)),
                window,
                cx,
            )
        });

        let enum_values_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Table.value_list_hint").to_string())
        });

        let name_sub = cx.subscribe_in(
            &name_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(ColumnsEditorEvent::Changed);
                }
            },
        );
        let length_sub = cx.subscribe_in(
            &length_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(ColumnsEditorEvent::Changed);
                }
            },
        );
        let scale_sub = cx.subscribe_in(
            &scale_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(ColumnsEditorEvent::Changed);
                }
            },
        );
        let default_sub = cx.subscribe_in(
            &default_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(ColumnsEditorEvent::Changed);
                }
            },
        );
        let comment_sub = cx.subscribe_in(
            &comment_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(ColumnsEditorEvent::Changed);
                }
            },
        );
        let type_sub = cx.subscribe_in(
            &type_select,
            window,
            |_this, _, _event: &SelectEvent<SearchableVec<String>>, _window, cx| {
                cx.emit(ColumnsEditorEvent::Changed);
            },
        );
        let charset_select_clone = charset_select.clone();
        let collation_select_clone = collation_select.clone();
        let charset_sub = cx.subscribe_in(
            &charset_select,
            window,
            move |this, _, _event: &SelectEvent<Vec<CharsetSelectItem>>, window, cx| {
                Self::update_collation_for_charset(
                    this.database_type.clone(),
                    &charset_select_clone,
                    &collation_select_clone,
                    window,
                    cx,
                );
                cx.emit(ColumnsEditorEvent::Changed);
            },
        );
        let collation_sub = cx.subscribe_in(
            &collation_select,
            window,
            |_this, _, _event: &SelectEvent<Vec<CollationSelectItem>>, _window, cx| {
                cx.emit(ColumnsEditorEvent::Changed);
            },
        );
        let enum_values_sub = cx.subscribe_in(
            &enum_values_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(ColumnsEditorEvent::Changed);
                }
            },
        );

        self._subscriptions.extend([
            name_sub,
            length_sub,
            scale_sub,
            default_sub,
            comment_sub,
            type_sub,
            charset_sub,
            collation_sub,
            enum_values_sub,
        ]);

        self.columns.push(ColumnEditorRow {
            source_name: None,
            name_input,
            type_select,
            length_input,
            scale_input,
            nullable: true,
            is_pk: false,
            auto_increment: false,
            is_unsigned: false,
            default_input,
            comment_input,
            charset_select,
            collation_select,
            enum_values_input,
        });

        let new_index = self.columns.len() - 1;
        self.selected_index = Some(new_index);
        self.update_filtered_indices(cx);

        if let Some(pos) = self.filtered_indices.iter().position(|&i| i == new_index) {
            self.scroll_handle
                .scroll_to_item(pos, gpui::ScrollStrategy::Top);
        }

        cx.emit(ColumnsEditorEvent::Changed);
        cx.notify();
    }

    fn remove_column(&mut self, cx: &mut Context<Self>) {
        if let Some(idx) = self.selected_index {
            if idx < self.columns.len() {
                self.columns.remove(idx);
                self.selected_index = None;
                self.update_filtered_indices(cx);
                cx.emit(ColumnsEditorEvent::Changed);
                cx.notify();
            }
        }
    }

    fn toggle_nullable(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(col) = self.columns.get_mut(idx) {
            col.nullable = !col.nullable;
            cx.emit(ColumnsEditorEvent::Changed);
            cx.notify();
        }
    }

    fn toggle_pk(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(col) = self.columns.get_mut(idx) {
            col.is_pk = !col.is_pk;
            cx.emit(ColumnsEditorEvent::Changed);
            cx.notify();
        }
    }

    fn toggle_auto_increment(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(col) = self.columns.get_mut(idx) {
            col.auto_increment = !col.auto_increment;
            cx.emit(ColumnsEditorEvent::Changed);
            cx.notify();
        }
    }

    fn move_column(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.columns.len() || to >= self.columns.len() {
            return;
        }
        let column = self.columns.remove(from);
        self.columns.insert(to, column);

        if let Some(selected) = self.selected_index {
            if selected == from {
                self.selected_index = Some(to);
            } else if from < selected && selected <= to {
                self.selected_index = Some(selected - 1);
            } else if to <= selected && selected < from {
                self.selected_index = Some(selected + 1);
            }
        }

        cx.emit(ColumnsEditorEvent::Changed);
        cx.notify();
    }

    pub fn get_columns(&self, cx: &App) -> Vec<ColumnDefinition> {
        self.columns
            .iter()
            .map(|row| {
                let name = row.name_input.read(cx).text().to_string();
                let base_type = row
                    .type_select
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .unwrap_or_else(|| "VARCHAR".to_string());
                let length_str = row.length_input.read(cx).text().to_string();
                let length = length_str.parse::<u32>().ok();
                let scale_str = row.scale_input.read(cx).text().to_string();
                let scale = scale_str.parse::<u32>().ok();

                let data_type = if self.column_editor_capabilities.supports_enum_values
                    && self.is_enum_type(&base_type, cx)
                {
                    let enum_values = row.enum_values_input.read(cx).text().to_string();
                    if enum_values.is_empty() {
                        base_type
                    } else {
                        format!("{}({})", base_type, enum_values)
                    }
                } else {
                    base_type
                };
                let is_unsigned = row.is_unsigned && Self::supports_unsigned_type(&data_type);

                let default_value = {
                    let val = row.default_input.read(cx).text().to_string();
                    if val.is_empty() { None } else { Some(val) }
                };
                let comment = row.comment_input.read(cx).text().to_string();
                let charset = row
                    .charset_select
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .filter(|s| !s.is_empty());
                let collation = row
                    .collation_select
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .filter(|s| !s.is_empty());

                ColumnDefinition {
                    name,
                    data_type,
                    length,
                    precision: None,
                    scale,
                    is_nullable: row.nullable,
                    is_primary_key: row.is_pk,
                    is_auto_increment: row.auto_increment,
                    is_unsigned,
                    default_value,
                    comment,
                    charset,
                    collation,
                }
            })
            .collect()
    }

    fn build_existing_collation_selection(
        plugin: Option<&dyn DatabasePlugin>,
        charset_name: &str,
        current_collation: Option<&str>,
    ) -> (Vec<CollationSelectItem>, usize) {
        let mut items: Vec<CollationSelectItem> = plugin
            .map(|plugin| plugin.get_collations(charset_name))
            .unwrap_or_default()
            .into_iter()
            .map(|info| CollationSelectItem { info })
            .collect();

        if let Some(collation) = current_collation.filter(|value| !value.is_empty()) {
            if !items.iter().any(|item| item.info.name == collation) {
                items.insert(
                    0,
                    CollationSelectItem {
                        info: CollationInfo {
                            name: collation.to_string(),
                            charset: charset_name.to_string(),
                            is_default: false,
                        },
                    },
                );
            }
        }

        if items.is_empty() {
            items.push(CollationSelectItem {
                info: CollationInfo {
                    name: "".into(),
                    charset: charset_name.to_string(),
                    is_default: true,
                },
            });
        }

        let selected_idx = current_collation
            .and_then(|collation| items.iter().position(|item| item.info.name == collation))
            .or_else(|| items.iter().position(|item| item.info.is_default))
            .unwrap_or(0);

        (items, selected_idx)
    }

    pub fn get_column_renames(&self, cx: &App) -> Vec<(String, String)> {
        self.columns
            .iter()
            .filter_map(|row| {
                let old_name = row.source_name.as_ref()?;
                let new_name = row.name_input.read(cx).text().to_string();
                if old_name.is_empty() || new_name.is_empty() || old_name == &new_name {
                    None
                } else {
                    Some((old_name.clone(), new_name))
                }
            })
            .collect()
    }

    fn select_row(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.selected_index = Some(idx);
        cx.notify();
    }

    pub fn load_columns(
        &mut self,
        columns: Vec<ColumnInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.columns.clear();
        self._subscriptions.clear();

        let global_state = cx.global::<GlobalDbState>();
        let plugin = global_state.db_manager.get_plugin(&self.database_type).ok();
        let primary_key_count = columns.iter().filter(|col| col.is_primary_key).count();

        for col in columns {
            let name_input = cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder(t!("Table.column_name").to_string());
                input.set_value(col.name.clone(), window, cx);
                input
            });

            let parsed_type = plugin
                .as_ref()
                .map(|p| p.parse_column_type(&col.data_type))
                .unwrap_or_else(|| fallback_parse_column_type(&col.data_type));
            let base_type = parsed_type.base_type.clone();
            let type_idx = Self::ensure_loaded_type_option(&mut self.data_types, &base_type);
            let select_type_items = SearchableVec::new(self.data_types.clone());
            let type_select = cx.new(|cx| {
                SelectState::new(
                    select_type_items,
                    Some(IndexPath::new(type_idx)),
                    window,
                    cx,
                )
            });

            let length_input = cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder(t!("Table.length").to_string());
                if let Some(len) = Self::extract_length_from_type(&col.data_type) {
                    input.set_value(len.to_string(), window, cx);
                }
                input
            });

            let scale_input = cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder(t!("Table.decimal_places").to_string());
                if let Some(scale) = Self::extract_scale_from_type(&col.data_type) {
                    input.set_value(scale.to_string(), window, cx);
                }
                input
            });

            let default_input = cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder(t!("Table.default_value").to_string());
                if let Some(ref default) = col.default_value {
                    input.set_value(default.clone(), window, cx);
                }
                input
            });

            let comment_input = cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder(t!("Table.comment").to_string());
                if let Some(ref comment) = col.comment {
                    input.set_value(comment.clone(), window, cx);
                }
                input
            });

            let charset_items: Vec<CharsetSelectItem> = std::iter::once(CharsetSelectItem {
                info: CharsetInfo {
                    name: "".to_string(),
                    description: t!("Table.default").to_string(),
                    default_collation: "".to_string(),
                },
            })
            .chain(
                self.charsets
                    .iter()
                    .cloned()
                    .map(|info| CharsetSelectItem { info }),
            )
            .collect();
            // 根据列已有的 charset 查找对应索引
            let charset_idx = col
                .charset
                .as_ref()
                .and_then(|cs| charset_items.iter().position(|item| item.info.name == *cs))
                .unwrap_or(0);
            let charset_select = cx.new(|cx| {
                SelectState::new(charset_items, Some(IndexPath::new(charset_idx)), window, cx)
            });

            // 根据列已有的 charset 加载对应的排序规则列表，并选中已有值
            let collation_select = cx.new(|cx| {
                let (items, selected_idx) = if let Some(ref charset_name) = col.charset {
                    Self::build_existing_collation_selection(
                        plugin.as_deref(),
                        charset_name,
                        col.collation.as_deref(),
                    )
                } else {
                    let items = vec![CollationSelectItem {
                        info: CollationInfo {
                            name: "".into(),
                            charset: "".into(),
                            is_default: true,
                        },
                    }];
                    (items, 0)
                };
                SelectState::new(items, Some(IndexPath::new(selected_idx)), window, cx)
            });

            let enum_values_input = cx.new(|cx| {
                let mut input = InputState::new(window, cx)
                    .placeholder(t!("Table.value_list_hint").to_string());
                if let Some(values) = Self::extract_enum_values(&col.data_type) {
                    input.set_value(values, window, cx);
                }
                input
            });

            let name_sub = cx.subscribe_in(
                &name_input,
                window,
                |_this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        cx.emit(ColumnsEditorEvent::Changed);
                    }
                },
            );
            let length_sub = cx.subscribe_in(
                &length_input,
                window,
                |_this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        cx.emit(ColumnsEditorEvent::Changed);
                    }
                },
            );
            let scale_sub = cx.subscribe_in(
                &scale_input,
                window,
                |_this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        cx.emit(ColumnsEditorEvent::Changed);
                    }
                },
            );
            let default_sub = cx.subscribe_in(
                &default_input,
                window,
                |_this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        cx.emit(ColumnsEditorEvent::Changed);
                    }
                },
            );
            let comment_sub = cx.subscribe_in(
                &comment_input,
                window,
                |_this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        cx.emit(ColumnsEditorEvent::Changed);
                    }
                },
            );
            let type_sub = cx.subscribe_in(
                &type_select,
                window,
                |_this, _, _event: &SelectEvent<SearchableVec<String>>, _window, cx| {
                    cx.emit(ColumnsEditorEvent::Changed);
                },
            );
            let charset_select_clone = charset_select.clone();
            let collation_select_clone = collation_select.clone();
            let charset_sub = cx.subscribe_in(
                &charset_select,
                window,
                move |this, _, _event: &SelectEvent<Vec<CharsetSelectItem>>, window, cx| {
                    Self::update_collation_for_charset(
                        this.database_type.clone(),
                        &charset_select_clone,
                        &collation_select_clone,
                        window,
                        cx,
                    );
                    cx.emit(ColumnsEditorEvent::Changed);
                },
            );
            let collation_sub = cx.subscribe_in(
                &collation_select,
                window,
                |_this, _, _event: &SelectEvent<Vec<CollationSelectItem>>, _window, cx| {
                    cx.emit(ColumnsEditorEvent::Changed);
                },
            );
            let enum_values_sub = cx.subscribe_in(
                &enum_values_input,
                window,
                |_this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        cx.emit(ColumnsEditorEvent::Changed);
                    }
                },
            );

            self._subscriptions.extend([
                name_sub,
                length_sub,
                scale_sub,
                default_sub,
                comment_sub,
                type_sub,
                charset_sub,
                collation_sub,
                enum_values_sub,
            ]);

            self.columns.push(ColumnEditorRow {
                source_name: Some(col.name.clone()),
                name_input,
                type_select,
                length_input,
                scale_input,
                nullable: col.is_nullable,
                is_pk: col.is_primary_key,
                auto_increment: column_is_auto_increment(
                    &self.database_type,
                    &col,
                    &parsed_type,
                    primary_key_count,
                ),
                is_unsigned: parsed_type.is_unsigned,
                default_input,
                comment_input,
                charset_select,
                collation_select,
                enum_values_input,
            });
        }

        self.update_filtered_indices(cx);
        cx.emit(ColumnsEditorEvent::Changed);
        cx.notify();
    }

    fn extract_length_from_type(data_type: &str) -> Option<u32> {
        if let Some(start) = data_type.find('(') {
            if let Some(end) = data_type.find(')') {
                let len_str = &data_type[start + 1..end];
                if let Some(comma) = len_str.find(',') {
                    return len_str[..comma].trim().parse().ok();
                }
                return len_str.trim().parse().ok();
            }
        }
        None
    }

    fn supports_unsigned_type(data_type: &str) -> bool {
        matches!(
            data_type.to_uppercase().as_str(),
            "TINYINT"
                | "SMALLINT"
                | "MEDIUMINT"
                | "INT"
                | "INTEGER"
                | "BIGINT"
                | "DECIMAL"
                | "NUMERIC"
                | "FLOAT"
                | "DOUBLE"
                | "REAL"
        )
    }

    fn extract_scale_from_type(data_type: &str) -> Option<u32> {
        if let Some(start) = data_type.find('(') {
            if let Some(end) = data_type.find(')') {
                let len_str = &data_type[start + 1..end];
                if let Some(comma) = len_str.find(',') {
                    return len_str[comma + 1..].trim().parse().ok();
                }
            }
        }
        None
    }

    fn extract_enum_values(data_type: &str) -> Option<String> {
        let upper = data_type.to_uppercase();
        if !upper.starts_with("ENUM") && !upper.starts_with("SET") {
            return None;
        }
        if let Some(start) = data_type.find('(') {
            if let Some(end) = data_type.rfind(')') {
                let values = &data_type[start + 1..end];
                return Some(values.to_string());
            }
        }
        None
    }

    fn is_enum_type(&self, data_type: &str, cx: &App) -> bool {
        let global_state = cx.global::<GlobalDbState>();
        if let Ok(plugin) = global_state.db_manager.get_plugin(&self.database_type) {
            plugin.is_enum_type(data_type)
        } else {
            false
        }
    }

    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        h_flex()
            .gap_1()
            .items_center()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                IconButton::new("add-col", IconName::Plus)
                    .hit_size(Size::Small)
                    .glyph_size(OneIconSize::Default)
                    .tooltip(t!("Table.add_column").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.add_column(window, cx))),
            )
            .child(
                IconButton::new("add-cols-batch", IconName::ListChecks)
                    .hit_size(Size::Small)
                    .glyph_size(OneIconSize::Default)
                    .tooltip(t!("Table.batch_add_columns").to_string())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_batch_add_columns_dialog(window, cx)
                    })),
            )
            .child(
                IconButton::new("remove-col", IconName::Minus)
                    .hit_size(Size::Small)
                    .glyph_size(OneIconSize::Default)
                    .tooltip(t!("Table.delete_column").to_string())
                    .on_click(cx.listener(|this, _, _window, cx| this.remove_column(cx))),
            )
            .when(self.can_refresh, |this| {
                this.child(
                    IconButton::new("refresh-columns", IconName::Refresh)
                        .hit_size(Size::Small)
                        .glyph_size(OneIconSize::Default)
                        .tooltip(t!("Table.refresh_structure").to_string())
                        .accessible_label(t!("Table.refresh_structure").to_string())
                        .on_click(cx.listener(|_, _, _, cx| {
                            cx.emit(ColumnsEditorEvent::RefreshRequested);
                        })),
                )
            })
            .child(div().flex_1())
            .child(
                Input::new(&self.search_input)
                    .w(px(200.))
                    .prefix(
                        Icon::new(IconName::Search)
                            .with_size(Size::Small)
                            .text_color(cx.theme().muted_foreground),
                    )
                    .cleanable(true),
            )
            .into_any_element()
    }

    fn render_table_header(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_3()
            .px_3()
            .py_2()
            .bg(cx.theme().muted.opacity(0.5))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(div().w(COLUMN_DRAG_HANDLE_WIDTH))
            .child(self.render_header_cell(
                COLUMN_NAME_COL,
                t!("Table.column_name").to_string(),
                false,
                cx,
            ))
            .child(self.render_header_cell(
                COLUMN_TYPE_COL,
                t!("Table.type").to_string(),
                false,
                cx,
            ))
            .child(self.render_header_cell(
                COLUMN_LENGTH_COL,
                t!("Table.length").to_string(),
                false,
                cx,
            ))
            .child(self.render_header_cell(
                COLUMN_SCALE_COL,
                t!("Table.decimal_places").to_string(),
                false,
                cx,
            ))
            .child(self.render_header_cell(
                COLUMN_NULLABLE_COL,
                t!("Table.nullable").to_string(),
                true,
                cx,
            ))
            .child(self.render_header_cell(
                COLUMN_PRIMARY_KEY_COL,
                t!("Table.primary_key").to_string(),
                true,
                cx,
            ))
            .child(self.render_header_cell(
                COLUMN_AUTO_INCREMENT_COL,
                t!("Table.auto_increment_column").to_string(),
                true,
                cx,
            ))
            .child(self.render_header_cell(
                COLUMN_DEFAULT_COL,
                t!("Table.default_value").to_string(),
                false,
                cx,
            ))
            .child(
                div()
                    .flex_1()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Table.comment").to_string()),
            )
            .into_any_element()
    }

    fn render_header_cell(
        &self,
        col_ix: usize,
        label: String,
        centered: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .relative()
            .w(self.column_width(col_ix))
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .when(centered, |this| this.text_center())
            .child(label)
            .child(self.render_column_resize_handle(col_ix, cx))
            .into_any_element()
    }

    /// 第 `col_ix` 列右缘的列宽抓取区：以列边界（本单元格右缘）为中心、两侧各
    /// [`COLUMN_EDITOR_RESIZE_GRAB_PADDING`]。
    ///
    /// 表头单元格之间隔着 `gap_3`，所以这里不能像相邻列紧贴的表格那样左右各挂一半，
    /// 而是让抓取区自己骑到边界上：往右多铺半格，落在那段空隙里，不会压到右侧单元格。
    fn render_column_resize_handle(&self, col_ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let group_id = SharedString::from(format!("column-editor-resize:{col_ix}"));
        self.column_resize_band(format!("column-editor-resize:{col_ix}"), col_ix, cx)
            .debug_selector(move || format!("column-editor-resize-{col_ix}"))
            .right_0()
            .mr(-COLUMN_EDITOR_RESIZE_GRAB_PADDING)
            .w(COLUMN_EDITOR_RESIZE_GRAB_PADDING * 2.)
            .group(group_id.clone())
            .justify_center()
            .child(
                div()
                    .h_full()
                    .w(COLUMN_EDITOR_RESIZE_LINE_WIDTH)
                    .bg(cx.theme().border.opacity(0.4))
                    .group_hover(&group_id, |el| el.bg(cx.theme().primary)),
            )
            .into_any_element()
    }

    /// 一侧抓取区的交互核心（命中、光标、拖动）：位置与外观由调用方决定。
    ///
    /// 抓取区以列边界为中心、两侧各 [`COLUMN_EDITOR_RESIZE_GRAB_PADDING`]。以前是一条
    /// 6px、整条缩在左列右缘里的抓取区：从边界右侧靠过来完全没有反馈，鼠标得精确停在
    /// 那几个像素上才拉得动。
    fn column_resize_band(
        &self,
        id: String,
        boundary_col_ix: usize,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        div()
            .id(id)
            .absolute()
            .top_0()
            .bottom_0()
            .w(COLUMN_EDITOR_RESIZE_GRAB_PADDING)
            .flex()
            .cursor_col_resize()
            .occlude()
            .on_drag_move(cx.listener(
                move |this, e: &DragMoveEvent<ResizeColumnEditorColumn>, _window, cx| {
                    let drag = e.drag(cx);
                    if drag.entity_id != cx.entity_id() || drag.col_ix != boundary_col_ix {
                        return;
                    }

                    let width = this.column_width(boundary_col_ix);
                    let delta = e.event.position.x - e.bounds.center().x;
                    Self::resize_column_width(
                        &mut this.column_widths,
                        boundary_col_ix,
                        width + delta,
                    );
                    cx.notify();
                },
            ))
            .on_drag(
                ResizeColumnEditorColumn {
                    entity_id: cx.entity_id(),
                    col_ix: boundary_col_ix,
                },
                |drag, _, _, cx| {
                    cx.stop_propagation();
                    cx.new(|_| drag.clone())
                },
            )
    }

    fn render_row(&self, idx: usize, row: &ColumnEditorRow, cx: &Context<Self>) -> AnyElement {
        let is_selected = self.selected_index == Some(idx);
        let name = row.name_input.read(cx).text().to_string();
        let drag_border_color = cx.theme().primary;

        h_flex()
            .id(("col-row", idx))
            .w_full()
            .gap_3()
            .px_3()
            .py_1p5()
            .when(is_selected, |this| {
                this.bg(cx.theme().primary.opacity(SELECTED_ROW_BACKGROUND_OPACITY))
            })
            .when_some(
                row_hover_background(is_selected, cx.theme()),
                |this, background| this.hover(move |style| style.bg(background)),
            )
            .border_b_1()
            .border_color(cx.theme().border.opacity(0.5))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _window, cx| {
                    this.select_row(idx, cx);
                }),
            )
            .drag_over::<DragColumn>(move |el, _, _, _cx| {
                el.border_t_2().border_color(drag_border_color)
            })
            .on_drop(cx.listener(move |this, drag: &DragColumn, _window, cx| {
                this.move_column(drag.index, idx, cx);
            }))
            .child(
                div()
                    .id(("col-row-drag-handle", idx))
                    .w(COLUMN_DRAG_HANDLE_WIDTH)
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_grab()
                    .on_drag(DragColumn::new(idx, name), |drag, _, _, cx| {
                        cx.stop_propagation();
                        cx.new(|_| drag.clone())
                    })
                    .child(
                        Icon::new(IconName::Menu)
                            .with_size(Size::Small)
                            .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                div()
                    .w(self.column_width(COLUMN_NAME_COL))
                    .child(Input::new(&row.name_input).w_full()),
            )
            .child(
                div()
                    .w(self.column_width(COLUMN_TYPE_COL))
                    .child(Select::new(&row.type_select).w_full()),
            )
            .child(
                div()
                    .w(self.column_width(COLUMN_LENGTH_COL))
                    .child(Input::new(&row.length_input).w_full()),
            )
            .child(
                div()
                    .w(self.column_width(COLUMN_SCALE_COL))
                    .child(Input::new(&row.scale_input).w_full()),
            )
            .child(
                div()
                    .w(self.column_width(COLUMN_NULLABLE_COL))
                    .flex()
                    .justify_center()
                    .child(Checkbox::new(("null", idx)).checked(row.nullable).on_click(
                        cx.listener(move |this, _, _window, cx| this.toggle_nullable(idx, cx)),
                    )),
            )
            .child(
                div()
                    .w(self.column_width(COLUMN_PRIMARY_KEY_COL))
                    .flex()
                    .justify_center()
                    .child(Checkbox::new(("pk", idx)).checked(row.is_pk).on_click(
                        cx.listener(move |this, _, _window, cx| this.toggle_pk(idx, cx)),
                    )),
            )
            .child(
                div()
                    .w(self.column_width(COLUMN_AUTO_INCREMENT_COL))
                    .flex()
                    .justify_center()
                    .child(
                        Checkbox::new(("ai", idx))
                            .checked(row.auto_increment)
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.toggle_auto_increment(idx, cx)
                            })),
                    ),
            )
            .child(
                div()
                    .w(self.column_width(COLUMN_DEFAULT_COL))
                    .child(Input::new(&row.default_input).w_full()),
            )
            .child(
                div()
                    .flex_1()
                    .child(Input::new(&row.comment_input).w_full()),
            )
            .into_any_element()
    }

    /// 详情面板只剩「列表行放不下」的字段：默认值已经在列表列里直接编辑，
    /// 这里只留字符集、排序规则和枚举可选值。某个库三者都没有时（SQL Server、
    /// Oracle 等）整个面板不渲染，避免底部留下一条空白带子挤压列表。
    fn render_detail_panel(&self, cx: &Context<Self>) -> AnyElement {
        let has_detail_fields = self.column_editor_capabilities.show_charset_in_detail
            || self.column_editor_capabilities.show_collation_in_detail
            || self.column_editor_capabilities.supports_enum_values;
        if !has_detail_fields {
            return div().into_any_element();
        }

        let Some(idx) = self.selected_index else {
            return v_flex()
                .items_center()
                .justify_center()
                .h(px(180.))
                .border_t_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Table.select_column_hint").to_string()),
                )
                .into_any_element();
        };

        let Some(row) = self.columns.get(idx) else {
            return div().into_any_element();
        };

        let selected_type = row
            .type_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        let is_enum_or_set = self.column_editor_capabilities.supports_enum_values
            && self.is_enum_type(&selected_type, cx);
        let show_charset = self.column_editor_capabilities.show_charset_in_detail;
        let show_collation = self.column_editor_capabilities.show_collation_in_detail;
        // 支持枚举值但当前类型不是 enum/set 时也没有可填的字段。
        if !show_charset && !show_collation && !is_enum_or_set {
            return div().into_any_element();
        }

        v_flex()
            .w_full()
            .items_center()
            .justify_center()
            .h(px(180.))
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .p_3()
            .gap_3()
            .when(show_charset, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .w(px(70.))
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{}:", t!("Table.charset"))),
                        )
                        .child(Select::new(&row.charset_select).w(px(200.))),
                )
            })
            .when(show_collation, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .w(px(70.))
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{}:", t!("Table.collation"))),
                        )
                        .child(Select::new(&row.collation_select).w(px(200.))),
                )
            })
            .when(is_enum_or_set, |this| {
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!("{}:", t!("Table.value"))),
                        )
                        .child(Input::new(&row.enum_values_input).w(px(400.))),
                )
            })
            .into_any_element()
    }
}

impl EventEmitter<ColumnsEditorEvent> for ColumnsEditor {}

impl Focusable for ColumnsEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ColumnsEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered_count = self.filtered_indices.len();
        let scroll_handle = self.scroll_handle.clone();

        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context(DB_SEARCH_CONTEXT)
            .on_action(cx.listener(Self::on_action_focus_search))
            .child(self.render_header(cx))
            .child(self.render_table_header(cx))
            .child(
                div()
                    .id("columns-list-container")
                    .flex_1()
                    .overflow_hidden()
                    .relative()
                    .child(
                        uniform_list("columns-list", filtered_count, {
                            cx.processor(move |editor, visible_range: Range<usize>, _window, cx| {
                                visible_range
                                    .filter_map(|pos| {
                                        let actual_idx =
                                            editor.filtered_indices.get(pos).copied()?;
                                        let row = editor.columns.get(actual_idx)?;
                                        Some(editor.render_row(actual_idx, row, cx))
                                    })
                                    .collect::<Vec<_>>()
                            })
                        })
                        .flex_grow_1()
                        .size_full()
                        .track_scroll(&scroll_handle)
                        .with_sizing_behavior(ListSizingBehavior::Auto)
                        .into_any_element(),
                    )
                    .child(Scrollbar::vertical(&self.scroll_handle)),
            )
            .child(self.render_detail_panel(cx))
    }
}

// === Indexes Editor ===

pub enum IndexesEditorEvent {
    Changed,
    RefreshRequested,
}

pub struct IndexesEditor {
    focus_handle: FocusHandle,
    can_refresh: bool,
    indexes: Vec<IndexEditorRow>,
    selected_index: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

struct IndexEditorRow {
    name_input: Entity<InputState>,
    columns_input: Entity<InputState>,
    is_unique: bool,
}

impl IndexesEditor {
    pub fn new(can_refresh: bool, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        Self {
            focus_handle,
            can_refresh,
            indexes: vec![],
            selected_index: None,
            _subscriptions: vec![],
        }
    }

    fn set_can_refresh(&mut self, can_refresh: bool, cx: &mut Context<Self>) {
        if self.can_refresh == can_refresh {
            return;
        }
        self.can_refresh = can_refresh;
        cx.notify();
    }

    fn add_index(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("Table.index_name").to_string()));
        let columns_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Table.columns_comma_separated").to_string())
        });

        // Subscribe to input changes
        let name_sub = cx.subscribe_in(
            &name_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(IndexesEditorEvent::Changed);
                }
            },
        );
        let columns_sub = cx.subscribe_in(
            &columns_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(IndexesEditorEvent::Changed);
                }
            },
        );

        self._subscriptions.extend([name_sub, columns_sub]);

        self.indexes.push(IndexEditorRow {
            name_input,
            columns_input,
            is_unique: false,
        });

        cx.emit(IndexesEditorEvent::Changed);
        cx.notify();
    }

    fn remove_index(&mut self, cx: &mut Context<Self>) {
        if let Some(idx) = self.selected_index {
            if idx < self.indexes.len() {
                self.indexes.remove(idx);
                self.selected_index = None;
                cx.emit(IndexesEditorEvent::Changed);
                cx.notify();
            }
        }
    }

    fn toggle_unique(&mut self, idx: usize, cx: &mut Context<Self>) {
        if let Some(index) = self.indexes.get_mut(idx) {
            index.is_unique = !index.is_unique;
            cx.emit(IndexesEditorEvent::Changed);
            cx.notify();
        }
    }

    fn select_row(&mut self, idx: usize, cx: &mut Context<Self>) {
        self.selected_index = Some(idx);
        cx.notify();
    }

    pub fn get_indexes(&self, cx: &App) -> Vec<IndexDefinition> {
        self.indexes
            .iter()
            .map(|row| {
                let name = row.name_input.read(cx).text().to_string();
                let columns_str = row.columns_input.read(cx).text().to_string();
                let columns: Vec<String> = columns_str
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                IndexDefinition {
                    name,
                    columns,
                    is_unique: row.is_unique,
                    is_primary: false,
                    index_type: None,
                    comment: String::new(),
                }
            })
            .collect()
    }

    pub fn load_indexes(
        &mut self,
        indexes: Vec<IndexInfo>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.indexes.clear();
        self._subscriptions.clear();

        for idx in indexes {
            if idx.is_primary || idx.name.to_uppercase() == "PRIMARY" {
                continue;
            }

            let name_input = cx.new(|cx| {
                let mut input =
                    InputState::new(window, cx).placeholder(t!("Table.index_name").to_string());
                input.set_value(idx.name.clone(), window, cx);
                input
            });

            let columns_str = idx.columns.join(", ");
            let columns_input = cx.new(|cx| {
                let mut input = InputState::new(window, cx)
                    .placeholder(t!("Table.columns_comma_separated").to_string());
                input.set_value(columns_str, window, cx);
                input
            });

            let name_sub = cx.subscribe_in(
                &name_input,
                window,
                |_this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        cx.emit(IndexesEditorEvent::Changed);
                    }
                },
            );
            let columns_sub = cx.subscribe_in(
                &columns_input,
                window,
                |_this, _, event: &InputEvent, _window, cx| {
                    if let InputEvent::Change = event {
                        cx.emit(IndexesEditorEvent::Changed);
                    }
                },
            );

            self._subscriptions.extend([name_sub, columns_sub]);

            self.indexes.push(IndexEditorRow {
                name_input,
                columns_input,
                is_unique: idx.is_unique,
            });
        }

        cx.emit(IndexesEditorEvent::Changed);
        cx.notify();
    }
}

impl EventEmitter<IndexesEditorEvent> for IndexesEditor {}

impl Focusable for IndexesEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for IndexesEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .px_3()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        IconButton::new("add-idx", IconName::Plus)
                            .hit_size(Size::Small)
                            .glyph_size(OneIconSize::Default)
                            .tooltip(t!("Table.add_index").to_string())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_index(window, cx)),
                            ),
                    )
                    .child(
                        IconButton::new("remove-idx", IconName::Minus)
                            .hit_size(Size::Small)
                            .glyph_size(OneIconSize::Default)
                            .tooltip(t!("Table.delete_index").to_string())
                            .on_click(cx.listener(|this, _, _window, cx| this.remove_index(cx))),
                    )
                    .when(self.can_refresh, |this| {
                        this.child(
                            IconButton::new("refresh-indexes", IconName::Refresh)
                                .hit_size(Size::Small)
                                .glyph_size(OneIconSize::Default)
                                .tooltip(t!("Table.refresh_structure").to_string())
                                .accessible_label(t!("Table.refresh_structure").to_string())
                                .on_click(cx.listener(|_, _, _, cx| {
                                    cx.emit(IndexesEditorEvent::RefreshRequested);
                                })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .gap_3()
                    .px_3()
                    .py_2()
                    .bg(cx.theme().muted.opacity(0.5))
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .w(px(160.))
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Table.index_name").to_string()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Table.columns").to_string()),
                    )
                    .child(
                        div()
                            .w(px(60.))
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .text_center()
                            .child(t!("Table.unique").to_string()),
                    ),
            )
            .child(
                v_flex()
                    .id("indexes-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .children(self.indexes.iter().enumerate().map(|(idx, row)| {
                        let is_selected = self.selected_index == Some(idx);
                        h_flex()
                            .id(("idx-row", idx))
                            .gap_3()
                            .px_3()
                            .py_1p5()
                            .when(is_selected, |this| {
                                this.bg(cx.theme().primary.opacity(SELECTED_ROW_BACKGROUND_OPACITY))
                            })
                            .when_some(
                                row_hover_background(is_selected, cx.theme()),
                                |this, background| this.hover(move |style| style.bg(background)),
                            )
                            .border_b_1()
                            .border_color(cx.theme().border.opacity(0.5))
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _, _window, cx| {
                                    this.select_row(idx, cx);
                                }),
                            )
                            .child(
                                div()
                                    .w(px(160.))
                                    .child(Input::new(&row.name_input).w_full()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .child(Input::new(&row.columns_input).w_full()),
                            )
                            .child(
                                div().w(px(60.)).flex().justify_center().child(
                                    Checkbox::new(("unique", idx))
                                        .checked(row.is_unique)
                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                            this.toggle_unique(idx, cx)
                                        })),
                                ),
                            )
                    })),
            )
    }
}

// === Table Options Editor ===

pub enum TableOptionsEvent {
    Changed,
}

#[derive(Clone, Debug)]
pub struct EngineSelectItem {
    pub name: String,
}

impl SelectItem for EngineSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.name
    }
}

#[derive(Clone, Debug)]
pub struct CharsetSelectItem {
    pub info: CharsetInfo,
}

impl SelectItem for CharsetSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        format!("{} - {}", self.info.name, self.info.description).into()
    }

    fn value(&self) -> &Self::Value {
        &self.info.name
    }
}

#[derive(Clone, Debug)]
pub struct CollationSelectItem {
    pub info: CollationInfo,
}

impl SelectItem for CollationSelectItem {
    type Value = String;

    fn title(&self) -> SharedString {
        if self.info.is_default {
            format!("{} (default)", self.info.name).into()
        } else {
            self.info.name.clone().into()
        }
    }

    fn value(&self) -> &Self::Value {
        &self.info.name
    }
}

pub struct TableOptionsEditor {
    focus_handle: FocusHandle,
    _database_type: DatabaseType,
    engine_select: Entity<SelectState<Vec<EngineSelectItem>>>,
    charset_select: Entity<SelectState<Vec<CharsetSelectItem>>>,
    collation_select: Entity<SelectState<Vec<CollationSelectItem>>>,
    comment_input: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl TableOptionsEditor {
    pub fn new(database_type: DatabaseType, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();

        let engines = vec![
            EngineSelectItem {
                name: "InnoDB".to_string(),
            },
            EngineSelectItem {
                name: "MyISAM".to_string(),
            },
            EngineSelectItem {
                name: "MEMORY".to_string(),
            },
        ];
        let engine_select =
            cx.new(|cx| SelectState::new(engines, Some(IndexPath::new(0)), window, cx));

        let charsets = Self::get_charsets(&database_type, cx);
        let charset_items: Vec<CharsetSelectItem> = charsets
            .iter()
            .cloned()
            .map(|info| CharsetSelectItem { info })
            .collect();
        let charset_select =
            cx.new(|cx| SelectState::new(charset_items, Some(IndexPath::new(0)), window, cx));

        let default_charset = charsets
            .first()
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "utf8mb4".to_string());
        let collations = Self::get_collations(&database_type, &default_charset, cx);
        let collation_items: Vec<CollationSelectItem> = collations
            .iter()
            .cloned()
            .map(|info| CollationSelectItem { info })
            .collect();
        let default_coll_idx = collation_items
            .iter()
            .position(|c| c.info.is_default)
            .unwrap_or(0);
        let collation_select = cx.new(|cx| {
            SelectState::new(
                collation_items,
                Some(IndexPath::new(default_coll_idx)),
                window,
                cx,
            )
        });

        let comment_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("Table.table_comment").to_string())
        });

        let engine_sub = cx.observe_in(&engine_select, window, |_this, _, _window, cx| {
            cx.emit(TableOptionsEvent::Changed);
        });
        let charset_select_clone = charset_select.clone();
        let collation_select_clone = collation_select.clone();
        let charset_sub = cx.observe_in(&charset_select, window, move |this, _, window, cx| {
            this.update_collations_for_charset(
                &charset_select_clone,
                &collation_select_clone,
                window,
                cx,
            );
            cx.emit(TableOptionsEvent::Changed);
        });
        let collation_sub = cx.observe_in(&collation_select, window, |_this, _, _window, cx| {
            cx.emit(TableOptionsEvent::Changed);
        });
        let comment_sub = cx.subscribe_in(
            &comment_input,
            window,
            |_this, _, event: &InputEvent, _window, cx| {
                if let InputEvent::Change = event {
                    cx.emit(TableOptionsEvent::Changed);
                }
            },
        );

        Self {
            focus_handle,
            _database_type: database_type,
            engine_select,
            charset_select,
            collation_select,
            comment_input,
            _subscriptions: vec![engine_sub, charset_sub, collation_sub, comment_sub],
        }
    }

    fn get_charsets(database_type: &DatabaseType, cx: &App) -> Vec<CharsetInfo> {
        let global_state = cx.global::<GlobalDbState>();
        if let Ok(plugin) = global_state.db_manager.get_plugin(database_type) {
            plugin.get_charsets()
        } else {
            vec![CharsetInfo {
                name: "utf8mb4".to_string(),
                description: "UTF-8 Unicode".to_string(),
                default_collation: "utf8mb4_general_ci".to_string(),
            }]
        }
    }

    fn get_collations(database_type: &DatabaseType, charset: &str, cx: &App) -> Vec<CollationInfo> {
        let global_state = cx.global::<GlobalDbState>();
        if let Ok(plugin) = global_state.db_manager.get_plugin(database_type) {
            plugin.get_collations(charset)
        } else {
            vec![CollationInfo {
                name: "utf8mb4_general_ci".to_string(),
                charset: "utf8mb4".to_string(),
                is_default: true,
            }]
        }
    }

    fn update_collations_for_charset(
        &self,
        charset_select: &Entity<SelectState<Vec<CharsetSelectItem>>>,
        collation_select: &Entity<SelectState<Vec<CollationSelectItem>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected_charset = charset_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_else(|| "utf8mb4".to_string());

        let collations = Self::get_collations(&self._database_type, &selected_charset, cx);
        let collation_items: Vec<CollationSelectItem> = collations
            .into_iter()
            .map(|info| CollationSelectItem { info })
            .collect();
        let default_idx = collation_items
            .iter()
            .position(|c| c.info.is_default)
            .unwrap_or(0);

        collation_select.update(cx, |state, cx| {
            state.set_items(collation_items, window, cx);
            state.set_selected_index(Some(IndexPath::new(default_idx)), window, cx);
        });
    }

    pub fn get_options(&self, cx: &App) -> TableOptions {
        let engine = self.engine_select.read(cx).selected_value().cloned();
        let charset = self.charset_select.read(cx).selected_value().cloned();
        let collation = self.collation_select.read(cx).selected_value().cloned();
        let comment = self.comment_input.read(cx).text().to_string();

        TableOptions {
            engine,
            charset,
            collation,
            comment,
            auto_increment: None,
        }
    }
}

impl EventEmitter<TableOptionsEvent> for TableOptionsEditor {}

impl Focusable for TableOptionsEditor {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TableOptionsEditor {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        v_flex().size_full().p_4().gap_4().child(
            h_form()
                .with_size(Size::Small)
                .columns(1)
                .label_width(px(80.))
                .child(
                    field()
                        .label(t!("Table.engine").to_string())
                        .items_center()
                        .child(Select::new(&self.engine_select).w(px(200.))),
                )
                .child(
                    field()
                        .label(t!("Table.charset").to_string())
                        .items_center()
                        .child(Select::new(&self.charset_select).w(px(200.))),
                )
                .child(
                    field()
                        .label(t!("Table.collation").to_string())
                        .items_center()
                        .child(Select::new(&self.collation_select).w(px(200.))),
                )
                .child(
                    field()
                        .label(t!("Table.table_comment").to_string())
                        .items_center()
                        .child(Input::new(&self.comment_input).w(px(300.))),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::{
        mssql::MsSqlPlugin, mysql::MySqlPlugin, oracle::OraclePlugin, plugin::DatabasePlugin,
        postgresql::PostgresPlugin, sqlite::SqlitePlugin,
    };
    use gpui_component::{Theme, ThemeColor};
    use std::{cell::Cell, rc::Rc};

    /// 表设计器 DDL 断言仅覆盖仍带原生插件的数据库类型。
    ///
    /// ClickHouse/TDengine 原生插件已移除(分别改由对应 IPC 外部驱动提供,
    /// DDL 行为由扩展侧覆盖),因此这两类与 `External` 一同排除在本组测试之外。
    fn design_test_database_types() -> Vec<DatabaseType> {
        DatabaseType::all()
            .into_iter()
            .filter(|database_type| {
                !matches!(
                    database_type,
                    DatabaseType::ClickHouse | DatabaseType::TDengine
                )
            })
            .cloned()
            .collect()
    }

    #[test]
    fn column_editor_resize_updates_width_with_bounds() {
        let mut widths = COLUMN_EDITOR_DEFAULT_WIDTHS;

        ColumnsEditor::resize_column_width(&mut widths, COLUMN_NAME_COL, px(260.0));
        assert_eq!(px(260.0), widths[COLUMN_NAME_COL]);

        ColumnsEditor::resize_column_width(&mut widths, COLUMN_NAME_COL, px(8.0));
        assert_eq!(
            COLUMN_EDITOR_MIN_WIDTHS[COLUMN_NAME_COL],
            widths[COLUMN_NAME_COL]
        );

        ColumnsEditor::resize_column_width(&mut widths, COLUMN_NAME_COL, px(900.0));
        assert_eq!(
            COLUMN_EDITOR_MAX_WIDTHS[COLUMN_NAME_COL],
            widths[COLUMN_NAME_COL]
        );
    }

    #[test]
    fn column_editor_resize_ignores_invalid_column_index() {
        let mut widths = COLUMN_EDITOR_DEFAULT_WIDTHS;

        ColumnsEditor::resize_column_width(&mut widths, COLUMN_EDITOR_COLUMN_COUNT, px(260.0));

        assert_eq!(COLUMN_EDITOR_DEFAULT_WIDTHS, widths);
    }

    #[test]
    fn hovering_a_selected_row_keeps_the_selection_highlight() {
        let theme = Theme::from(ThemeColor::dark().as_ref());

        // 选中行不叠加悬停底色：悬停样式在基础样式之后 refine，一旦叠加就会
        // 把选中底色盖掉，看起来像「鼠标移到这一行，高亮没了」。
        assert_eq!(None, row_hover_background(true, &theme));
        assert_eq!(
            Some(theme.muted.opacity(HOVERED_ROW_BACKGROUND_OPACITY)),
            row_hover_background(false, &theme)
        );
        assert_ne!(
            row_hover_background(true, &theme),
            row_hover_background(false, &theme)
        );
    }

    fn build_col(name: &str) -> ColumnDefinition {
        ColumnDefinition {
            name: name.to_string(),
            data_type: "INT".to_string(),
            is_nullable: false,
            ..Default::default()
        }
    }

    fn build_design(columns: Vec<ColumnDefinition>, index_columns: Vec<&str>) -> TableDesign {
        let indexes = if index_columns.is_empty() {
            vec![]
        } else {
            vec![IndexDefinition {
                name: "idx_test".to_string(),
                columns: index_columns
                    .into_iter()
                    .map(|name| name.to_string())
                    .collect(),
                is_unique: false,
                is_primary: false,
                index_type: None,
                comment: String::new(),
            }]
        };

        TableDesign {
            database_name: "test_db".to_string(),
            table_name: "users".to_string(),
            columns,
            indexes,
            foreign_keys: vec![],
            options: TableOptions::default(),
        }
    }

    fn build_plugin(database_type: &DatabaseType) -> Box<dyn DatabasePlugin> {
        match database_type {
            DatabaseType::MySQL => Box::new(MySqlPlugin::new()),
            DatabaseType::PostgreSQL => Box::new(PostgresPlugin::new()),
            DatabaseType::SQLite => Box::new(SqlitePlugin::new()),
            #[cfg(feature = "builtin-duckdb")]
            DatabaseType::DuckDB => Box::new(DuckDbPlugin::new()),
            #[cfg(not(feature = "builtin-duckdb"))]
            DatabaseType::DuckDB => Box::new(ExternalDatabasePlugin::new()),
            DatabaseType::MSSQL => Box::new(MsSqlPlugin::new()),
            DatabaseType::Oracle => Box::new(OraclePlugin::new()),
            DatabaseType::ClickHouse | DatabaseType::TDengine | DatabaseType::External { .. } => {
                unreachable!(
                    "ClickHouse/TDengine/External 无原生插件,表设计器 DDL 断言已过滤该类型",
                )
            }
        }
    }

    fn build_delete_and_rename_conflict_case() -> (TableDesign, TableDesign, Vec<(String, String)>)
    {
        let original = build_design(
            vec![build_col("a"), build_col("b"), build_col("c")],
            vec!["b"],
        );
        let current = build_design(vec![build_col("a"), build_col("c")], vec!["a"]);
        let renames = vec![("b".to_string(), "a".to_string())];
        (original, current, renames)
    }

    fn assert_contains_rename_sql(sql: &str, database_type: DatabaseType) {
        match database_type {
            DatabaseType::MySQL => {
                assert!(
                    sql.contains("CHANGE COLUMN"),
                    "MySQL 应使用 CHANGE COLUMN: {sql}"
                );
                assert!(sql.contains("`b`"), "MySQL 重命名 SQL 应包含源列 b: {sql}");
                assert!(
                    sql.contains("`a`"),
                    "MySQL 重命名 SQL 应包含目标列 a: {sql}"
                );
            }
            DatabaseType::PostgreSQL => {
                assert!(
                    sql.contains("RENAME COLUMN \"b\" TO \"a\""),
                    "PostgreSQL 应使用 RENAME COLUMN: {sql}"
                );
            }
            DatabaseType::SQLite | DatabaseType::DuckDB => {
                assert!(
                    sql.contains("RENAME COLUMN \"b\" TO \"a\""),
                    "SQLite/DuckDB 应使用 RENAME COLUMN: {sql}"
                );
            }
            DatabaseType::MSSQL => {
                assert!(
                    sql.contains("EXEC sp_rename '[users].[b]', 'a', 'COLUMN';"),
                    "MSSQL 应使用 sp_rename COLUMN: {sql}"
                );
            }
            DatabaseType::Oracle => {
                assert!(
                    sql.contains("RENAME COLUMN \"b\" TO \"a\""),
                    "Oracle 应使用 RENAME COLUMN: {sql}"
                );
            }
            DatabaseType::ClickHouse => {
                assert!(
                    sql.contains("RENAME COLUMN `b` TO `a`"),
                    "ClickHouse 应使用 RENAME COLUMN: {sql}"
                );
            }
            _ => {}
        }
    }

    fn assert_not_drop_source_column(sql: &str, plugin: &dyn DatabasePlugin) {
        let drop_source = format!("DROP COLUMN {}", plugin.quote_identifier("b"));
        assert!(
            !sql.contains(&drop_source),
            "重命名来源列 b 不应被误删，出现 SQL: {drop_source} in {sql}"
        );
    }

    #[test]
    fn test_preview_refresh_schedule_state_coalesces_requests_in_same_cycle() {
        let mut state = PreviewRefreshScheduleState::default();

        assert!(state.request_refresh(), "第一次请求应当安排一次预览刷新");
        assert!(
            !state.request_refresh(),
            "同一事件周期内的第二次请求不应重复安排刷新"
        );
    }

    #[test]
    fn test_preview_refresh_schedule_state_allows_reschedule_after_finish() {
        let mut state = PreviewRefreshScheduleState::default();

        assert!(state.request_refresh(), "第一次请求应当成功安排刷新");
        state.finish_refresh();
        assert!(
            state.request_refresh(),
            "完成一次刷新后，后续请求应当可以再次安排刷新"
        );
    }

    #[test]
    fn test_contains_destructive_sql_detects_drop_column() {
        let sql = "ALTER TABLE users DROP COLUMN age;";
        assert!(TableDesigner::contains_destructive_sql(sql));
    }

    #[test]
    fn test_contains_destructive_sql_detects_drop_index() {
        let sql = "ALTER TABLE users DROP INDEX idx_users_name;";
        assert!(TableDesigner::contains_destructive_sql(sql));
    }

    #[test]
    fn test_contains_destructive_sql_detects_drop_constraint_and_table() {
        assert!(TableDesigner::contains_destructive_sql(
            "ALTER TABLE users DROP CONSTRAINT users_pk;"
        ));
        assert!(TableDesigner::contains_destructive_sql(
            "DROP TABLE users_backup;"
        ));
    }

    #[test]
    fn test_contains_destructive_sql_ignores_comment_only_drop_keyword() {
        let sql = "-- DROP COLUMN age\nALTER TABLE users ADD COLUMN age INT;";
        assert!(!TableDesigner::contains_destructive_sql(sql));
    }

    #[test]
    fn test_contains_destructive_sql_ignores_non_drop_changes() {
        assert!(!TableDesigner::contains_destructive_sql(
            "ALTER TABLE users ADD COLUMN age INT;"
        ));
        assert!(!TableDesigner::contains_destructive_sql(
            "ALTER TABLE users MODIFY COLUMN age BIGINT;"
        ));
    }

    #[test]
    fn test_contains_destructive_sql_ignores_no_changes_output() {
        assert!(!TableDesigner::contains_destructive_sql(
            "-- No changes detected"
        ));
    }

    #[test]
    fn test_normalize_column_renames_filters_invalid_items() {
        let original = build_design(
            vec![build_col("a"), build_col("b"), build_col("c")],
            vec!["a"],
        );
        let current = build_design(vec![build_col("a"), build_col("c")], vec!["a"]);

        let normalized = TableDesigner::normalize_column_renames(
            &original,
            &current,
            &[
                ("b".to_string(), "a".to_string()),
                ("x".to_string(), "z".to_string()),
                ("b".to_string(), "a2".to_string()),
                ("c".to_string(), "".to_string()),
            ],
        );

        assert_eq!(normalized, vec![("b".to_string(), "a".to_string())]);
    }

    #[test]
    fn test_map_design_for_diff_rewrites_column_and_index_names() {
        let current = build_design(vec![build_col("a"), build_col("c")], vec!["a"]);
        let mapped =
            db::plugin::map_design_for_diff(&current, &[("b".to_string(), "a".to_string())]);

        let column_names: Vec<&str> = mapped.columns.iter().map(|col| col.name.as_str()).collect();
        assert_eq!(column_names, vec!["b", "c"]);
        assert_eq!(mapped.indexes[0].columns, vec!["b".to_string()]);
    }

    #[test]
    fn test_merge_alter_sql_behaviour() {
        let sql = db::plugin::merge_alter_sql(
            "-- No changes detected".to_string(),
            vec!["ALTER TABLE \"users\" RENAME COLUMN \"b\" TO \"a\";".to_string()],
        );
        assert_eq!(sql, "ALTER TABLE \"users\" RENAME COLUMN \"b\" TO \"a\";");

        let no_change = db::plugin::merge_alter_sql(String::new(), Vec::new());
        assert_eq!(no_change, "-- No changes detected");
    }

    #[test]
    fn test_loaded_column_type_missing_from_picker_is_preserved() {
        let mut data_types = vec!["INT".to_string(), "VARCHAR".to_string()];

        let idx = ColumnsEditor::ensure_loaded_type_option(&mut data_types, "jsonb");

        assert_eq!(idx, 2);
        assert_eq!(data_types, vec!["INT", "VARCHAR", "jsonb"]);
    }

    #[test]
    fn test_build_table_design_derives_primary_key_from_pk_index() {
        // IPC 驱动的 schema/columns 不回列级主键（is_primary 恒 false），
        // 设计器必须从 PK 索引成员反推，否则 PRIMARY KEY 整条丢失。
        let columns = vec![
            ColumnInfo {
                name: "id".to_string(),
                data_type: "NUMBER(10)".to_string(),
                is_nullable: false,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
                is_auto_increment: false,
            },
            ColumnInfo {
                name: "amount".to_string(),
                data_type: "DECIMAL(10,4)".to_string(),
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
                is_auto_increment: false,
            },
        ];
        let indexes = vec![IndexInfo {
            name: "PK_ORDERS".to_string(),
            columns: vec!["ID".to_string()],
            is_unique: true,
            is_primary: true,
            index_type: Some("NORMAL".to_string()),
        }];

        let design = build_table_design_from_metadata(
            DatabaseType::Oracle,
            "app".to_string(),
            "orders".to_string(),
            &columns,
            &indexes,
            None,
            None,
        );

        assert!(design.columns[0].is_primary_key, "id must be primary");
        assert!(
            !design.columns[1].is_primary_key,
            "amount must not be primary"
        );
        // 精度随 raw_type 走解析链，不能丢。
        assert_eq!(design.columns[0].length, Some(10));
        assert_eq!(design.columns[1].length, Some(10));
        assert_eq!(design.columns[1].scale, Some(4));
    }

    #[test]
    fn test_primary_indexes_with_database_specific_names_are_not_editable_indexes() {
        let design = build_table_design_from_metadata(
            DatabaseType::PostgreSQL,
            "app".to_string(),
            "users".to_string(),
            &[],
            &[
                IndexInfo {
                    name: "users_pkey".to_string(),
                    columns: vec!["id".to_string()],
                    is_unique: true,
                    is_primary: true,
                    index_type: Some("btree".to_string()),
                },
                IndexInfo {
                    name: "idx_users_email".to_string(),
                    columns: vec!["email".to_string()],
                    is_unique: true,
                    is_primary: false,
                    index_type: Some("btree".to_string()),
                },
            ],
            None,
            None,
        );

        assert_eq!(design.indexes.len(), 1);
        assert_eq!(design.indexes[0].name, "idx_users_email");
    }

    #[test]
    fn test_build_table_design_from_metadata_preserves_table_comment() {
        let table_info = TableInfo {
            name: "users".to_string(),
            object_type: db::TableObjectType::Table,
            schema: Some("public".to_string()),
            comment: Some("User table".to_string()),
            engine: None,

            create_time: None,
            charset: None,
            collation: None,
        };

        let design = build_table_design_from_metadata(
            DatabaseType::PostgreSQL,
            "app".to_string(),
            "users".to_string(),
            &[],
            &[],
            Some(&table_info),
            None,
        );

        assert_eq!("User table", design.options.comment);
    }

    /// issue #586：既有表的引擎/字符集/排序规则要一起进入原始设计。
    ///
    /// 只带表注释时，设计器的选项栏永远是默认值，保存时也无从判断表设置
    /// 是否被改动（MySQL 的 ALTER 在 original 侧为 None 时会直接跳过表选项）。
    #[test]
    fn test_build_table_design_from_metadata_preserves_table_options() {
        let table_info = TableInfo {
            name: "users".to_string(),
            object_type: db::TableObjectType::Table,
            schema: Some("app".to_string()),
            comment: Some("用户表".to_string()),
            engine: Some("InnoDB".to_string()),
            create_time: None,
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_0900_ai_ci".to_string()),
        };

        let design = build_table_design_from_metadata(
            DatabaseType::MySQL,
            "app".to_string(),
            "users".to_string(),
            &[],
            &[],
            Some(&table_info),
            None,
        );

        assert_eq!(Some("InnoDB"), design.options.engine.as_deref());
        assert_eq!(Some("utf8mb4"), design.options.charset.as_deref());
        assert_eq!(
            Some("utf8mb4_0900_ai_ci"),
            design.options.collation.as_deref()
        );
    }

    /// issue #586：新增列带上表字符集/排序规则时，ADD COLUMN 要显式写出
    /// CHARACTER SET/COLLATE；表选项本身没变，ALTER 不应附带表级设置。
    #[test]
    fn batch_column_specs_parse_type_and_length() {
        // issue #30 #7：批量加列时可以直接写「列名 类型(长度)」，逗号分隔也接受。
        let specs = parse_batch_column_specs(
            "email varchar(255), nick char(10)\ncreated_at DATETIME(6,2)\n  \nEMAIL text",
        );
        assert_eq!(3, specs.len());
        assert_eq!("email", specs[0].name);
        assert_eq!(Some("varchar"), specs[0].data_type.as_deref());
        assert_eq!(Some("255"), specs[0].type_args.as_deref());
        assert_eq!("nick", specs[1].name);
        assert_eq!(Some("char"), specs[1].data_type.as_deref());
        assert_eq!(Some("10"), specs[1].type_args.as_deref());
        assert_eq!("created_at", specs[2].name);
        assert_eq!(Some("6,2"), specs[2].type_args.as_deref());

        // 没有类型的条目只带列名。
        let specs = parse_batch_column_specs("plain_name");
        assert_eq!(None, specs[0].data_type);
        assert_eq!(None, specs[0].type_args);
    }

    #[test]
    fn batch_column_names_ignore_blank_lines_and_duplicate_case_insensitively() {
        // issue #30 #7：批量加列时一行一个列名，空白行与重复列名要跳过，
        // 否则会一次性加出重复列，提交到数据库时才报错。
        let names = parse_batch_column_names("  name \n\nemail\nName\n  \ncreated_at\n");
        assert_eq!(vec!["name", "email", "created_at"], names);
    }

    #[test]
    fn test_mysql_add_column_inherits_table_charset() {
        let plugin = build_plugin(&DatabaseType::MySQL);
        let mut original = build_design(vec![build_col("id")], vec!["id"]);
        original.options.engine = Some("InnoDB".to_string());
        original.options.charset = Some("utf8mb4".to_string());
        original.options.collation = Some("utf8mb4_0900_ai_ci".to_string());

        let mut added = build_col("nickname");
        added.data_type = "VARCHAR".to_string();
        added.length = Some(64);
        added.is_nullable = true;
        added.charset = Some("utf8mb4".to_string());
        added.collation = Some("utf8mb4_0900_ai_ci".to_string());
        let mut current = original.clone();
        current.columns.push(added);

        let sql = plugin.build_alter_table_sql(&original, &current);

        assert!(sql.contains("ADD COLUMN"), "{sql}");
        assert!(
            sql.contains("VARCHAR(64) CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_ai_ci"),
            "{sql}"
        );
        assert!(!sql.contains("DEFAULT CHARSET"), "{sql}");
        assert!(!sql.contains("COLLATE=utf8mb4_0900_ai_ci"), "{sql}");
    }

    #[test]
    fn test_mysql_rename_sql_uses_change_column() {
        let plugin = MySqlPlugin::new();
        let col = build_col("a");
        let sql = plugin.build_column_rename_sql("users", "b", "a", Some(&col));
        assert!(sql.contains("CHANGE COLUMN"));
    }

    #[test]
    fn test_postgresql_rename_sql_uses_rename_column() {
        let plugin = PostgresPlugin::new();
        let sql = plugin.build_column_rename_sql("users", "b", "a", None);
        assert!(sql.contains("RENAME COLUMN \"b\" TO \"a\""));
    }

    #[test]
    fn test_sqlite_rename_sql_uses_rename_column() {
        let plugin = SqlitePlugin::new();
        let sql = plugin.build_column_rename_sql("users", "b", "a", None);
        assert!(sql.contains("RENAME COLUMN \"b\" TO \"a\""));
    }

    #[test]
    fn test_mssql_rename_sql_uses_sp_rename_column() {
        let plugin = MsSqlPlugin::new();
        let sql = plugin.build_column_rename_sql("users", "b", "a", None);
        assert_eq!(sql, "EXEC sp_rename '[users].[b]', 'a', 'COLUMN';");
    }

    #[test]
    fn test_oracle_rename_sql_uses_rename_column() {
        let plugin = OraclePlugin::new();
        let sql = plugin.build_column_rename_sql("users", "b", "a", None);
        assert!(sql.contains("RENAME COLUMN \"b\" TO \"a\""));
    }

    #[test]
    fn test_mysql_change_column_keeps_new_definition() {
        let plugin = MySqlPlugin::new();
        let mut renamed_col = build_col("a");
        renamed_col.data_type = "BIGINT".to_string();
        renamed_col.is_nullable = true;

        let sql = plugin.build_column_rename_sql("users", "b", "a", Some(&renamed_col));

        assert!(sql.contains("`a` BIGINT"));
        assert!(
            !sql.contains("NOT NULL"),
            "nullable=true 时不应强制生成 NOT NULL: {}",
            sql
        );
    }

    #[test]
    fn test_build_alter_table_sql_with_renames_contains_rename_for_all_databases() {
        let (original, current, renames) = build_delete_and_rename_conflict_case();

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            assert_contains_rename_sql(&sql, database_type);
        }
    }

    #[test]
    fn test_build_alter_table_sql_with_renames_not_drop_source_for_all_databases() {
        let (original, current, renames) = build_delete_and_rename_conflict_case();

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            assert_not_drop_source_column(&sql, plugin.as_ref());
        }
    }

    #[test]
    fn test_build_alter_table_sql_with_empty_renames_keeps_base_alter_sql() {
        let original = build_design(vec![build_col("a")], vec![]);
        let current = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let plugin = PostgresPlugin::new();

        let base_sql = plugin.build_alter_table_sql(&original, &current);
        let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &[]);

        assert_eq!(sql.trim(), base_sql.trim());
    }

    /// 测试简单重命名场景：仅修改列名，不修改其他属性
    #[test]
    fn test_simple_rename_generates_rename_not_drop_add() {
        let original = build_design(vec![build_col("a"), build_col("b"), build_col("c")], vec![]);
        // 用户将 b 重命名为 b2，其他不变
        let current = build_design(
            vec![build_col("a"), build_col("b2"), build_col("c")],
            vec![],
        );
        let renames = vec![("b".to_string(), "b2".to_string())];

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            // 不应包含 DROP COLUMN
            let drop_b = format!("DROP COLUMN {}", plugin.quote_identifier("b"));
            assert!(
                !sql.contains(&drop_b),
                "[{:?}] 简单重命名不应生成 DROP COLUMN: {sql}",
                database_type
            );
            // 不应包含 ADD COLUMN b2
            let add_b2 = format!("ADD COLUMN {}", plugin.quote_identifier("b2"));
            // MySQL ADD COLUMN 可能不包含引号后的完整格式，用更宽松的匹配
            let add_keyword = "ADD COLUMN";
            if !matches!(database_type, DatabaseType::SQLite | DatabaseType::DuckDB) {
                assert!(
                    !sql.contains(&add_b2)
                        && (!sql.contains(add_keyword) || sql.contains("RENAME")),
                    "[{:?}] 简单重命名不应生成 ADD COLUMN: {sql}",
                    database_type
                );
            }
            // 应包含 RENAME 相关语句
            let has_rename = sql.contains("RENAME COLUMN")
                || sql.contains("CHANGE COLUMN")
                || sql.contains("sp_rename");
            assert!(has_rename, "[{:?}] 应包含重命名SQL: {sql}", database_type);
            println!("[{:?}] SQL: {}", database_type, sql);
        }
    }

    /// 测试模拟真实场景：original_design 的 data_type 与 collect_design 的 data_type 大小写不同
    #[test]
    fn test_rename_with_type_case_mismatch_no_drop_add() {
        // 模拟 build_original_design 返回小写 data_type（从DB解析得到）
        let original = build_design(
            vec![
                ColumnDefinition {
                    name: "id".to_string(),
                    data_type: "integer".to_string(),
                    is_nullable: false,
                    is_primary_key: true,
                    ..Default::default()
                },
                ColumnDefinition {
                    name: "name".to_string(),
                    data_type: "character varying".to_string(),
                    length: Some(50),
                    is_nullable: true,
                    ..Default::default()
                },
            ],
            vec![],
        );
        // 模拟 collect_design 返回大写 data_type（从下拉选择框获取）
        let current = build_design(
            vec![
                ColumnDefinition {
                    name: "id".to_string(),
                    data_type: "INTEGER".to_string(),
                    is_nullable: false,
                    is_primary_key: true,
                    ..Default::default()
                },
                ColumnDefinition {
                    name: "username".to_string(),
                    data_type: "VARCHAR".to_string(),
                    length: Some(50),
                    is_nullable: true,
                    ..Default::default()
                },
            ],
            vec![],
        );
        let renames = vec![("name".to_string(), "username".to_string())];

        let plugin = PostgresPlugin::new();
        let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);

        println!("Type case mismatch SQL: {}", sql);

        // 关键断言：不应包含 DROP COLUMN "name"
        assert!(
            !sql.contains("DROP COLUMN"),
            "类型大小写不同时不应生成 DROP COLUMN: {sql}"
        );
        // 不应包含 ADD COLUMN
        assert!(
            !sql.contains("ADD COLUMN"),
            "类型大小写不同时不应生成 ADD COLUMN: {sql}"
        );
        // 应包含 RENAME COLUMN
        assert!(
            sql.contains("RENAME COLUMN \"name\" TO \"username\""),
            "应包含正确的 RENAME SQL: {sql}"
        );
    }

    // ===== 以下为补充测试，覆盖各种表修改场景 =====

    /// 完全无变更时应返回 "-- No changes detected"
    #[test]
    fn test_no_changes_returns_no_changes_for_all_databases() {
        let design = build_design(vec![build_col("a"), build_col("b")], vec![]);

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&design, &design);
            assert_eq!(
                sql, "-- No changes detected",
                "[{:?}] 无变更时应返回 no changes",
                database_type
            );
        }
    }

    /// 仅新增列（无重命名）
    #[test]
    fn test_add_column_only_for_all_databases() {
        let original = build_design(vec![build_col("a")], vec![]);
        let current = build_design(vec![build_col("a"), build_col("b")], vec![]);

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            let quoted_b = plugin.quote_identifier("b");
            assert!(
                sql.contains(&quoted_b),
                "[{:?}] 新增列应包含列名 b: {sql}",
                database_type
            );
            assert!(
                !sql.contains("DROP COLUMN"),
                "[{:?}] 仅新增列不应包含 DROP COLUMN: {sql}",
                database_type
            );
        }
    }

    /// 仅删除列（无重命名）
    #[test]
    fn test_drop_column_only_for_all_databases() {
        let original = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let current = build_design(vec![build_col("a")], vec![]);

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            // SQLite 使用 table recreation 方式，不包含 DROP COLUMN 关键词
            if !matches!(database_type, DatabaseType::SQLite | DatabaseType::DuckDB) {
                assert!(
                    sql.contains("DROP COLUMN"),
                    "[{:?}] 删除列应包含 DROP COLUMN: {sql}",
                    database_type
                );
            }
            assert!(
                !sql.starts_with("-- No changes"),
                "[{:?}] 删除列不应返回 no changes: {sql}",
                database_type
            );
        }
    }

    /// 仅修改列类型（无重命名）
    #[test]
    fn test_modify_column_type_for_all_databases() {
        let original = build_design(vec![build_col("a")], vec![]);
        let mut modified_col = build_col("a");
        modified_col.data_type = "BIGINT".to_string();
        let current = build_design(vec![modified_col], vec![]);

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            assert!(
                sql.contains("BIGINT"),
                "[{:?}] 修改类型后应包含新类型 BIGINT: {sql}",
                database_type
            );
            assert!(
                !sql.starts_with("-- No changes"),
                "[{:?}] 修改类型不应返回 no changes: {sql}",
                database_type
            );
        }
    }

    /// 修改列 nullable 属性
    #[test]
    fn test_modify_column_nullable_for_all_databases() {
        let original = build_design(vec![build_col("a")], vec![]);
        let mut nullable_col = build_col("a");
        nullable_col.is_nullable = true;
        let current = build_design(vec![nullable_col], vec![]);

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            assert!(
                !sql.starts_with("-- No changes"),
                "[{:?}] 修改 nullable 不应返回 no changes: {sql}",
                database_type
            );
        }
    }

    /// 重命名 + 同时新增另一列
    #[test]
    fn test_rename_and_add_column_simultaneously() {
        let original = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let current = build_design(
            vec![build_col("a_new"), build_col("b"), build_col("c")],
            vec![],
        );
        let renames = vec![("a".to_string(), "a_new".to_string())];

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            // 应包含重命名
            let has_rename = sql.contains("RENAME COLUMN")
                || sql.contains("CHANGE COLUMN")
                || sql.contains("sp_rename");
            assert!(has_rename, "[{:?}] 应包含重命名 SQL: {sql}", database_type);
            // 应包含新增列 c
            let quoted_c = plugin.quote_identifier("c");
            assert!(
                sql.contains(&quoted_c),
                "[{:?}] 应包含新增列 c: {sql}",
                database_type
            );
            // 不应 DROP 被重命名的源列
            let drop_a = format!("DROP COLUMN {}", plugin.quote_identifier("a"));
            assert!(
                !sql.contains(&drop_a),
                "[{:?}] 不应 DROP 重命名源列 a: {sql}",
                database_type
            );
        }
    }

    /// 重命名 + 同时删除另一列
    #[test]
    fn test_rename_and_delete_another_column() {
        let original = build_design(vec![build_col("a"), build_col("b"), build_col("c")], vec![]);
        // 重命名 a→a2，删除 c
        let current = build_design(vec![build_col("a2"), build_col("b")], vec![]);
        let renames = vec![("a".to_string(), "a2".to_string())];

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            // 应包含重命名
            let has_rename = sql.contains("RENAME COLUMN")
                || sql.contains("CHANGE COLUMN")
                || sql.contains("sp_rename");
            assert!(has_rename, "[{:?}] 应包含重命名 SQL: {sql}", database_type);
            // 不应 DROP 重命名源列 a
            let drop_a = format!("DROP COLUMN {}", plugin.quote_identifier("a"));
            assert!(
                !sql.contains(&drop_a),
                "[{:?}] 不应 DROP 重命名源列 a: {sql}",
                database_type
            );
            // SQLite 使用 table recreation，不直接包含 DROP COLUMN
            if !matches!(database_type, DatabaseType::SQLite | DatabaseType::DuckDB) {
                // 应 DROP 被删除的列 c
                let drop_c = format!("DROP COLUMN {}", plugin.quote_identifier("c"));
                assert!(
                    sql.contains(&drop_c),
                    "[{:?}] 应 DROP 被删除的列 c: {sql}",
                    database_type
                );
            }
        }
    }

    /// 重命名 + 同时修改另一列的类型
    #[test]
    fn test_rename_and_modify_another_column() {
        let original = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let mut modified_b = build_col("b");
        modified_b.data_type = "BIGINT".to_string();
        let current = build_design(vec![build_col("a_new"), modified_b], vec![]);
        let renames = vec![("a".to_string(), "a_new".to_string())];

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            // 应包含重命名
            let has_rename = sql.contains("RENAME COLUMN")
                || sql.contains("CHANGE COLUMN")
                || sql.contains("sp_rename");
            assert!(has_rename, "[{:?}] 应包含重命名 SQL: {sql}", database_type);
            // 应包含修改列 b 的类型
            assert!(
                sql.contains("BIGINT"),
                "[{:?}] 应包含修改后的类型 BIGINT: {sql}",
                database_type
            );
        }
    }

    /// 同时重命名多列
    #[test]
    fn test_multiple_renames_simultaneously() {
        let original = build_design(vec![build_col("a"), build_col("b"), build_col("c")], vec![]);
        let current = build_design(vec![build_col("x"), build_col("y"), build_col("c")], vec![]);
        let renames = vec![
            ("a".to_string(), "x".to_string()),
            ("b".to_string(), "y".to_string()),
        ];

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            // 不应 DROP 源列
            let drop_a = format!("DROP COLUMN {}", plugin.quote_identifier("a"));
            let drop_b = format!("DROP COLUMN {}", plugin.quote_identifier("b"));
            assert!(
                !sql.contains(&drop_a),
                "[{:?}] 不应 DROP 重命名源列 a: {sql}",
                database_type
            );
            assert!(
                !sql.contains(&drop_b),
                "[{:?}] 不应 DROP 重命名源列 b: {sql}",
                database_type
            );
            // 不应 ADD 目标列
            assert!(
                !sql.contains("ADD COLUMN") || sql.contains("RENAME") || sql.contains("CHANGE"),
                "[{:?}] 不应 ADD COLUMN: {sql}",
                database_type
            );
            // 应包含两条重命名语句
            let rename_count = sql.match_indices("RENAME COLUMN").count()
                + sql.match_indices("CHANGE COLUMN").count()
                + sql.match_indices("sp_rename").count();
            assert_eq!(
                rename_count, 2,
                "[{:?}] 应包含 2 条重命名语句，实际 {} 条: {sql}",
                database_type, rename_count
            );
        }
    }

    /// 重命名的列同时也是索引列
    #[test]
    fn test_rename_column_in_index() {
        let original = build_design(
            vec![build_col("a"), build_col("b"), build_col("c")],
            vec!["b"],
        );
        // 重命名索引列 b→b2，索引也应指向 b2
        let mut current = build_design(
            vec![build_col("a"), build_col("b2"), build_col("c")],
            vec!["b2"],
        );
        current.indexes[0].name = "idx_test".to_string();
        let renames = vec![("b".to_string(), "b2".to_string())];

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            // 应包含重命名语句
            let has_rename = sql.contains("RENAME COLUMN")
                || sql.contains("CHANGE COLUMN")
                || sql.contains("sp_rename");
            assert!(has_rename, "[{:?}] 应包含重命名 SQL: {sql}", database_type);
            // 不应误删再重建索引（因为 map_design_for_diff 会将 b2 映射回 b）
            assert!(
                !sql.contains("DROP INDEX"),
                "[{:?}] 重命名索引列不应删除索引: {sql}",
                database_type
            );
        }
    }

    /// 新增索引（无列重命名）
    #[test]
    fn test_add_index_for_all_databases() {
        let original = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let current = build_design(vec![build_col("a"), build_col("b")], vec!["a"]);

        for database_type in design_test_database_types() {
            if matches!(database_type, DatabaseType::TDengine) {
                // TDengine 无二级索引概念(supports_indexes = false),不参与索引 DDL 断言
                continue;
            }
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            // 应包含 INDEX 相关关键字
            assert!(
                sql.to_uppercase().contains("INDEX"),
                "[{:?}] 新增索引应包含 INDEX 关键字: {sql}",
                database_type
            );
        }
    }

    /// 删除索引（无列重命名）
    #[test]
    fn test_drop_index_for_all_databases() {
        let original = build_design(vec![build_col("a"), build_col("b")], vec!["a"]);
        let current = build_design(vec![build_col("a"), build_col("b")], vec![]);

        for database_type in design_test_database_types() {
            if matches!(database_type, DatabaseType::TDengine) {
                // TDengine 无二级索引概念,不参与索引 DDL 断言
                continue;
            }
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            assert!(
                sql.to_uppercase().contains("DROP") && sql.to_uppercase().contains("INDEX"),
                "[{:?}] 删除索引应包含 DROP INDEX: {sql}",
                database_type
            );
        }
    }

    /// map_design_for_diff 支持多列重命名
    #[test]
    fn test_map_design_for_diff_multiple_renames() {
        let current = build_design(
            vec![build_col("x"), build_col("y"), build_col("z")],
            vec!["x", "y"],
        );
        let renames = vec![
            ("a".to_string(), "x".to_string()),
            ("b".to_string(), "y".to_string()),
        ];
        let mapped = db::plugin::map_design_for_diff(&current, &renames);
        let names: Vec<&str> = mapped.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b", "z"]);
        // 索引列也应被回退
        assert_eq!(
            mapped.indexes[0].columns,
            vec!["a".to_string(), "b".to_string()]
        );
    }

    /// map_design_for_diff 当列名不匹配时不做任何修改
    #[test]
    fn test_map_design_for_diff_no_matching_column() {
        let current = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let renames = vec![("x".to_string(), "y".to_string())];
        let mapped = db::plugin::map_design_for_diff(&current, &renames);
        let names: Vec<&str> = mapped.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    /// map_design_for_diff 传空重命名列表时返回原样
    #[test]
    fn test_map_design_for_diff_empty_renames() {
        let current = build_design(vec![build_col("a"), build_col("b")], vec!["a"]);
        let mapped = db::plugin::map_design_for_diff(&current, &[]);
        let names: Vec<&str> = mapped.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        assert_eq!(mapped.indexes[0].columns, vec!["a".to_string()]);
    }

    /// merge_alter_sql 同时有 base SQL 和 rename 语句时两者都保留
    #[test]
    fn test_merge_alter_sql_combines_base_and_renames() {
        let base = "ALTER TABLE \"users\" ADD COLUMN \"c\" INT NOT NULL;".to_string();
        let renames = vec![
            "ALTER TABLE \"users\" RENAME COLUMN \"a\" TO \"x\";".to_string(),
            "ALTER TABLE \"users\" RENAME COLUMN \"b\" TO \"y\";".to_string(),
        ];
        let result = db::plugin::merge_alter_sql(base.clone(), renames);
        assert!(result.contains(&base), "应包含 base SQL");
        assert!(
            result.contains("RENAME COLUMN \"a\" TO \"x\""),
            "应包含第一条 rename"
        );
        assert!(
            result.contains("RENAME COLUMN \"b\" TO \"y\""),
            "应包含第二条 rename"
        );
        // 各语句用换行分隔
        assert_eq!(result.lines().count(), 3, "应有 3 行 SQL 语句");
    }

    /// merge_alter_sql base 为空字符串时只保留 rename 语句
    #[test]
    fn test_merge_alter_sql_empty_base_with_renames() {
        let renames = vec!["ALTER TABLE t RENAME COLUMN a TO b;".to_string()];
        let result = db::plugin::merge_alter_sql(String::new(), renames);
        assert_eq!(result, "ALTER TABLE t RENAME COLUMN a TO b;");
    }

    /// merge_alter_sql base 为纯空白时跳过 base
    #[test]
    fn test_merge_alter_sql_whitespace_base() {
        let renames = vec!["RENAME SQL;".to_string()];
        let result = db::plugin::merge_alter_sql("   \n  ".to_string(), renames);
        assert_eq!(result, "RENAME SQL;");
    }

    /// normalize_column_renames 过滤自身重命名（old == new）
    #[test]
    fn test_normalize_column_renames_filters_self_renames() {
        let original = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let current = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let normalized = TableDesigner::normalize_column_renames(
            &original,
            &current,
            &[("a".to_string(), "a".to_string())],
        );
        assert!(normalized.is_empty(), "old == new 的重命名应被过滤");
    }

    /// normalize_column_renames 去重：同一旧列名只保留第一次出现
    #[test]
    fn test_normalize_column_renames_dedup_old_name() {
        let original = build_design(vec![build_col("a"), build_col("b"), build_col("c")], vec![]);
        let current = build_design(vec![build_col("x"), build_col("y"), build_col("c")], vec![]);
        let normalized = TableDesigner::normalize_column_renames(
            &original,
            &current,
            &[
                ("a".to_string(), "x".to_string()),
                ("a".to_string(), "y".to_string()), // 重复旧列名
            ],
        );
        assert_eq!(
            normalized,
            vec![("a".to_string(), "x".to_string())],
            "同一旧列名只保留第一次"
        );
    }

    /// normalize_column_renames 去重：同一新列名只保留第一次出现
    #[test]
    fn test_normalize_column_renames_dedup_new_name() {
        let original = build_design(vec![build_col("a"), build_col("b"), build_col("c")], vec![]);
        let current = build_design(vec![build_col("x"), build_col("b"), build_col("c")], vec![]);
        let normalized = TableDesigner::normalize_column_renames(
            &original,
            &current,
            &[
                ("a".to_string(), "x".to_string()),
                ("b".to_string(), "x".to_string()), // 重复新列名
            ],
        );
        assert_eq!(
            normalized,
            vec![("a".to_string(), "x".to_string())],
            "同一新列名只保留第一次"
        );
    }

    /// MySQL build_column_rename_sql 无列定义时回退为 RENAME COLUMN
    #[test]
    fn test_mysql_rename_sql_fallback_without_column_def() {
        let plugin = MySqlPlugin::new();
        let sql = plugin.build_column_rename_sql("users", "old_col", "new_col", None);
        assert!(
            sql.contains("RENAME COLUMN"),
            "MySQL 无列定义时应回退为 RENAME COLUMN: {sql}"
        );
        assert!(
            !sql.contains("CHANGE COLUMN"),
            "MySQL 无列定义时不应使用 CHANGE COLUMN: {sql}"
        );
    }

    /// MySQL CHANGE COLUMN 保留 default 值和 comment
    #[test]
    fn test_mysql_change_column_preserves_default_and_comment() {
        let plugin = MySqlPlugin::new();
        let col = ColumnDefinition {
            name: "new_name".to_string(),
            data_type: "VARCHAR".to_string(),
            length: Some(100),
            is_nullable: true,
            default_value: Some("'hello'".to_string()),
            comment: "用户名字段".to_string(),
            ..Default::default()
        };
        let sql = plugin.build_column_rename_sql("users", "old_name", "new_name", Some(&col));
        assert!(sql.contains("CHANGE COLUMN"), "应使用 CHANGE COLUMN: {sql}");
        assert!(sql.contains("DEFAULT 'hello'"), "应保留 DEFAULT 值: {sql}");
        assert!(
            sql.contains("COMMENT '用户名字段'"),
            "应保留 COMMENT: {sql}"
        );
    }

    /// MySQL CHANGE COLUMN 保留 auto_increment
    #[test]
    fn test_mysql_change_column_preserves_auto_increment() {
        let plugin = MySqlPlugin::new();
        let col = ColumnDefinition {
            name: "id".to_string(),
            data_type: "INT".to_string(),
            is_nullable: false,
            is_auto_increment: true,
            ..Default::default()
        };
        let sql = plugin.build_column_rename_sql("users", "old_id", "id", Some(&col));
        assert!(
            sql.contains("AUTO_INCREMENT"),
            "应保留 AUTO_INCREMENT: {sql}"
        );
    }

    /// MSSQL sp_rename 正确处理包含单引号的列名
    #[test]
    fn test_mssql_rename_escapes_single_quotes() {
        let plugin = MsSqlPlugin::new();
        let sql = plugin.build_column_rename_sql("users", "col'a", "col'b", None);
        // 单引号应被转义
        assert!(sql.contains("col''a"), "旧列名中的单引号应被转义: {sql}");
        assert!(sql.contains("col''b"), "新列名中的单引号应被转义: {sql}");
    }

    /// build_alter_table_sql_with_renames 传入空设计（无列、无索引）
    #[test]
    fn test_alter_table_with_renames_empty_designs() {
        let empty = build_design(vec![], vec![]);
        let plugin = PostgresPlugin::new();
        let sql = plugin.build_alter_table_sql_with_renames(&empty, &empty, &[]);
        assert_eq!(sql, "-- No changes detected");
    }

    /// 重命名 + 修改同一列的类型（重命名列同时改了 data_type）
    #[test]
    fn test_rename_and_modify_same_column() {
        let original = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let mut renamed_and_modified = build_col("a_new");
        renamed_and_modified.data_type = "BIGINT".to_string();
        let current = build_design(vec![renamed_and_modified, build_col("b")], vec![]);
        let renames = vec![("a".to_string(), "a_new".to_string())];

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql_with_renames(&original, &current, &renames);
            // 应包含重命名
            let has_rename = sql.contains("RENAME COLUMN")
                || sql.contains("CHANGE COLUMN")
                || sql.contains("sp_rename");
            assert!(has_rename, "[{:?}] 应包含重命名 SQL: {sql}", database_type);
            // 对于 MySQL 的 CHANGE COLUMN，类型变更和重命名合为一条
            // 对于其他数据库，类型变更和重命名分别生成
            if database_type == DatabaseType::MySQL {
                // MySQL CHANGE COLUMN 自带完整列定义，包含新类型
                assert!(
                    sql.contains("BIGINT"),
                    "[MySQL] CHANGE COLUMN 应包含新类型 BIGINT: {sql}"
                );
            } else {
                // 其他数据库应同时包含 RENAME 和 ALTER COLUMN（类型变更）
                assert!(
                    sql.contains("BIGINT"),
                    "[{:?}] 应包含类型变更 BIGINT: {sql}",
                    database_type
                );
            }
        }
    }

    /// 所有列都被删除后应生成合法 SQL
    #[test]
    fn test_delete_all_columns() {
        let original = build_design(vec![build_col("a"), build_col("b")], vec![]);
        let current = build_design(vec![], vec![]);

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            // 应生成非空 SQL（而非 no changes）
            assert!(
                !sql.starts_with("-- No changes"),
                "[{:?}] 删除所有列不应返回 no changes: {sql}",
                database_type
            );
        }
    }

    /// 从零列新增多列
    #[test]
    fn test_add_multiple_columns_from_empty() {
        let original = build_design(vec![], vec![]);
        let current = build_design(vec![build_col("a"), build_col("b"), build_col("c")], vec![]);

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            let quoted_a = plugin.quote_identifier("a");
            let quoted_b = plugin.quote_identifier("b");
            let quoted_c = plugin.quote_identifier("c");
            assert!(
                sql.contains(&quoted_a) && sql.contains(&quoted_b) && sql.contains(&quoted_c),
                "[{:?}] 应包含所有新增列 a, b, c: {sql}",
                database_type
            );
        }
    }

    /// 同时新增和删除索引
    #[test]
    fn test_add_and_drop_index_simultaneously() {
        let mut original = build_design(
            vec![build_col("a"), build_col("b"), build_col("c")],
            vec!["a"],
        );
        original.indexes[0].name = "idx_old".to_string();

        let mut current = build_design(
            vec![build_col("a"), build_col("b"), build_col("c")],
            vec!["b"],
        );
        current.indexes[0].name = "idx_new".to_string();

        for database_type in design_test_database_types() {
            if matches!(database_type, DatabaseType::TDengine) {
                // TDengine 无二级索引概念,不参与索引 DDL 断言
                continue;
            }
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            let upper = sql.to_uppercase();
            assert!(
                upper.contains("DROP") && upper.contains("INDEX"),
                "[{:?}] 应包含 DROP INDEX: {sql}",
                database_type
            );
            // 应包含创建新索引（INDEX 关键字出现在创建和删除两处）
            let idx_count = upper.match_indices("INDEX").count();
            assert!(
                idx_count >= 2,
                "[{:?}] INDEX 关键字应至少出现 2 次（增+删）: {sql}",
                database_type
            );
        }
    }

    /// 添加带完整属性的列（default、comment、nullable）
    #[test]
    fn test_add_column_with_full_attributes() {
        let original = build_design(vec![build_col("id")], vec![]);
        let new_col = ColumnDefinition {
            name: "email".to_string(),
            data_type: "VARCHAR".to_string(),
            length: Some(255),
            is_nullable: true,
            default_value: Some("''".to_string()),
            comment: "邮箱地址".to_string(),
            ..Default::default()
        };
        let current = build_design(vec![build_col("id"), new_col], vec![]);

        // MySQL 支持 COMMENT 和 DEFAULT
        let plugin = MySqlPlugin::new();
        let sql = plugin.build_alter_table_sql(&original, &current);
        assert!(sql.contains("VARCHAR(255)"), "应包含类型和长度: {sql}");
        assert!(sql.contains("DEFAULT ''"), "应包含 DEFAULT: {sql}");
        assert!(sql.contains("COMMENT '邮箱地址'"), "应包含 COMMENT: {sql}");
    }

    /// 修改列 default 值
    #[test]
    fn test_modify_column_default_value() {
        let mut col_orig = build_col("a");
        col_orig.default_value = Some("0".to_string());
        let original = build_design(vec![col_orig], vec![]);

        let mut col_new = build_col("a");
        col_new.default_value = Some("1".to_string());
        let current = build_design(vec![col_new], vec![]);

        for database_type in design_test_database_types() {
            let plugin = build_plugin(&database_type);
            let sql = plugin.build_alter_table_sql(&original, &current);
            assert!(
                !sql.starts_with("-- No changes"),
                "[{:?}] 修改 DEFAULT 值不应返回 no changes: {sql}",
                database_type
            );
        }
    }

    #[test]
    fn test_column_info_to_definition_preserves_text_metadata() {
        let column = ColumnInfo {
            name: "session_id".to_string(),
            data_type: "varchar(255)".to_string(),
            is_nullable: false,
            is_primary_key: false,
            default_value: None,
            comment: Some("会话ID".to_string()),
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_general_ci".to_string()),
            is_auto_increment: false,
        };
        let parsed = MySqlPlugin::new().parse_column_type(&column.data_type);

        let definition = column_info_to_definition(DatabaseType::MySQL, &column, parsed, 0);

        assert_eq!(definition.data_type, "varchar");
        assert_eq!(definition.length, Some(255));
        assert_eq!(definition.comment, "会话ID");
        assert_eq!(definition.charset.as_deref(), Some("utf8mb4"));
        assert_eq!(definition.collation.as_deref(), Some("utf8mb4_general_ci"));
    }

    #[test]
    fn test_column_info_to_definition_preserves_unsigned_and_enum_values() {
        let numeric = ColumnInfo {
            name: "amount".to_string(),
            data_type: "int(11) unsigned".to_string(),
            is_nullable: false,
            is_primary_key: false,
            default_value: Some("0".to_string()),
            comment: None,
            charset: None,
            collation: None,
            is_auto_increment: false,
        };
        let enum_col = ColumnInfo {
            name: "status".to_string(),
            data_type: "enum('todo','done')".to_string(),
            is_nullable: false,
            is_primary_key: false,
            default_value: Some("'todo'".to_string()),
            comment: None,
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_bin".to_string()),
            is_auto_increment: false,
        };

        let numeric_definition = column_info_to_definition(
            DatabaseType::MySQL,
            &numeric,
            MySqlPlugin::new().parse_column_type(&numeric.data_type),
            0,
        );
        let enum_definition = column_info_to_definition(
            DatabaseType::MySQL,
            &enum_col,
            MySqlPlugin::new().parse_column_type(&enum_col.data_type),
            0,
        );

        assert!(numeric_definition.is_unsigned);
        assert_eq!(numeric_definition.length, Some(11));
        assert_eq!(enum_definition.data_type, "enum('todo','done')");
        assert_eq!(enum_definition.collation.as_deref(), Some("utf8mb4_bin"));
    }

    #[test]
    fn test_column_info_to_definition_prefers_metadata_auto_increment() {
        // MSSQL 的 IDENTITY 在类型字符串里没有任何痕迹，只能来自元数据；
        // 以前这里只认类型字符串解析，设计器就会丢掉 IDENTITY。
        let identity = ColumnInfo {
            name: "id".to_string(),
            data_type: "bigint".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
            is_auto_increment: true,
        };
        let parsed = MsSqlPlugin::new().parse_column_type(&identity.data_type);
        assert!(
            !parsed.is_auto_increment,
            "类型字符串解析不出 IDENTITY，这正是必须读元数据的原因"
        );

        let definition = column_info_to_definition(DatabaseType::MSSQL, &identity, parsed, 1);

        assert!(
            definition.is_auto_increment,
            "元数据的自增标记必须传导到设计"
        );
    }

    #[test]
    fn test_column_info_to_definition_sqlite_rowid_alias_fallback() {
        // 旧来源没填元数据时，SQLite 仍按「单列 INTEGER 主键 = rowid 别名」兜底。
        let column = ColumnInfo {
            name: "id".to_string(),
            data_type: "INTEGER".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
            is_auto_increment: false,
        };
        let parsed = SqlitePlugin::new().parse_column_type(&column.data_type);

        let single_pk = column_info_to_definition(DatabaseType::SQLite, &column, parsed.clone(), 1);
        let composite_pk = column_info_to_definition(DatabaseType::SQLite, &column, parsed, 2);

        assert!(
            single_pk.is_auto_increment,
            "单列 INTEGER 主键是 rowid 别名"
        );
        assert!(!composite_pk.is_auto_increment, "联合主键不构成 rowid 别名");
    }

    #[test]
    fn test_build_existing_collation_selection_preserves_utf8mb3_collation() {
        let plugin = MySqlPlugin::new();

        let (items, selected_idx) = ColumnsEditor::build_existing_collation_selection(
            Some(&plugin),
            "utf8mb3",
            Some("utf8mb3_general_ci"),
        );

        assert!(
            items
                .iter()
                .any(|item| item.info.name == "utf8mb3_general_ci")
        );
        assert_eq!(items[selected_idx].info.name, "utf8mb3_general_ci");
    }

    #[test]
    fn test_build_existing_collation_selection_preserves_unknown_collation() {
        let plugin = MySqlPlugin::new();

        let (items, selected_idx) = ColumnsEditor::build_existing_collation_selection(
            Some(&plugin),
            "utf8mb4",
            Some("utf8mb4_custom_ci"),
        );

        assert_eq!(items[selected_idx].info.name, "utf8mb4_custom_ci");
        assert_eq!(items[selected_idx].info.charset, "utf8mb4");
    }

    fn table_info(name: &str, schema: Option<&str>, comment: Option<&str>) -> TableInfo {
        TableInfo {
            name: name.into(),
            object_type: Default::default(),
            schema: schema.map(Into::into),
            comment: comment.map(Into::into),
            engine: None,
            create_time: None,
            charset: None,
            collation: None,
        }
    }

    #[test]
    fn sqlite_composite_pk_first_column_is_not_auto_increment() {
        // 复现 issue #301：WITHOUT ROWID 联合主键表的元数据。
        let columns = vec![
            ColumnInfo {
                name: "device_id".to_string(),
                data_type: "INTEGER".to_string(),
                is_nullable: false,
                is_primary_key: true,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
                is_auto_increment: false,
            },
            ColumnInfo {
                name: "tag_id".to_string(),
                data_type: "INTEGER".to_string(),
                is_nullable: false,
                is_primary_key: true,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
                is_auto_increment: false,
            },
            ColumnInfo {
                name: "ts".to_string(),
                data_type: "INTEGER".to_string(),
                is_nullable: false,
                is_primary_key: true,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
                is_auto_increment: false,
            },
            ColumnInfo {
                name: "value".to_string(),
                data_type: "REAL".to_string(),
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
                is_auto_increment: false,
            },
        ];

        let design = build_table_design_from_metadata(
            DatabaseType::SQLite,
            "main".to_string(),
            "datapoints".to_string(),
            &columns,
            &[],
            None,
            Some(&SqlitePlugin::new()),
        );

        let device_id = design
            .columns
            .iter()
            .find(|col| col.name == "device_id")
            .expect("device_id column should exist");
        assert!(
            !device_id.is_auto_increment,
            "联合主键首列不应被当作自增，否则 DDL 渲染成 INTEGER PRIMARY KEY AUTOINCREMENT"
        );
        assert!(device_id.is_primary_key);
    }

    #[test]
    fn sqlite_single_integer_pk_still_auto_increment() {
        let columns = vec![
            ColumnInfo {
                name: "id".to_string(),
                data_type: "INTEGER".to_string(),
                is_nullable: true,
                is_primary_key: true,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
                is_auto_increment: false,
            },
            ColumnInfo {
                name: "name".to_string(),
                data_type: "TEXT".to_string(),
                is_nullable: true,
                is_primary_key: false,
                default_value: None,
                comment: None,
                charset: None,
                collation: None,
                is_auto_increment: false,
            },
        ];

        let design = build_table_design_from_metadata(
            DatabaseType::SQLite,
            "main".to_string(),
            "users".to_string(),
            &columns,
            &[],
            None,
            Some(&SqlitePlugin::new()),
        );

        let id = design
            .columns
            .iter()
            .find(|col| col.name == "id")
            .expect("id column should exist");
        assert!(
            id.is_auto_increment,
            "单列 INTEGER 主键仍是 rowid 别名，应保留自增语义"
        );
    }

    #[test]
    fn find_loaded_table_info_matches_schema_when_available() {
        let found = find_loaded_table_info(
            vec![
                table_info("other", Some("testuser"), None),
                table_info("demo_child", Some("testuser"), Some("demo表")),
            ],
            "demo_child",
            Some("testuser"),
        );
        assert_eq!(found.and_then(|t| t.comment).as_deref(), Some("demo表"));
    }

    #[test]
    fn find_loaded_table_info_rejects_schema_mismatch() {
        let found = find_loaded_table_info(
            vec![table_info("demo_child", Some("other_owner"), None)],
            "demo_child",
            Some("testuser"),
        );
        assert!(found.is_none());
    }

    #[test]
    fn find_loaded_table_info_falls_back_to_name_when_schema_absent() {
        // 部分驱动(如 go dbipc)不在 schema/objects 返回 owner/schema,
        // wire 对标 None;此时应退化为按表名匹配,保证表注释能回显。
        let found = find_loaded_table_info(
            vec![table_info("demo_child", None, Some("demo表"))],
            "demo_child",
            Some("testuser"),
        );
        assert_eq!(found.and_then(|t| t.comment).as_deref(), Some("demo表"));
    }

    #[test]
    fn find_loaded_table_info_matches_by_name_when_no_schema_requested() {
        let found = find_loaded_table_info(
            vec![
                table_info("demo_child", Some("a"), Some("demo表")),
                table_info("demo_child", Some("b"), None),
            ],
            "demo_child",
            None,
        );
        assert_eq!(found.and_then(|t| t.comment).as_deref(), Some("demo表"));
    }

    #[test]
    fn initial_open_arms_the_structure_loading_panel() {
        let mut state = StructureLoadState::default();

        state.begin(StructureLoadReason::InitialOpen);

        assert!(state.is_loading());
    }

    #[test]
    fn after_save_reload_keeps_the_panel_in_place() {
        let mut state = StructureLoadState::default();

        state.begin(StructureLoadReason::AfterSave);

        assert!(!state.is_loading());
        assert!(state.is_refreshing());
    }

    #[test]
    fn a_new_designer_starts_without_the_loading_panel() {
        let state = StructureLoadState::default();

        assert!(!state.is_loading());
    }

    #[test]
    fn finishing_a_load_always_clears_the_loading_panel() {
        let mut state = StructureLoadState::default();
        state.begin(StructureLoadReason::InitialOpen);

        state.finish();

        assert!(!state.is_loading());
    }

    #[test]
    fn manual_refresh_uses_the_full_structure_loading_panel() {
        let mut state = StructureLoadState::default();

        state.begin(StructureLoadReason::ManualRefresh);

        assert!(state.is_loading());
        assert!(state.is_refreshing());

        state.finish();

        assert!(!state.is_loading());
        assert!(!state.is_refreshing());
    }

    #[test]
    fn columns_and_indexes_toolbar_refresh_buttons_reach_the_parent_loader() {
        let source = include_str!("table_designer_tab.rs");
        let columns_header = source
            .split("fn render_header(")
            .nth(1)
            .unwrap()
            .split("fn render_table_header(")
            .next()
            .unwrap();
        let indexes_header = source
            .split("impl Render for IndexesEditor")
            .nth(1)
            .unwrap()
            .split("impl EventEmitter<IndexesEditorEvent>")
            .next()
            .unwrap();

        assert!(columns_header.contains("IconButton::new(\"refresh-columns\", IconName::Refresh)"));
        assert!(columns_header.contains("cx.emit(ColumnsEditorEvent::RefreshRequested)"));
        assert!(indexes_header.contains("IconButton::new(\"refresh-indexes\", IconName::Refresh)"));
        assert!(indexes_header.contains("cx.emit(IndexesEditorEvent::RefreshRequested)"));

        let subscriptions = source
            .split("let cols_sub = cx.subscribe_in(")
            .nth(1)
            .unwrap()
            .split("let mut designer = Self {")
            .next()
            .unwrap();
        assert!(subscriptions.contains("ColumnsEditorEvent::RefreshRequested"));
        assert!(subscriptions.contains("this.refresh_structure(window, cx)"));
        assert!(subscriptions.contains("IndexesEditorEvent::RefreshRequested"));

        let loader = source
            .split("fn refresh_structure(")
            .nth(1)
            .unwrap()
            .split("fn load_table_structure(")
            .next()
            .unwrap();
        assert!(loader.contains("StructureLoadReason::ManualRefresh"));
    }

    #[test]
    fn refresh_structure_messages_are_translated() {
        let title = t!("Table.refresh_structure").to_string();
        let prompt = t!("Table.refresh_discard_changes_prompt").to_string();

        assert!(!title.contains("Table.refresh_structure"), "{title}");
        assert!(
            !prompt.contains("Table.refresh_discard_changes_prompt"),
            "{prompt}"
        );
    }

    /// 打开既有表时真实面板会先停在加载态（列/索引/表信息回来前不渲染空表单）。
    #[gpui::test]
    fn opening_an_existing_table_arms_the_loading_panel(cx: &mut gpui::TestAppContext) {
        let (_designer, armed_at_open, _visual) = designer_window(cx, Some("users"));

        assert!(armed_at_open);
    }

    /// 新建表没有可拉取的结构，不应该出现加载态。
    #[gpui::test]
    fn opening_a_new_table_keeps_the_form_visible(cx: &mut gpui::TestAppContext) {
        let (_designer, armed_at_open, _visual) = designer_window(cx, None);

        assert!(!armed_at_open);
    }

    /// issue #586：回填表设置后，选项栏显示表自己的字符集/排序规则，
    /// 之后新增的列默认继承它们，而不是停在「- 默认」/「(default)」。
    #[gpui::test]
    fn loaded_table_options_reach_the_options_bar_and_new_columns(cx: &mut gpui::TestAppContext) {
        let (designer, _armed_at_open, visual) = designer_window(cx, None);
        let options = TableOptions {
            engine: Some("InnoDB".to_string()),
            charset: Some("utf8mb4".to_string()),
            collation: Some("utf8mb4_0900_ai_ci".to_string()),
            ..TableOptions::default()
        };

        visual.update(|window, cx| {
            let columns_editor = designer.read(cx).columns_editor.clone();
            designer.update(cx, |designer, cx| {
                designer.apply_loaded_table_options(&options, window, cx);
            });
            columns_editor.update(cx, |editor, cx| editor.add_column(window, cx));
        });

        visual.read(|cx| {
            let designer = designer.read(cx);
            assert_eq!(
                Some("utf8mb4".to_string()),
                designer.charset_select.read(cx).selected_value().cloned()
            );
            assert_eq!(
                Some("utf8mb4_0900_ai_ci".to_string()),
                designer.collation_select.read(cx).selected_value().cloned()
            );

            let column = designer
                .columns_editor
                .read(cx)
                .get_columns(cx)
                .pop()
                .expect("新增列应当存在");
            assert_eq!(Some("utf8mb4".to_string()), column.charset);
            assert_eq!(Some("utf8mb4_0900_ai_ci".to_string()), column.collation);
        });

        // 只回传排序规则时，从排序规则名反推字符集，不能落回默认值。
        let collation_only = TableOptions {
            charset: None,
            collation: Some("utf8mb4_unicode_ci".to_string()),
            ..TableOptions::default()
        };
        visual.update(|window, cx| {
            designer.update(cx, |designer, cx| {
                designer.apply_loaded_table_options(&collation_only, window, cx);
            });
        });
        visual.read(|cx| {
            let designer = designer.read(cx);
            assert_eq!(
                Some("utf8mb4".to_string()),
                designer.charset_select.read(cx).selected_value().cloned()
            );
            assert_eq!(
                Some("utf8mb4_unicode_ci".to_string()),
                designer.collation_select.read(cx).selected_value().cloned()
            );
        });
    }

    /// 加载期间面板是加载中占位，而不是一片空白表单；加载结束后回到表单。
    ///
    /// 直接驱动加载态（不触发真实查询）：这里要验证的是渲染门控，
    /// 触发时机由 `opening_an_existing_table_arms_the_loading_panel` 负责。
    #[gpui::test]
    fn structure_loading_replaces_the_panel_for_initial_open_and_manual_refresh(
        cx: &mut gpui::TestAppContext,
    ) {
        let (designer, _armed_at_open, visual) = designer_window(cx, None);

        visual.update(|_window, cx| {
            designer.update(cx, |designer, cx| {
                designer
                    .structure_load
                    .begin(StructureLoadReason::InitialOpen);
                cx.notify();
            });
        });
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(visual.debug_bounds(STRUCTURE_LOADING_SELECTOR).is_some());

        visual.update(|_window, cx| {
            designer.update(cx, |designer, cx| designer.finish_structure_load(cx));
        });
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(visual.debug_bounds(STRUCTURE_LOADING_SELECTOR).is_none());

        visual.update(|_window, cx| {
            designer.update(cx, |designer, cx| {
                designer
                    .structure_load
                    .begin(StructureLoadReason::ManualRefresh);
                cx.notify();
            });
        });
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(visual.debug_bounds(STRUCTURE_LOADING_SELECTOR).is_some());

        visual.update(|_window, cx| {
            designer.update(cx, |designer, cx| designer.finish_structure_load(cx));
        });
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });
        assert!(visual.debug_bounds(STRUCTURE_LOADING_SELECTOR).is_none());
    }

    /// 结构加载失败的提示必须能在当前 locale 解析出内容。
    ///
    /// 这条文案是加载失败时唯一的用户可见反馈（空白网格、没有提示正是本次
    /// 问题的现场），key 打错会让用户直接看到原始 key。
    #[test]
    fn load_structure_failed_message_is_translated() {
        let message = t!("Table.load_structure_failed", error = "boom").to_string();

        assert!(message.contains("boom"), "{message}");
        assert!(
            !message.contains("Table.load_structure_failed"),
            "{message}"
        );
    }

    /// 表设计器第 0/1 列之间那条列边界上的抓取区。
    const DESIGNER_HANDLE: &str = "column-editor-resize-0";
    /// 拖动距离。留够余量，别和 2px 的拖动起步阈值贴太近。
    const DESIGNER_DRAG_DISTANCE: f32 = 60.;
    /// 拖动后列宽至少该增加这么多（指针相对边界有偏移，所以不写等号）。
    const DESIGNER_WIDENED_AT_LEAST: f32 = 40.;

    /// 第 `col_ix` 列的当前宽度。
    fn designer_column_width(
        designer: &Entity<TableDesigner>,
        visual: &mut gpui::VisualTestContext,
        col_ix: usize,
    ) -> Pixels {
        visual.read(|cx| {
            designer
                .read(cx)
                .columns_editor
                .read(cx)
                .column_width(col_ix)
        })
    }

    /// issue #333：表设计器的列宽抓取区必须骑在列边界上，而不是整条缩在左列右缘里。
    ///
    /// 表头单元格之间隔着 `gap_3`，抓取区是往右多铺半格、落进那段空隙里的，所以宽度
    /// 是边界两侧各半格合起来的一整条。
    #[gpui::test]
    fn the_column_editor_grab_straddles_the_boundary(cx: &mut gpui::TestAppContext) {
        let (_designer, _armed, visual) = designer_window(cx, None);
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let grab = visual
            .debug_bounds(DESIGNER_HANDLE)
            .expect("第 0 列右缘的抓取区应当被渲染出来");

        assert_eq!(
            COLUMN_EDITOR_RESIZE_GRAB_PADDING * 2.,
            grab.size.width,
            "抓取区应当以列边界为中心、两侧各铺半格插进空隙里"
        );
    }

    /// 从 `start` 按下、向右拖 [`DESIGNER_DRAG_DISTANCE`]，返回属性列拖后的宽度。
    fn drag_column_editor(
        designer: &Entity<TableDesigner>,
        visual: &mut gpui::VisualTestContext,
        start: gpui::Point<Pixels>,
    ) -> Pixels {
        let end = gpui::point(start.x + px(DESIGNER_DRAG_DISTANCE), start.y);
        visual.simulate_mouse_down(start, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.simulate_mouse_move(end, gpui::MouseButton::Left, gpui::Modifiers::default());
        // 多走一格再松手，确保拖动本身真正起步（起步阈值 2px）。
        visual.simulate_mouse_move(
            gpui::point(end.x + px(1.), end.y),
            gpui::MouseButton::Left,
            gpui::Modifiers::default(),
        );
        visual.simulate_mouse_up(end, gpui::MouseButton::Left, gpui::Modifiers::default());
        visual.run_until_parked();

        designer_column_width(designer, visual, COLUMN_NAME_COL)
    }

    /// issue #333：在列边界**右侧**（空隙那一侧）按下也要能拖动列宽。
    ///
    /// 分隔线画在抓取区正中，往右 3px 正是旧实现完全没有反馈的位置，按下去只会落到
    /// 第 1 列的表头上。
    #[gpui::test]
    fn dragging_column_editor_from_the_right_of_the_line_widens_the_column(
        cx: &mut gpui::TestAppContext,
    ) {
        let (designer, _armed, visual) = designer_window(cx, None);
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let width_before = designer_column_width(&designer, visual, COLUMN_NAME_COL);
        let grab = visual
            .debug_bounds(DESIGNER_HANDLE)
            .expect("第 0 列右缘的抓取区");
        let width_after = drag_column_editor(
            &designer,
            visual,
            gpui::point(grab.center().x + px(3.), grab.center().y),
        );

        assert!(
            width_after > width_before + px(DESIGNER_WIDENED_AT_LEAST),
            "从分隔线右侧拖动的也应是第 0 列被拖宽约 {DESIGNER_DRAG_DISTANCE}px：{width_before:?} → {width_after:?}"
        );
    }

    /// issue #333：在列边界**左侧**按下要能拖动列宽（旧实现这条路径是好的，别改坏）。
    #[gpui::test]
    fn dragging_column_editor_from_the_left_of_the_line_widens_the_column(
        cx: &mut gpui::TestAppContext,
    ) {
        let (designer, _armed, visual) = designer_window(cx, None);
        visual.update(|window, cx| {
            window.draw(cx).clear(cx);
        });

        let width_before = designer_column_width(&designer, visual, COLUMN_NAME_COL);
        let grab = visual
            .debug_bounds(DESIGNER_HANDLE)
            .expect("第 0 列右缘的抓取区");
        let width_after = drag_column_editor(
            &designer,
            visual,
            gpui::point(grab.center().x - px(3.), grab.center().y),
        );

        assert!(
            width_after > width_before + px(DESIGNER_WIDENED_AT_LEAST),
            "从分隔线左侧拖动也应把第 0 列拖宽约 {DESIGNER_DRAG_DISTANCE}px：{width_before:?} → {width_after:?}"
        );
    }

    /// 空白预览内容不进剪贴板，否则复制出来的是一串空气。
    #[test]
    fn blank_preview_sql_is_not_offered_for_copy() {
        assert_eq!(None, preview_sql_to_copy(""));
        assert_eq!(None, preview_sql_to_copy("  \n\t"));
    }

    /// 复制载荷必须与生成出来的 SQL 逐字一致（缩进和换行都不能动）。
    #[test]
    fn preview_sql_copy_payload_keeps_the_generated_sql_verbatim() {
        const SQL: &str = "CREATE TABLE `users` (\n  `id` INT\n);";

        assert_eq!(
            Some(SQL.to_string()),
            preview_sql_to_copy(SQL).and_then(|item| item.text())
        );
    }

    /// 复制按钮跟着页签走：预览类和 DDL 页签在「保存」左边出现一个，
    /// 其它页签没有可复制的 SQL，就不摆这个按钮。
    #[gpui::test]
    fn the_copy_action_follows_the_preview_and_ddl_tabs(cx: &mut gpui::TestAppContext) {
        const SQL: &str = "CREATE TABLE `users` (\n  `id` INT\n);";
        let (designer, _armed_at_open, visual) = designer_window(cx, None);

        for tab in [DesignerTab::SqlPreview, DesignerTab::Ddl] {
            visual.update(|window, cx| {
                designer.update(cx, |designer, cx| {
                    designer.active_tab = tab.clone();
                    let input = if designer.active_tab == DesignerTab::SqlPreview {
                        designer.sql_preview_input.clone()
                    } else {
                        designer.ddl_preview_input.clone()
                    };
                    input.update(cx, |state, cx| state.set_value(SQL, window, cx));
                    cx.notify();
                });
            });
            visual.update(|window, cx| {
                window.draw(cx).clear(cx);
            });

            assert!(
                visual.debug_bounds(PREVIEW_COPY_SELECTOR).is_some(),
                "{tab:?} 页签应当在工具栏渲染复制按钮"
            );
        }

        visual.update(|window, cx| {
            designer.update(cx, |designer, cx| {
                designer.active_tab = DesignerTab::Columns;
                cx.notify();
            });
            window.draw(cx).clear(cx);
        });

        assert!(
            visual.debug_bounds(PREVIEW_COPY_SELECTOR).is_none(),
            "列页签没有可复制的 SQL，不该渲染复制按钮"
        );
    }

    /// 复制按钮的词条必须真实存在。
    ///
    /// 词条写错时 `t!` 会退回字面 key 直接画在按钮上（用户看到的就是
    /// 「Query.copy_sql」），这类错误编译和渲染都不会报。
    #[test]
    fn the_copy_action_wording_is_translated() {
        for key in ["Table.copy_sql", "Table.sql_copied"] {
            assert_ne!(key, t!(key).as_ref(), "词条 `{key}` 缺失");
        }
    }

    /// 两个预览页签的编辑器都必须只读而不是禁用：禁用态收不到选区，
    /// 用户就没法自己在编辑器里选中片段复制。
    #[test]
    fn preview_editors_are_readonly_so_text_can_be_selected() {
        let source = include_str!("table_designer_tab.rs");

        for (name, field) in [
            ("render_sql_preview", "sql_preview_input"),
            ("render_ddl_preview", "ddl_preview_input"),
        ] {
            let body = function_body(source, name);
            assert!(
                body.contains(&format!("Editor::new(&self.{field})")),
                "{name} 应当渲染 {field} 的编辑器"
            );
            assert!(
                body.contains(".readonly(true)"),
                "{name} 的编辑器要只读，才能选中复制"
            );
            assert!(
                !body.contains(".disabled(true)"),
                "{name} 的编辑器不能是禁用态，否则选区都拿不到"
            );
        }
    }

    /// 列列表要能直接看到并改默认值（issue #477）：
    /// 表头有默认值列、行里是可编辑输入框、详情面板不再重复渲染同一个输入。
    #[test]
    fn the_columns_list_shows_the_default_value_column() {
        let source = include_str!("table_designer_tab.rs");

        let header = function_body(source, "render_table_header");
        let default_header = header
            .find("COLUMN_DEFAULT_COL")
            .expect("列列表表头要显示默认值列");
        let comment_header = header.find("Table.comment").expect("表头要有注释列");
        assert!(
            default_header < comment_header,
            "默认值列要排在注释列之前，和各家设计器的习惯顺序一致"
        );

        let row = function_body(source, "render_row");
        assert!(
            row.contains("column_width(COLUMN_DEFAULT_COL)"),
            "列列表行要按默认值列的宽度渲染单元格"
        );
        assert!(
            row.contains("Input::new(&row.default_input)"),
            "默认值要在列列表里直接编辑"
        );

        // 同一个 InputState 渲染两处，两处都会画出焦点框和光标。
        let detail = function_body(source, "render_detail_panel");
        assert!(
            !detail.contains("row.default_input"),
            "详情面板不能再渲染默认值输入框，否则和列表行抢同一份输入状态"
        );
        assert!(
            detail.contains("show_charset")
                && detail.contains("show_collation")
                && detail.contains("is_enum_or_set"),
            "详情面板只保留字符集 / 排序规则 / 枚举可选值"
        );
    }

    /// 工具栏里复制按钮摆在「保存」左边，而不是预览区里单独占一行。
    #[test]
    fn the_copy_action_sits_next_to_the_save_button_in_the_toolbar() {
        let toolbar = function_body(include_str!("table_designer_tab.rs"), "render_toolbar");

        let copy_at = toolbar
            .find("render_copy_action(PREVIEW_COPY_SELECTOR")
            .expect("工具栏要渲染复制按钮");
        let save_at = toolbar
            .find("Button::new(\"execute\")")
            .expect("工具栏要有保存按钮");

        assert!(
            copy_at < save_at,
            "复制按钮要放在保存按钮旁边（保存左边），而不是页签内容区"
        );
    }

    /// 截出 `fn <name>(` 的实现体，断言只看实现，不看别处。
    fn function_body<'a>(source: &'a str, name: &str) -> &'a str {
        let needle = format!("fn {name}(");
        let start = source.find(&needle).expect("function implementation");
        let body = &source[start..];
        let end = body.find("\n    fn ").expect("next method");
        &body[..end]
    }

    /// 构造表设计器窗口，返回 (designer, 打开瞬间是否在加载态, visual)。
    ///
    /// 加载态在构造处当场采样：结构查询是异步的，回过头再读字段就会和「查询已经
    /// 失败并收掉加载态」赛跑，采样点必须在同一个不中断的闭包内。
    fn designer_window<'a>(
        cx: &'a mut gpui::TestAppContext,
        table_name: Option<&str>,
    ) -> (Entity<TableDesigner>, bool, &'a mut gpui::VisualTestContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            one_core::gpui_tokio::init(cx);
            cx.set_global(GlobalDbState::default());
        });

        let mut config = TableDesignerConfig::new("conn-1", "app", DatabaseType::MySQL);
        if let Some(table_name) = table_name {
            config = config.with_table_name(table_name);
        }

        let armed_at_open = Rc::new(Cell::new(false));
        let probe = armed_at_open.clone();
        let (designer, visual) = cx.add_window_view(move |window, cx| {
            let designer = TableDesigner::new("表设计器", config, window, cx);
            probe.set(designer.structure_load.is_loading());
            designer
        });

        (designer, armed_at_open.get(), visual)
    }
}
