//! 独立产品的连接表单宿主（新建 / 编辑 / 复制）。
//!
//! 「新建连接」是两步向导，交互沿用产品分离前的旧版样式：
//! 第 1 步整页选择连接类型（左侧类目 + 右侧卡片网格），第 2 步渲染
//! 对应表单，表单左下角提供「上一步」返回类型选择。
//! 类型清单由领域通过 [`domain_api::ConnectionFormProvider`] 注册，因此
//! 多产品构建（全量 / Data / Shell）各自只展示已装配的类型，不写死任何一种。
//! 表单保存后自身写仓库并广播 `ConnectionDataEvent`，宿主列表据此刷新。

use domain_api::{ConnectionFormRequest, ConnectionKindDescriptor, ConnectionKindGroup};
use gpui::{
    AnyView, App, AppContext as _, Context, FocusHandle, Focusable, FontWeight, IntoElement,
    KeyBinding, Render, SharedString, Styled, Window, div, prelude::*, px,
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
const KIND_CARD_WIDTH: f32 = 280.0;
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
    open_reusable_popup_window(
        PopupWindowOptions::new(mode.title()).size(DIALOG_WIDTH, DIALOG_HEIGHT),
        DIALOG_REUSE_KEY,
        move |window, cx| {
            cx.new(|cx| ConnectionFormDialog::new(mode, preset.clone(), workspace_id, window, cx))
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum KindCategory {
    All,
    Group(ConnectionKindGroup),
}

impl KindCategory {
    fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Group(ConnectionKindGroup::Database) => "database",
            Self::Group(ConnectionKindGroup::Terminal) => "terminal",
            Self::Group(ConnectionKindGroup::Remote) => "remote",
            Self::Group(ConnectionKindGroup::Extension) => "extension",
        }
    }

    fn matches(self, group: ConnectionKindGroup) -> bool {
        match self {
            Self::All => true,
            Self::Group(selected) => selected == group,
        }
    }
}

/// 表单弹窗视图。
pub struct ConnectionFormDialog {
    focus_handle: FocusHandle,
    kinds: Vec<ConnectionKindDescriptor>,
    step: WizardStep,
    category: KindCategory,
    selected_kind: Option<SharedString>,
    /// 第 2 步的表单视图；与 `form_kind` 一起缓存，返回上一步时不丢已填内容。
    form: Option<AnyView>,
    form_kind: Option<SharedString>,
    mode: ConnectionFormMode,
    preset: Option<(SharedString, StoredConnection)>,
    workspace_id: Option<i64>,
}

impl ConnectionFormDialog {
    fn new(
        mode: ConnectionFormMode,
        preset: Option<(SharedString, StoredConnection)>,
        workspace_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kinds = crate::registry().kinds(cx);
        let mut dialog = Self {
            focus_handle: cx.focus_handle(),
            kinds,
            step: WizardStep::SelectKind,
            category: KindCategory::All,
            selected_kind: None,
            form: None,
            form_kind: None,
            mode,
            preset,
            workspace_id,
        };
        if dialog.mode == ConnectionFormMode::New {
            // 与旧版一致：默认选中第一个种类，让「下一步」立即可用。
            dialog.selected_kind = dialog.kinds.first().map(|kind| kind.id.clone());
        } else if let Some((kind_id, _)) = dialog.preset.clone() {
            // 编辑 / 复制跳过类型选择，直接进表单。
            dialog.selected_kind = Some(kind_id.clone());
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

    /// 左侧类目：全部 + 当前产品实际装配了类型的分组。
    fn categories(&self) -> Vec<KindCategory> {
        let mut categories = vec![KindCategory::All];
        for group in [
            ConnectionKindGroup::Database,
            ConnectionKindGroup::Terminal,
            ConnectionKindGroup::Remote,
            ConnectionKindGroup::Extension,
        ] {
            if self.kinds.iter().any(|kind| kind.group == group) {
                categories.push(KindCategory::Group(group));
            }
        }
        categories
    }

    fn kinds_in(&self, category: KindCategory) -> Vec<ConnectionKindDescriptor> {
        let mut kinds: Vec<_> = self
            .kinds
            .iter()
            .filter(|kind| category.matches(kind.group))
            .cloned()
            .collect();
        kinds.sort_by_key(|kind| (kind.group.order(), kind.order));
        kinds
    }

    fn select_category(&mut self, category: KindCategory, cx: &mut Context<Self>) {
        if self.category == category {
            return;
        }
        self.category = category;
        let kinds = self.kinds_in(category);
        if !kinds
            .iter()
            .any(|kind| Some(&kind.id) == self.selected_kind.as_ref())
        {
            self.selected_kind = kinds.first().map(|kind| kind.id.clone());
        }
        cx.notify();
    }

    fn select_kind(&mut self, kind_id: SharedString, cx: &mut Context<Self>) {
        if self.selected_kind.as_ref() == Some(&kind_id) {
            return;
        }
        self.selected_kind = Some(kind_id);
        cx.notify();
    }

    /// 键盘上下移动选中项（在「全部」类目里跨分组移动，循环）。
    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.step != WizardStep::SelectKind {
            return;
        }
        let kinds = self.kinds_in(self.category);
        if kinds.is_empty() {
            return;
        }
        let current = self
            .selected_kind
            .as_ref()
            .and_then(|selected| kinds.iter().position(|kind| &kind.id == selected));
        let next = match current {
            Some(index) => (index as isize + delta).rem_euclid(kinds.len() as isize) as usize,
            None if delta < 0 => kinds.len() - 1,
            None => 0,
        };
        self.selected_kind = Some(kinds[next].id.clone());
        cx.notify();
    }

    fn open_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind_id) = self.selected_kind.clone() else {
            return;
        };
        self.open_kind(kind_id, window, cx);
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
        self.selected_kind = Some(kind_id);
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

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .h(px(44.0))
            .px_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(self.mode.title()),
                    )
                    .when(self.is_wizard(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(step_indicator(self.step)),
                        )
                    }),
            )
            .child(
                Button::new("connection-form-close")
                    .small()
                    .label(t!("NewConnection.close").to_string())
                    .on_click(cx.listener(|_, _, window, cx| {
                        let _ = one_core::window_close::close_window_for_reuse(window, cx);
                    })),
            )
    }

    /// 第 1 步：左侧类目 + 右侧卡片网格 + 底部「取消 / 下一步」。
    fn render_selection_page(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .key_context(WIZARD_KEY_CONTEXT)
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_open_selected))
            .bg(cx.theme().background)
            .child(self.render_header(cx))
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .child(self.render_category_sidebar(cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(self.render_kind_grid(cx))
                            .child(self.render_selection_footer(cx)),
                    ),
            )
    }

    fn render_category_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = v_flex()
            .id("connection-kind-categories")
            .w(px(CATEGORY_SIDEBAR_WIDTH))
            .h_full()
            .min_h_0()
            .p_2()
            .gap_0p5()
            .bg(cx.theme().sidebar)
            .border_r_1()
            .border_color(cx.theme().border)
            .overflow_y_scrollbar();
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
                    .gap_2()
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

    fn render_kind_grid(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let kinds = self.kinds_in(self.category);
        let content = v_flex()
            .flex_1()
            .min_h_0()
            .min_w_0()
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
        if kinds.is_empty() {
            return content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("NewConnection.empty").to_string()),
            );
        }
        let mut grid = div().flex().flex_wrap().w_full().gap_3();
        for kind in &kinds {
            grid = grid.child(
                div()
                    .w(px(KIND_CARD_WIDTH))
                    .flex_shrink_0()
                    .child(self.render_kind_card(kind, cx)),
            );
        }
        content.child(grid)
    }

    fn render_kind_card(
        &self,
        kind: &ConnectionKindDescriptor,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.selected_kind.as_ref() == Some(&kind.id);
        let click_id = kind.id.clone();
        let double_click_id = kind.id.clone();
        let label = kind.display_label(cx);
        let description = kind_description(&kind.id, kind.group);
        let icon = kind_icon(&kind.id, kind.group);
        let theme = cx.theme();
        v_flex()
            .id(SharedString::from(format!(
                "connection-kind-card-{}",
                kind.id
            )))
            .justify_center()
            .w_full()
            .h(px(KIND_CARD_HEIGHT))
            .p_3()
            .rounded_lg()
            .border_1()
            .bg(theme.background)
            .border_color(if selected {
                theme.list_active_border
            } else {
                theme.border
            })
            .when(selected, |card| card.shadow_sm())
            .hover(|card| card.shadow_sm().border_color(theme.list_active_border))
            .cursor_pointer()
            .overflow_hidden()
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_kind(click_id.clone(), cx);
            }))
            .on_double_click(cx.listener(move |this, _, window, cx| {
                this.open_kind(double_click_id.clone(), window, cx);
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
                    .disabled(self.selected_kind.is_none())
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
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .child(self.render_header(cx))
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

fn step_indicator(step: WizardStep) -> String {
    let current = match step {
        WizardStep::SelectKind => 1,
        WizardStep::Form => 2,
    };
    t!("NewConnection.step_indicator", current = current).to_string()
}

fn category_title(category: KindCategory) -> SharedString {
    let title = match category {
        KindCategory::All => t!("NewConnection.category_all").to_string(),
        KindCategory::Group(ConnectionKindGroup::Database) => {
            t!("NewConnection.category_database").to_string()
        }
        KindCategory::Group(ConnectionKindGroup::Terminal) => {
            t!("NewConnection.category_terminal").to_string()
        }
        KindCategory::Group(ConnectionKindGroup::Remote) => {
            t!("NewConnection.category_remote").to_string()
        }
        KindCategory::Group(ConnectionKindGroup::Extension) => {
            t!("NewConnection.category_extension").to_string()
        }
    };
    title.into()
}

fn category_icon(category: KindCategory) -> IconName {
    match category {
        KindCategory::All => IconName::LayoutDashboard,
        KindCategory::Group(ConnectionKindGroup::Database) => IconName::DatabaseLine,
        KindCategory::Group(ConnectionKindGroup::Terminal) => IconName::Terminal,
        KindCategory::Group(ConnectionKindGroup::Remote) => IconName::Monitor,
        KindCategory::Group(ConnectionKindGroup::Extension) => IconName::ExtensionsLine,
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

fn kind_icon(kind_id: &str, group: ConnectionKindGroup) -> Icon {
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

fn kind_description(kind_id: &str, group: ConnectionKindGroup) -> SharedString {
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
