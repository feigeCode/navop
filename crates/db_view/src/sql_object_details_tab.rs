//! Tab that shows a SQL object's details (table/column/function markdown).
//!
//! Opened by Cmd/Ctrl+click on an identifier and by the editor context menu's
//! 「查看对象详情」. One tab per object: re-resolving the same object activates
//! the existing tab instead of stacking duplicates.
//!
//! 表对象的建表 DDL 不在 markdown 主体里，而是由 [`crate::table_ddl`] 按方言
//! 异步生成（与表设计器的 DDL 预览逐字一致）。所以页签先渲染列信息并显示
//! 「DDL 生成中…」，取到结果后就地替换。

use gpui::prelude::*;
use gpui::{
    App, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Window, div,
};
use one_assets::IconName;
use one_core::tab_container::{
    GlobalTabContainer, TabContainer, TabContent, TabContentEvent, TabItem,
};
use rust_i18n::t;

use crate::sql_editor_hover::{SqlObjectDetails, SqlObjectDetailsKind};
use crate::table_ddl::{
    SqlTableRef, TableDdlSection, TableDdlSources, load_ddl_section, with_ddl_section,
};

/// Tab id prefix; the object slug keeps one tab per object.
const TAB_ID_PREFIX: &str = "sql-object-details:";

/// `from` value for details tabs. Deliberately not a connection id: details are
/// a snapshot of the editor's schema, so they must not be swept away together
/// with a connection's tabs.
const TAB_SOURCE: &str = "sql-object-details";

/// Tab id for an object slug produced by
/// [`crate::sql_editor_hover::resolve_object_details`].
pub fn object_details_tab_id(object_id: &str) -> String {
    format!("{TAB_ID_PREFIX}{object_id}")
}

/// 打开对象详情页签需要的一切。
pub struct ObjectDetailsRequest {
    /// markdown 主体 + 对象身份（tab id / 标题 / 图标）。
    pub details: SqlObjectDetails,
    /// 建表 DDL 的来源；只有表对象（`details.table` 有值）才会用到。
    pub ddl: Option<TableDdlSources>,
    /// 发起方所在的页签容器，见 [`open_object_details_tab`]。
    pub host: Option<Entity<TabContainer>>,
}

/// 一次 DDL 生成的目标：表坐标 + 来源。
#[derive(Clone)]
pub struct DdlSubject {
    table: SqlTableRef,
    sources: TableDdlSources,
}

/// Opens the details tab for `request.details`, or re-activates it when it is
/// already open.
///
/// `request.host` is the tab container the requesting view lives in: details
/// are a sibling of the SQL editor tab, so they belong in the database tab's
/// inner container rather than the window-level tab bar. Embedded editors
/// (`host` is `None`) fall back to the window container.
///
/// The tab is added on the next effect cycle rather than on the spot: the
/// Cmd/Ctrl+click entry point runs inside the editor input's own `update`
/// (`InputBaseState::go_to_definition`), and activating a tab first deactivates
/// the one that is active - the SQL editor tab's `on_deactivate` writes back
/// into that same input state, which would be a double lease. Deferring lets
/// every entity leave the stack first.
///
/// Returns `false` when neither container is available (an editor outside any
/// window container).
pub fn open_object_details_tab(
    request: ObjectDetailsRequest,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let container = request.host.clone().or_else(|| {
        cx.try_global::<GlobalTabContainer>()
            .map(|global| global.primary_pane())
    });
    let Some(container) = container else {
        return false;
    };
    let content = DetailsTabContent::of(&request);
    window.defer(cx, move |window, cx| {
        add_object_details_tab(&content, &container, window, cx);
    });
    true
}

/// 开页签所需的静态内容（tab id 由对象 id 推出）。
#[derive(Clone)]
struct DetailsTabContent {
    tab_id: String,
    title: SharedString,
    icon: IconName,
    body: SharedString,
    ddl: Option<DdlSubject>,
}

impl DetailsTabContent {
    fn of(request: &ObjectDetailsRequest) -> Self {
        let details = &request.details;
        // 表坐标与来源齐备才去取 DDL；视图/列/函数本身就没有建表 DDL。
        let ddl = match (&details.table, &request.ddl) {
            (Some(table), Some(sources)) => Some(DdlSubject {
                table: table.clone(),
                sources: sources.clone(),
            }),
            _ => None,
        };
        Self {
            tab_id: object_details_tab_id(&details.id),
            title: t!(
                "Query.object_details_title",
                object = details.label.as_str()
            )
            .to_string()
            .into(),
            icon: object_details_icon(details.kind),
            body: details.markdown.clone().into(),
            ddl,
        }
    }
}

fn add_object_details_tab(
    content: &DetailsTabContent,
    container: &Entity<TabContainer>,
    window: &mut Window,
    cx: &mut App,
) {
    let tab_id = content.tab_id.clone();
    let content = content.clone();
    container.update(cx, |container, cx| {
        container.activate_or_add_tab_lazy(
            tab_id.clone(),
            move |_window, cx| {
                let tab = cx.new(|cx| SqlObjectDetailsTab::new(content.clone(), cx));
                TabItem::new(tab_id, TAB_SOURCE, tab)
            },
            window,
            cx,
        );
    });
}

fn object_details_icon(kind: SqlObjectDetailsKind) -> IconName {
    match kind {
        SqlObjectDetailsKind::Table => IconName::Table,
        SqlObjectDetailsKind::Column => IconName::Column,
        SqlObjectDetailsKind::Function => IconName::FileText,
    }
}

/// The details body itself: read-only markdown, selectable and scrollable.
struct SqlObjectDetailsTab {
    body: SharedString,
    ddl_section: Option<TableDdlSection>,
    title: SharedString,
    icon: IconName,
    focus_handle: FocusHandle,
}

impl SqlObjectDetailsTab {
    fn new(content: DetailsTabContent, cx: &mut Context<Self>) -> Self {
        // 先出列信息 + 「DDL 生成中…」，否则取 DDL 这段时间里看起来像是没有 DDL。
        let ddl_section = content.ddl.as_ref().map(|_| TableDdlSection::Loading);
        let tab = Self {
            body: content.body,
            ddl_section,
            title: content.title,
            icon: content.icon,
            focus_handle: cx.focus_handle(),
        };
        if let Some(ddl) = content.ddl {
            tab.load_ddl(ddl, cx);
        }
        tab
    }

    /// 取驱动生成的建表 DDL，拿到后就地替换页签内容。
    fn load_ddl(&self, ddl: DdlSubject, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let section = load_ddl_section(Some(&ddl.table), Some(&ddl.sources), cx).await;
            let _ = this.update(cx, |tab, cx| {
                tab.ddl_section = section;
                cx.notify();
            });
        })
        .detach();
    }

    /// 主体 + DDL 区段。每次渲染都重算，这样 DDL 回来后的 `notify` 就能生效。
    fn markdown(&self) -> SharedString {
        with_ddl_section(&self.body, self.ddl_section.as_ref()).into()
    }
}

impl Focusable for SqlObjectDetailsTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for SqlObjectDetailsTab {}

impl Render for SqlObjectDetailsTab {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("sql-object-details")
            .size_full()
            .p_3()
            .overflow_y_scroll()
            .child(
                gpui_base::TextView::markdown("sql-object-details-markdown", self.markdown())
                    .selectable(true),
            )
    }
}

impl TabContent for SqlObjectDetailsTab {
    fn content_key(&self) -> &'static str {
        "SqlObjectDetails"
    }

    fn title(&self, _cx: &App) -> SharedString {
        self.title.clone()
    }

    fn icon(&self, _cx: &App) -> Option<gpui_component::Icon> {
        Some(self.icon.color())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;
    use one_core::storage::DatabaseType;

    fn table_details() -> SqlObjectDetails {
        SqlObjectDetails {
            markdown: "**TABLE** `users`".into(),
            range: 14..19,
            id: "app.public:table:users".into(),
            label: "users".into(),
            kind: SqlObjectDetailsKind::Table,
            table: Some(SqlTableRef {
                name: "users".into(),
                database: "app".into(),
                schema: Some("public".into()),
            }),
        }
    }

    /// 视图与列一样没有建表 DDL，所以 `table` 是 `None`。
    fn view_details() -> SqlObjectDetails {
        SqlObjectDetails {
            kind: SqlObjectDetailsKind::Table,
            table: None,
            label: "active_users".into(),
            id: "app.public:table:active_users".into(),
            ..table_details()
        }
    }

    /// 桩 DDL 来源：不连库。
    fn stub_sources(ddl: &'static str) -> TableDdlSources {
        TableDdlSources::new(
            crate::table_ddl::TableDdlLoader::new(move |_cx, _target| {
                gpui::Task::ready(Ok(ddl.to_string()))
            }),
            stub_context(),
        )
    }

    /// 桩 DDL 来源：永远不会返回，用来观察「生成中」这段时间的页签。
    fn stalling_sources() -> TableDdlSources {
        TableDdlSources::new(
            crate::table_ddl::TableDdlLoader::new(|cx, _target| {
                cx.spawn(async move |_cx: &mut AsyncApp| -> anyhow::Result<String> {
                    smol::future::pending::<()>().await;
                    unreachable!("the stalled loader must never resolve")
                })
            }),
            stub_context(),
        )
    }

    fn stub_context() -> crate::table_ddl::TableDdlContext {
        crate::table_ddl::TableDdlContext {
            connection_id: "conn-1".into(),
            database_type: DatabaseType::MySQL,
        }
    }

    fn content_of(details: SqlObjectDetails, ddl: Option<TableDdlSources>) -> DetailsTabContent {
        DetailsTabContent::of(&ObjectDetailsRequest {
            details,
            ddl,
            host: None,
        })
    }

    #[test]
    fn tab_ids_are_prefixed_so_details_tabs_never_collide_with_other_tabs() {
        assert_eq!(
            object_details_tab_id("app.public:table:users"),
            "sql-object-details:app.public:table:users"
        );
    }

    #[test]
    fn object_kinds_get_distinct_icons() {
        let icons = [
            object_details_icon(SqlObjectDetailsKind::Table),
            object_details_icon(SqlObjectDetailsKind::Column),
            object_details_icon(SqlObjectDetailsKind::Function),
        ];

        for (index, icon) in icons.iter().enumerate() {
            assert!(
                !icons[..index].contains(icon),
                "each object kind needs its own icon"
            );
        }
    }

    #[test]
    fn ddl_is_only_requested_for_tables_that_have_a_source() {
        let ddl = Some(stub_sources("CREATE TABLE `users` (`id` INT);"));

        assert!(content_of(table_details(), ddl.clone()).ddl.is_some());
        assert!(
            content_of(table_details(), None).ddl.is_none(),
            "没有连接上下文（内嵌编辑器）时不取 DDL"
        );
        assert!(
            content_of(view_details(), ddl).ddl.is_none(),
            "视图没有建表 DDL"
        );
    }

    /// 只看实体状态，不需要真的画出来。
    struct DetailsTabHarness {
        tab: Entity<SqlObjectDetailsTab>,
    }

    impl Render for DetailsTabHarness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().child(self.tab.clone())
        }
    }

    #[gpui::test]
    fn the_details_tab_swaps_the_loading_note_for_the_loader_ddl(cx: &mut TestAppContext) {
        const DDL: &str = "CREATE TABLE `users` (\n  `id` INT\n);";
        let (tab, visual) = details_tab(cx, stub_sources(DDL));

        visual.run_until_parked();

        let loaded = visual.read(|cx| tab.read(cx).markdown().to_string());
        assert!(loaded.contains(DDL), "驱动返回的 DDL 要落到页签里");
        assert!(
            !loaded.contains(&t!("Query.object_details_ddl_loading").to_string()),
            "生成完成后不该再显示加载提示"
        );
    }

    /// 取 DDL 期间先出加载提示，而不是看起来像没有 DDL。
    #[gpui::test]
    fn the_details_tab_says_the_ddl_is_being_generated(cx: &mut TestAppContext) {
        let (tab, visual) = details_tab(cx, stalling_sources());

        let loading = visual.read(|cx| tab.read(cx).markdown().to_string());

        assert!(loading.contains(&t!("Query.object_details_ddl_loading").to_string()));
        assert!(!loading.contains("```sql"), "还没拿到 DDL 就不先摆空代码块");
    }

    /// 在测试窗口里开一个只带给定 DDL 来源的详情页签。
    fn details_tab(
        cx: &mut TestAppContext,
        sources: TableDdlSources,
    ) -> (Entity<SqlObjectDetailsTab>, &mut gpui::VisualTestContext) {
        let mut tab = None;
        let (_, visual) = cx.add_window_view(|_window, cx| {
            gpui_component::init(cx);
            let content = content_of(table_details(), Some(sources));
            let entity = cx.new(|cx| SqlObjectDetailsTab::new(content, cx));
            tab = Some(entity.clone());
            DetailsTabHarness { tab: entity }
        });
        (tab.expect("the details tab should be created"), visual)
    }
}
