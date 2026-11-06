//! 独立产品的连接表单宿主（新建 / 编辑 / 复制）。
//!
//! 「新建连接」是两步向导，交互沿用产品分离前的旧版样式：
//! 第 1 步整页选择连接类型（左侧类目 + 右侧卡片网格），第 2 步渲染
//! 对应表单，表单左下角提供「上一步」返回类型选择。
//! 类型清单由领域通过 [`domain_api::ConnectionFormProvider`] 注册，因此
//! 多产品构建（全量 / Data / Shell）各自只展示已装配的类型，不写死任何一种。
//! 表单保存后自身写仓库并广播 `ConnectionDataEvent`，宿主列表据此刷新。

use domain_api::{
    ConnectionFormRequest, ConnectionKindDescriptor, ConnectionKindGroup, ConnectionKindIcon,
};
use gpui::{
    AnyView, AnyWindowHandle, App, AppContext as _, Context, FocusHandle, Focusable, FontWeight,
    IntoElement, KeyBinding, Render, SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::WindowExt as _;
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, InteractiveElementExt as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    scroll::ScrollableElement as _,
    v_flex,
};
use one_assets::IconName;
use one_core::popup_window::{PopupWindowOptions, open_reusable_popup_window};
use one_core::storage::StoredConnection;
use rust_i18n::t;

const DIALOG_WIDTH: f32 = 980.0;
const DIALOG_HEIGHT: f32 = 660.0;
const DIALOG_REUSE_KEY: &str = "app-kit-connection-form";

/// 类型选择页的键盘上下文。
///
/// 只挂在第 1 步的容器上：第 2 步表单里的输入框回车不会被向导拦截。
const WIZARD_KEY_CONTEXT: &str = "ConnectionKindWizard";

const CATEGORY_SIDEBAR_WIDTH: f32 = 180.0;
const KIND_CARD_HEIGHT: f32 = 112.0;
const KIND_ICON_SIZE: f32 = 40.0;
const KIND_ICON_BOX: f32 = 48.0;

/// 领域注册的种类 id 常量（与 data_domain / shell_domain 的注册保持一致）。
const DB_KIND_PREFIX: &str = "db:";
const KIND_REDIS: &str = "redis";
const KIND_MONGODB: &str = "mongodb";
const KIND_SSH: &str = "ssh";
const KIND_FTP: &str = "ftp";
const KIND_SERIAL: &str = "serial";
const KIND_TELNET: &str = "telnet";
const KIND_PORT_FORWARDING: &str = "port_forwarding";
const KIND_RDP: &str = "rdp";
const KIND_VNC: &str = "vnc";

gpui::actions!(
    connection_kind_wizard,
    [WizardSelectPrevious, WizardSelectNext, WizardOpenSelected]
);

/// 表单用途：决定标题与请求构造方式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnectionFormMode {
    New,
    Edit,
    Duplicate,
}

impl ConnectionFormMode {
    fn title(self) -> SharedString {
        match self {
            Self::New => t!("NewConnection.title_new").to_string().into(),
            Self::Edit => t!("NewConnection.title_edit").to_string().into(),
            Self::Duplicate => t!("NewConnection.title_duplicate").to_string().into(),
        }
    }

    fn request(
        self,
        kind_id: SharedString,
        connection: Option<StoredConnection>,
    ) -> ConnectionFormRequest {
        match (self, connection) {
            (Self::Edit, Some(connection)) => ConnectionFormRequest::editing(kind_id, connection),
            (Self::Duplicate, Some(connection)) => {
                ConnectionFormRequest::duplicate(kind_id, connection)
            }
            _ => ConnectionFormRequest::new(kind_id),
        }
    }
}

/// 新建连接向导。
pub fn open_new_connection(window: &mut Window, cx: &mut App) {
    open_new_connection_in_workspace(None, window, cx);
}

/// 新建连接向导，并预选工作区分组。
pub fn open_new_connection_in_workspace(
    workspace_id: Option<i64>,
    window: &mut Window,
    cx: &mut App,
) {
    open_dialog(ConnectionFormMode::New, None, workspace_id, window, cx);
}

/// 编辑已有连接。
pub fn edit_connection(connection: StoredConnection, window: &mut Window, cx: &mut App) {
    open_for_existing(ConnectionFormMode::Edit, connection, window, cx);
}

/// 复制已有连接：预填表单，保存后生成新连接。
pub fn duplicate_connection(connection: StoredConnection, window: &mut Window, cx: &mut App) {
    open_for_existing(ConnectionFormMode::Duplicate, connection, window, cx);
}

fn open_for_existing(
    mode: ConnectionFormMode,
    connection: StoredConnection,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(kind_id) = crate::registry().kind_for_connection(&connection, cx) else {
        window.push_notification(
            Notification::error(format!("不支持的连接类型：{}", connection.connection_type)),
            cx,
        );
        return;
    };
    let workspace_id = connection.workspace_id;
    open_dialog(mode, Some((kind_id, connection)), workspace_id, window, cx);
}

fn open_dialog(
    mode: ConnectionFormMode,
    preset: Option<(SharedString, StoredConnection)>,
    workspace_id: Option<i64>,
    window: &mut Window,
    cx: &mut App,
) {
    // 连接参数里可能带密码，保存时会走主密钥加密；与全量产品一致，先确保
    // 主密钥可用，未设置/已锁定时先弹设置或解锁弹窗，本次不继续打开表单。
    if !crate::vault::ensure_master_key_ready(window, cx) {
        return;
    }
    // 记住父窗口：向导里的「更多连接 / 安装扩展」需要回到主窗口打开扩展管理 tab。
    let parent_window = window.window_handle();
    open_reusable_popup_window(
        PopupWindowOptions::new(mode.title()).size(DIALOG_WIDTH, DIALOG_HEIGHT),
        DIALOG_REUSE_KEY,
        move |window, cx| {
            cx.new(|cx| {
                ConnectionFormDialog::new(
                    mode,
                    preset.clone(),
                    workspace_id,
                    parent_window,
                    window,
                    cx,
                )
            })
        },
        Some(window),
        cx,
    );
}

/// 向导当前所处步骤。
#[derive(Clone, Copy, PartialEq, Eq)]
enum WizardStep {
    /// 第 1 步：选择连接类型。
    SelectKind,
    /// 第 2 步：连接表单。
    Form,
}

/// 类型选择页左侧的类目。
///
/// 与产品分离前的旧版向导保持一致：固定 7 个类目，即使当前产品没有任何
/// 归属某类目的连接类型也照常展示（便于用户从「安装扩展」入口补齐）。
///
/// 扩展贡献的连接不单独成「扩展」类目：只要它带连接入口就归入中间件。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WizardCategory {
    All,
    Database,
    DomesticDatabase,
    TimeSeries,
    NoSql,
    Middleware,
    Terminal,
}

impl WizardCategory {
    const ALL: [Self; 7] = [
        Self::All,
        Self::Database,
        Self::DomesticDatabase,
        Self::TimeSeries,
        Self::NoSql,
        Self::Middleware,
        Self::Terminal,
    ];

    fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Database => "database",
            Self::DomesticDatabase => "domestic_database",
            Self::TimeSeries => "time_series",
            Self::NoSql => "nosql",
            Self::Middleware => "middleware",
            Self::Terminal => "terminal",
        }
    }

    /// 「全部」类目始终匹配；其余类目只匹配自身。
    fn matches(self, category: Self) -> bool {
        self == Self::All || self == category
    }
}

/// 第 1 步列表里的一个条目：真实连接类型，或「安装扩展 / 更多连接」入口。
#[derive(Clone)]
enum WizardEntry {
    Kind(ConnectionKindDescriptor),
    /// 空类目的「+」安装入口：点击进入扩展管理页安装对应驱动。
    Install(WizardCategory),
    /// 列表末尾的「更多连接」入口。
    More,
}

impl WizardEntry {
    /// 选中状态使用稳定 key 保存；伪条目用 `@` 前缀避免和领域 id 冲突。
    fn key(&self) -> SharedString {
        match self {
            Self::Kind(kind) => kind.id.clone(),
            Self::Install(category) => SharedString::from(format!("@install-{}", category.key())),
            Self::More => SharedString::from("@more-connections"),
        }
    }

    fn category(&self) -> WizardCategory {
        match self {
            Self::Kind(kind) => kind_category(kind),
            Self::Install(category) => *category,
            Self::More => WizardCategory::All,
        }
    }

    fn label(&self, cx: &App) -> SharedString {
        match self {
            Self::Kind(kind) => kind.display_label(cx),
            Self::Install(WizardCategory::Middleware) => {
                t!("NewConnection.more_middleware").to_string().into()
            }
            Self::Install(WizardCategory::TimeSeries) => {
                t!("NewConnection.more_time_series").to_string().into()
            }
            Self::Install(_) | Self::More => {
                t!("NewConnection.more_connections").to_string().into()
            }
        }
    }

    fn description(&self) -> SharedString {
        match self {
            Self::Kind(kind) => kind_description(kind),
            Self::Install(_) | Self::More => t!("NewConnection.description_more_connections")
                .to_string()
                .into(),
        }
    }

    fn icon(&self) -> Icon {
        match self {
            Self::Kind(kind) => kind_icon(kind),
            Self::Install(_) | Self::More => IconName::Plus.mono().with_size(px(KIND_ICON_SIZE)),
        }
    }

    /// 点击后是否跳转扩展管理页，而不是打开连接表单。
    fn opens_extensions(&self) -> bool {
        !matches!(self, Self::Kind(_))
    }
}

/// 表单弹窗视图。
pub struct ConnectionFormDialog {
    focus_handle: FocusHandle,
    kinds: Vec<ConnectionKindDescriptor>,
    step: WizardStep,
    category: WizardCategory,
    /// 当前选中的条目 key（真实种类 id，或 `@install-*` / `@more-connections`）。
    selection: Option<SharedString>,
    /// 第 2 步的表单视图；与 `form_kind` 一起缓存，返回上一步时不丢已填内容。
    form: Option<AnyView>,
    form_kind: Option<SharedString>,
    mode: ConnectionFormMode,
    preset: Option<(SharedString, StoredConnection)>,
    workspace_id: Option<i64>,
    /// 父窗口：「更多连接 / 安装扩展」入口需要在主窗口里打开扩展管理 tab。
    parent_window: AnyWindowHandle,
}

impl ConnectionFormDialog {
    fn new(
        mode: ConnectionFormMode,
        preset: Option<(SharedString, StoredConnection)>,
        workspace_id: Option<i64>,
        parent_window: AnyWindowHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kinds = crate::registry().kinds(cx);
        let mut dialog = Self {
            focus_handle: cx.focus_handle(),
            kinds,
            step: WizardStep::SelectKind,
            category: WizardCategory::All,
            selection: None,
            form: None,
            form_kind: None,
            mode,
            preset,
            workspace_id,
            parent_window,
        };
        if dialog.mode == ConnectionFormMode::New {
            // 与旧版一致：默认选中「全部」里的第一项，让「下一步」立即可用。
            dialog.selection = dialog
                .entries_in(WizardCategory::All)
                .first()
                .map(WizardEntry::key);
        } else if let Some((kind_id, _)) = dialog.preset.clone() {
            // 编辑 / 复制跳过类型选择，直接进表单。
            dialog.selection = Some(kind_id.clone());
            dialog.open_kind(kind_id, window, cx);
        }
        // 上下左右 + 回车：第 1 步选择类型，与旧版向导一致。
        cx.bind_keys([
            KeyBinding::new("up", WizardSelectPrevious, Some(WIZARD_KEY_CONTEXT)),
            KeyBinding::new("left", WizardSelectPrevious, Some(WIZARD_KEY_CONTEXT)),
            KeyBinding::new("down", WizardSelectNext, Some(WIZARD_KEY_CONTEXT)),
            KeyBinding::new("right", WizardSelectNext, Some(WIZARD_KEY_CONTEXT)),
            KeyBinding::new("enter", WizardOpenSelected, Some(WIZARD_KEY_CONTEXT)),
        ]);
        dialog.focus_handle.focus(window, cx);
        dialog
    }

    /// 是否走两步向导（只有「新建」需要先选类型）。
    fn is_wizard(&self) -> bool {
        self.mode == ConnectionFormMode::New
    }

    /// 左侧类目：与旧版一致固定 8 个（空类目也照常展示）。
    fn categories(&self) -> [WizardCategory; 7] {
        WizardCategory::ALL
    }

    /// 当前产品是否装配了数据库领域：决定是否展示「安装时序 / 中间件」入口。
    fn has_database_kinds(&self) -> bool {
        self.kinds.iter().any(|kind| {
            matches!(
                kind_category(kind),
                WizardCategory::Database | WizardCategory::TimeSeries | WizardCategory::NoSql
            )
        })
    }

    /// 类目下的条目：真实种类按旧版顺序排列，空类目补「+」入口，最后放「更多连接」。
    fn entries_in(&self, category: WizardCategory) -> Vec<WizardEntry> {
        let mut entries: Vec<(u8, usize, WizardEntry)> = self
            .kinds
            .iter()
            .enumerate()
            .filter_map(|(index, kind)| {
                let entry = WizardEntry::Kind(kind.clone());
                if !category.matches(entry.category()) {
                    return None;
                }
                let (bucket, rank) = kind_sort_key(kind, index);
                Some((bucket, rank, entry))
            })
            .collect();
        entries.sort_by_key(|(bucket, rank, _)| (*bucket, *rank));
        let mut entries: Vec<WizardEntry> =
            entries.into_iter().map(|(_, _, entry)| entry).collect();
        // 旧版只为「中间件 / 时序数据库」这两个依赖外部驱动填充的类目补安装入口。
        for install_category in [WizardCategory::Middleware, WizardCategory::TimeSeries] {
            if category != install_category || !self.has_database_kinds() {
                continue;
            }
            if !self
                .kinds
                .iter()
                .any(|kind| kind_category(kind) == install_category)
            {
                entries.push(WizardEntry::Install(install_category));
            }
        }
        if category == WizardCategory::All {
            entries.push(WizardEntry::More);
        }
        entries
    }

    fn select_category(&mut self, category: WizardCategory, cx: &mut Context<Self>) {
        if self.category == category {
            return;
        }
        self.category = category;
        let entries = self.entries_in(category);
        if !entries
            .iter()
            .any(|entry| Some(&entry.key()) == self.selection.as_ref())
        {
            self.selection = entries.first().map(WizardEntry::key);
        }
        cx.notify();
    }

    fn select_entry(&mut self, key: SharedString, cx: &mut Context<Self>) {
        if self.selection.as_ref() == Some(&key) {
            return;
        }
        self.selection = Some(key);
        cx.notify();
    }

    /// 键盘上下移动选中项（在当前类目内循环）。
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.step != WizardStep::SelectKind {
            return;
        }
        let entries = self.entries_in(self.category);
        if entries.is_empty() {
            return;
        }
        let current = self
            .selection
            .as_ref()
            .and_then(|selected| entries.iter().position(|entry| &entry.key() == selected));
        let next = match current {
            Some(index) => (index as isize + delta).rem_euclid(entries.len() as isize) as usize,
            None if delta < 0 => entries.len() - 1,
            None => 0,
        };
        self.selection = Some(entries[next].key());
        cx.notify();
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.selection.clone() else {
            return;
        };
        self.open_entry(key, window, cx);
    }

    /// 打开一个条目：真实种类进表单，伪条目跳扩展管理页。
    fn open_entry(&mut self, key: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self
            .entries_in(WizardCategory::All)
            .into_iter()
            .find(|entry| entry.key() == key)
        else {
            return;
        };
        if entry.opens_extensions() {
            self.open_extensions(window, cx);
        } else if let WizardEntry::Kind(kind) = entry {
            self.open_kind(kind.id, window, cx);
        }
    }

    /// 切到主窗口的扩展管理 tab，并关闭当前向导。
    fn open_extensions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opened = crate::host::extensions_host()
            .is_some_and(|host| host.open_extensions(self.parent_window, window, cx));
        if opened {
            let _ = one_core::window_close::close_window_for_reuse(window, cx);
            return;
        }
        window.push_notification(Notification::error("扩展管理页不可用".to_string()), cx);
    }

    fn open_kind(&mut self, kind_id: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        if self.form_kind.as_ref() != Some(&kind_id) {
            let connection = self
                .preset
                .as_ref()
                .map(|(_, connection)| connection.clone());
            let Some(form) = build_form(
                kind_id.clone(),
                self.mode,
                connection,
                self.workspace_id,
                window,
                cx,
            ) else {
                window.push_notification(
                    Notification::error(format!("当前产品不支持该连接类型：{kind_id}")),
                    cx,
                );
                return;
            };
            self.form = Some(form);
            self.form_kind = Some(kind_id.clone());
        }
        self.selection = Some(kind_id);
        self.step = WizardStep::Form;
        cx.notify();
    }

    fn go_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.step = WizardStep::SelectKind;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn on_select_previous(
        &mut self,
        _: &WizardSelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(-1, cx);
    }

    fn on_select_next(&mut self, _: &WizardSelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn on_open_selected(
        &mut self,
        _: &WizardOpenSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_selected(window, cx);
    }

    /// 第 1 步：左侧类目 + 右侧卡片网格 + 底部「取消 / 下一步」。
    fn render_selection_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context(WIZARD_KEY_CONTEXT)
            .size_full()
            .min_h_0()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_open_selected))
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_hidden()
                    .child(self.render_category_sidebar(cx))
                    .child(self.render_card_area(cx)),
            )
            .child(self.render_selection_footer(cx))
    }

    fn render_category_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = v_flex()
            .flex_none()
            .w(px(CATEGORY_SIDEBAR_WIDTH))
            .h_full()
            .min_h_0()
            .p_2()
            .gap_2()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border);
        for category in self.categories() {
            let selected = self.category == category;
            let theme = cx.theme();
            sidebar = sidebar.child(
                div()
                    .id(SharedString::from(format!(
                        "connection-kind-category-{}",
                        category.key()
                    )))
                    .flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .cursor_pointer()
                    .when(selected, |item| {
                        item.bg(theme.list_active)
                            .border_l_3()
                            .border_color(theme.list_active_border)
                    })
                    .when(!selected, |item| {
                        item.hover(|item| item.bg(theme.sidebar_accent))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_category(category, cx);
                    }))
                    .child(
                        Icon::new(category_icon(category))
                            .mono()
                            .with_size(px(16.0)),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.foreground)
                            .when(selected, |label| label.font_weight(FontWeight::MEDIUM))
                            .child(category_title(category)),
                    ),
            );
        }
        sidebar
    }

    fn render_card_area(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entries = self.entries_in(self.category);
        let content = v_flex()
            .size_full()
            .overflow_y_scrollbar()
            .bg(cx.theme().muted)
            .p_6()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().foreground)
                            .child(t!("NewConnection.select_type_title").to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("NewConnection.select_type_hint").to_string()),
                    ),
            );
        let card_area = div()
            .flex_1()
            .h_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden();
        // 与旧版一致：没有成员的类目留空白，由「+」安装入口或更多连接引导。
        if entries.is_empty() {
            return card_area.child(content);
        }
        // 默认三列等宽网格；卡片数量随类目变化，列数保持不变。
        let mut grid = div().grid().grid_cols(3).w_full().gap_3();
        for entry in &entries {
            grid = grid.child(self.render_kind_card(entry, cx));
        }
        card_area.child(content.child(grid))
    }

    fn render_kind_card(&self, entry: &WizardEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let key = entry.key();
        let selected = self.selection.as_ref() == Some(&key);
        let click_entry = entry.clone();
        let double_click_key = key.clone();
        let label = entry.label(cx);
        let description = entry.description();
        let icon = entry.icon();
        let theme = cx.theme();
        v_flex()
            .id(SharedString::from(format!("connection-kind-card-{key}")))
            .justify_center()
            .w_full()
            .h(px(KIND_CARD_HEIGHT))
            .p_3()
            .rounded_lg()
            .border_1()
            .relative()
            .bg(theme.background)
            .border_color(if selected {
                theme.list_active_border
            } else {
                theme.border
            })
            .shadow_sm()
            .when(selected, |card| card.shadow_lg().border_l_3())
            .hover(|card| card.shadow_lg().border_color(theme.list_active_border))
            .cursor_pointer()
            .overflow_hidden()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_entry(click_entry.key(), cx);
                // 与旧版一致：「更多连接」点击即跳转，安装入口等「下一步 / 回车」。
                if matches!(&click_entry, WizardEntry::More) {
                    this.open_entry(click_entry.key(), window, cx);
                }
            }))
            .on_double_click(cx.listener(move |this, _, window, cx| {
                this.open_entry(double_click_key.clone(), window, cx);
            }))
            .child(
                h_flex()
                    .items_center()
                    .gap_3()
                    .w_full()
                    .child(
                        div()
                            .w(px(KIND_ICON_BOX))
                            .h(px(KIND_ICON_BOX))
                            .rounded(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(icon),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme.foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(label),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .child(description),
                            ),
                    ),
            )
    }

    fn render_selection_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .flex_none()
            .w_full()
            .justify_end()
            .gap_2()
            .p_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                Button::new("connection-kind-cancel")
                    .small()
                    .label(t!("NewConnection.cancel").to_string())
                    .on_click(cx.listener(|_, _, window, cx| {
                        let _ = one_core::window_close::close_window_for_reuse(window, cx);
                    })),
            )
            .child(
                Button::new("connection-kind-next")
                    .small()
                    .primary()
                    .label(t!("NewConnection.next").to_string())
                    .disabled(self.selection.is_none())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_selected(window, cx);
                    })),
            )
    }

    /// 第 2 步：表单页，左下角浮「上一步」返回类型选择。
    fn render_form_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex().flex_1().min_h_0().min_w_0().relative();
        body = match self.form.clone() {
            Some(form) => body.child(div().flex_1().min_h_0().min_w_0().child(form)),
            None => body.child(
                v_flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("NewConnection.empty").to_string()),
            ),
        };
        if self.is_wizard() {
            body = body.child(
                div().absolute().left(px(16.0)).bottom(px(16.0)).child(
                    Button::new("connection-kind-back")
                        .small()
                        .outline()
                        .label(t!("NewConnection.previous").to_string())
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.go_back(window, cx);
                        })),
                ),
            );
        }
        v_flex()
            .size_full()
            .min_h_0()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .child(body)
    }
}

/// 以通用请求直接打开连接表单弹窗（导入草稿等宿主复用）。
///
/// 表单自身写仓库并关窗；宿主通过 `request.on_saved` 刷新自己的列表。
pub fn open_form(request: ConnectionFormRequest, window: &mut Window, cx: &mut App) -> bool {
    if !crate::vault::ensure_master_key_ready(window, cx) {
        return false;
    }
    let kind_id = request.kind_id.clone();
    let Some(form) = crate::registry().build_connection_form(request.clone(), window, cx) else {
        window.push_notification(
            Notification::error(format!("当前产品不支持该连接类型：{kind_id}")),
            cx,
        );
        return false;
    };
    let reuse_key = format!("{DIALOG_REUSE_KEY}:{kind_id}");
    open_reusable_popup_window(
        PopupWindowOptions::new(t!("NewConnection.window_title").to_string())
            .size(DIALOG_WIDTH, DIALOG_HEIGHT),
        reuse_key,
        move |_window, _cx| form.clone(),
        Some(window),
        cx,
    );
    true
}

fn build_form(
    kind_id: SharedString,
    mode: ConnectionFormMode,
    connection: Option<StoredConnection>,
    workspace_id: Option<i64>,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyView> {
    let request = mode
        .request(kind_id, connection)
        .with_workspace(workspace_id);
    crate::registry().build_connection_form(request, window, cx)
}

fn category_title(category: WizardCategory) -> SharedString {
    let title = match category {
        WizardCategory::All => t!("NewConnection.category_all").to_string(),
        WizardCategory::Database => t!("NewConnection.category_database").to_string(),
        WizardCategory::DomesticDatabase => {
            t!("NewConnection.category_domestic_database").to_string()
        }
        WizardCategory::TimeSeries => t!("NewConnection.category_time_series").to_string(),
        // 与旧版一致：这两个类目的标签走英文固定文案。
        WizardCategory::NoSql => "NoSQL".to_string(),
        WizardCategory::Middleware => t!("NewConnection.category_middleware").to_string(),
        WizardCategory::Terminal => t!("NewConnection.category_terminal").to_string(),
    };
    title.into()
}

fn category_icon(category: WizardCategory) -> IconName {
    match category {
        WizardCategory::All => IconName::LayoutDashboard,
        WizardCategory::Database | WizardCategory::DomesticDatabase => IconName::DatabaseLine,
        WizardCategory::TimeSeries => IconName::ChartPie,
        WizardCategory::NoSql => IconName::Server,
        WizardCategory::Middleware => IconName::Network,
        WizardCategory::Terminal => IconName::Terminal,
    }
}

/// 连接类型所属的旧版类目。
fn kind_category(kind: &ConnectionKindDescriptor) -> WizardCategory {
    if let Some(category) = kind
        .category()
        .and_then(|category| manifest_category(category.as_ref()))
    {
        return category;
    }
    if let Some(db_type) = kind.id.strip_prefix(DB_KIND_PREFIX) {
        return if db_type == "TDengine" {
            WizardCategory::TimeSeries
        } else {
            WizardCategory::Database
        };
    }
    match kind.id.as_ref() {
        KIND_REDIS | KIND_MONGODB => WizardCategory::NoSql,
        _ => match kind.group {
            ConnectionKindGroup::Database => WizardCategory::Database,
            ConnectionKindGroup::Terminal | ConnectionKindGroup::Remote => WizardCategory::Terminal,
            // 扩展贡献的连接统一归入中间件，不再单列「扩展」类目。
            ConnectionKindGroup::Extension => WizardCategory::Middleware,
        },
    }
}

/// 驱动清单声明的类目 → 向导类目；未知取值交给壳按分组推导。
fn manifest_category(category: &str) -> Option<WizardCategory> {
    match category {
        "database" => Some(WizardCategory::Database),
        "domestic_database" => Some(WizardCategory::DomesticDatabase),
        "time_series" => Some(WizardCategory::TimeSeries),
        "nosql" => Some(WizardCategory::NoSql),
        "middleware" => Some(WizardCategory::Middleware),
        "terminal" => Some(WizardCategory::Terminal),
        "extension" | "extensions" => Some(WizardCategory::Middleware),
        _ => None,
    }
}

/// 「全部」类目里的旧版顺序：终端 / 远程类在前，数据库类按注册顺序在后。
///
/// 返回 `(bucket, rank)`：bucket 0 为内置终端类，按固定 rank 排；bucket 1 为
/// 其余类型，rank 用注册顺序，从而保持各产品注册的顺序不被改写。
fn kind_sort_key(kind: &ConnectionKindDescriptor, index: usize) -> (u8, usize) {
    let rank = match kind.id.as_ref() {
        KIND_SSH => Some(0),
        KIND_RDP => Some(1),
        KIND_VNC => Some(2),
        KIND_REDIS => Some(3),
        KIND_MONGODB => Some(4),
        KIND_SERIAL => Some(5),
        KIND_TELNET => Some(6),
        KIND_FTP => Some(7),
        KIND_PORT_FORWARDING => Some(8),
        _ => None,
    };
    match rank {
        Some(rank) => (0, rank),
        None => (1, index),
    }
}

fn group_icon(group: ConnectionKindGroup) -> IconName {
    match group {
        ConnectionKindGroup::Database => IconName::Database,
        ConnectionKindGroup::Terminal => IconName::Terminal,
        ConnectionKindGroup::Remote => IconName::Monitor,
        ConnectionKindGroup::Extension => IconName::ExtensionsColor,
    }
}

fn kind_icon(kind: &ConnectionKindDescriptor) -> Icon {
    // 领域解析好的图标优先（外部驱动清单里的品牌图标）。
    if let Some(icon) = kind.icon() {
        return match icon {
            ConnectionKindIcon::Asset(path) => {
                crate::visuals::icon_from_asset_path(path.clone(), px(KIND_ICON_SIZE))
            }
            ConnectionKindIcon::File(path) => {
                crate::visuals::icon_from_file_path(path.clone(), px(KIND_ICON_SIZE))
            }
        };
    }
    let kind_id = kind.id.as_ref();
    let group = kind.group;
    if let Some(db_type) = kind_id.strip_prefix(DB_KIND_PREFIX) {
        return database_kind_icon(db_type);
    }
    let icon = match kind_id {
        KIND_REDIS => IconName::RedisColor,
        KIND_MONGODB => IconName::MongoDB,
        KIND_SSH => IconName::TerminalColor,
        KIND_FTP => IconName::FtpColor,
        KIND_SERIAL => IconName::SerialPort,
        KIND_TELNET => IconName::SquareTerminalColor,
        KIND_PORT_FORWARDING => IconName::PortForwardingColor,
        KIND_RDP => IconName::Rdp,
        KIND_VNC => IconName::Vnc,
        _ => group_icon(group),
    };
    icon.color().with_size(px(KIND_ICON_SIZE))
}

fn database_kind_icon(db_type: &str) -> Icon {
    // TDengine 的品牌图标由全量产品的 AppAssets 按路径内嵌提供。
    if db_type == "TDengine" {
        return Icon::default()
            .path(one_core::storage::NAVOP_TDENGINE_COLOR_ICON)
            .color()
            .with_size(px(KIND_ICON_SIZE));
    }
    let icon = match db_type {
        "MySQL" => IconName::MySQLColor,
        "PostgreSQL" => IconName::PostgreSQLColor,
        "SQLite" => IconName::SQLiteColor,
        "DuckDB" => IconName::DuckDB,
        "MSSQL" => IconName::MSSQLColor,
        "Oracle" => IconName::OracleColor,
        "ClickHouse" => IconName::ClickHouseColor,
        _ => IconName::Database,
    };
    icon.color().with_size(px(KIND_ICON_SIZE))
}

fn kind_description(kind: &ConnectionKindDescriptor) -> SharedString {
    // 领域给了描述时直接用（外部驱动清单的描述）。
    if let Some(description) = kind.description() {
        return description.clone();
    }
    let kind_id = kind.id.as_ref();
    let group = kind.group;
    let description = if kind_id.starts_with(DB_KIND_PREFIX) {
        t!("NewConnection.description_database").to_string()
    } else {
        match kind_id {
            KIND_REDIS => t!("NewConnection.description_redis").to_string(),
            KIND_MONGODB => t!("NewConnection.description_mongodb").to_string(),
            KIND_SSH => t!("NewConnection.description_ssh").to_string(),
            KIND_FTP => t!("NewConnection.description_ftp").to_string(),
            KIND_SERIAL => t!("NewConnection.description_serial").to_string(),
            KIND_TELNET => t!("NewConnection.description_telnet").to_string(),
            KIND_PORT_FORWARDING => t!("NewConnection.description_port_forwarding").to_string(),
            KIND_RDP => t!("NewConnection.description_rdp").to_string(),
            KIND_VNC => t!("NewConnection.description_vnc").to_string(),
            _ => group_description(group),
        }
    };
    description.into()
}

fn group_description(group: ConnectionKindGroup) -> String {
    match group {
        ConnectionKindGroup::Database => t!("NewConnection.description_database").to_string(),
        ConnectionKindGroup::Terminal => t!("NewConnection.description_terminal").to_string(),
        ConnectionKindGroup::Remote => t!("NewConnection.description_remote").to_string(),
        ConnectionKindGroup::Extension => t!("NewConnection.description_extension").to_string(),
    }
}

impl Focusable for ConnectionFormDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ConnectionFormDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.step {
            WizardStep::SelectKind => self.render_selection_page(cx).into_any_element(),
            WizardStep::Form => self.render_form_page(cx).into_any_element(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(_cx: &App) -> SharedString {
        "默认".into()
    }

    fn descriptor(id: &str, group: ConnectionKindGroup) -> ConnectionKindDescriptor {
        ConnectionKindDescriptor::new(id, label, group, 200)
    }

    #[test]
    fn manifest_category_overrides_group_fallback() {
        let kind = descriptor("db:external:kingbase", ConnectionKindGroup::Database)
            .with_category("domestic_database");
        assert_eq!(WizardCategory::DomesticDatabase, kind_category(&kind));

        let kind = descriptor("db:external:iotdb", ConnectionKindGroup::Database);
        assert_eq!(WizardCategory::Database, kind_category(&kind));

        let unknown = descriptor("db:external:demo", ConnectionKindGroup::Database)
            .with_category("unknown_category");
        assert_eq!(WizardCategory::Database, kind_category(&unknown));
    }

    #[test]
    fn builtin_kinds_keep_legacy_categories() {
        let mysql = descriptor("db:MySQL", ConnectionKindGroup::Database);
        assert_eq!(WizardCategory::Database, kind_category(&mysql));
        let tdengine = descriptor("db:TDengine", ConnectionKindGroup::Database);
        assert_eq!(WizardCategory::TimeSeries, kind_category(&tdengine));
        let redis = descriptor(KIND_REDIS, ConnectionKindGroup::Database);
        assert_eq!(WizardCategory::NoSql, kind_category(&redis));
        let ssh = descriptor(KIND_SSH, ConnectionKindGroup::Terminal);
        assert_eq!(WizardCategory::Terminal, kind_category(&ssh));
    }

    #[test]
    fn extension_kinds_join_middleware_category() {
        let extension = descriptor("ext:com.navop.foo", ConnectionKindGroup::Extension);
        assert_eq!(WizardCategory::Middleware, kind_category(&extension));

        let declared = descriptor("ext:com.navop.foo", ConnectionKindGroup::Extension)
            .with_category("extensions");
        assert_eq!(WizardCategory::Middleware, kind_category(&declared));
    }

    #[test]
    fn manifest_metadata_wins_over_builtin_defaults() {
        let kind = descriptor("db:external:kingbase", ConnectionKindGroup::Database)
            .with_label("KingbaseES")
            .with_description("人大金仓数据库连接");
        assert_eq!("人大金仓数据库连接", kind_description(&kind).as_ref());
    }
}
