//! 独立产品的连接表单宿主（新建 / 编辑 / 复制）。
//!
//! 弹窗左栏列出当前产品已装配的连接种类（由领域通过
//! [`domain_api::ConnectionFormProvider`] 注册），右栏渲染对应表单。
//! 表单保存后自身写仓库并广播 `ConnectionDataEvent`，宿主列表据此刷新。

use domain_api::{ConnectionFormRequest, ConnectionKindDescriptor, ConnectionKindGroup};
use gpui::{
    AnyView, App, AppContext as _, Context, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::WindowExt as _;
use gpui_component::{
    ActiveTheme as _, Sizable as _, button::Button, h_flex, notification::Notification, v_flex,
};
use one_core::popup_window::{PopupWindowOptions, open_reusable_popup_window};
use one_core::storage::StoredConnection;

const DIALOG_WIDTH: f32 = 980.0;
const DIALOG_HEIGHT: f32 = 660.0;
const KIND_ROW_HEIGHT: f32 = 28.0;
const DIALOG_REUSE_KEY: &str = "app-kit-connection-form";

/// 表单用途：决定标题与请求构造方式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnectionFormMode {
    New,
    Edit,
    Duplicate,
}

impl ConnectionFormMode {
    fn title(self) -> &'static str {
        match self {
            Self::New => "新建连接",
            Self::Edit => "编辑连接",
            Self::Duplicate => "复制连接",
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

/// 表单弹窗视图。
pub struct ConnectionFormDialog {
    focus_handle: FocusHandle,
    kinds: Vec<ConnectionKindDescriptor>,
    selected_kind: Option<SharedString>,
    form: Option<AnyView>,
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
        // 新建时默认选中第一个种类，避免右栏空着让用户以为没有可用表单。
        let selected_kind = preset
            .as_ref()
            .map(|(kind_id, _)| kind_id.clone())
            .or_else(|| kinds.first().map(|kind| kind.id.clone()));
        let form = selected_kind.as_ref().and_then(|kind_id| {
            build_form(
                kind_id.clone(),
                mode,
                preset.as_ref().map(|(_, connection)| connection.clone()),
                workspace_id,
                window,
                cx,
            )
        });
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            focus_handle,
            kinds,
            selected_kind,
            form,
            mode,
            preset,
            workspace_id,
        }
    }

    fn select_kind(&mut self, kind_id: SharedString, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_kind.as_ref() == Some(&kind_id) {
            return;
        }
        let connection = self.preset.as_ref().map(|(_, conn)| conn.clone());
        self.form = build_form(
            kind_id.clone(),
            self.mode,
            connection,
            self.workspace_id,
            window,
            cx,
        );
        self.selected_kind = Some(kind_id);
        cx.notify();
    }

    fn render_kind_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = v_flex()
            .id("connection-kind-list")
            .w(px(200.0))
            .h_full()
            .p_2()
            .gap_0p5()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(cx.theme().border);
        let mut last_group: Option<ConnectionKindGroup> = None;
        for kind in self.kinds.clone() {
            if last_group != Some(kind.group) {
                list = list.child(
                    div()
                        .mt_2()
                        .mb_1()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(group_title(kind.group)),
                );
                last_group = Some(kind.group);
            }
            let selected = self.selected_kind.as_ref() == Some(&kind.id);
            let id = kind.id.clone();
            list = list.child(
                div()
                    .id(SharedString::from(format!("connection-kind-{}", kind.id)))
                    .h(px(KIND_ROW_HEIGHT))
                    .px_2()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .text_sm()
                    .when(selected, |row| row.bg(cx.theme().muted))
                    .hover(|row| row.bg(cx.theme().muted))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.select_kind(id.clone(), window, cx);
                    }))
                    .child(kind.display_label(cx)),
            );
        }
        list
    }
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

fn group_title(group: ConnectionKindGroup) -> &'static str {
    match group {
        ConnectionKindGroup::Database => "数据库",
        ConnectionKindGroup::Terminal => "终端与文件",
        ConnectionKindGroup::Remote => "远程桌面与转发",
        ConnectionKindGroup::Extension => "扩展",
    }
}

impl Focusable for ConnectionFormDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ConnectionFormDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.form.clone() {
            Some(form) => div().flex_1().min_h_0().min_w_0().child(form),
            None => v_flex()
                .flex_1()
                .items_center()
                .justify_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("当前产品没有可新建的连接类型"),
        };
        let header = h_flex()
            .h(px(44.0))
            .px_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(self.mode.title()),
            )
            .child(
                Button::new("connection-form-close")
                    .small()
                    .label("关闭")
                    .on_click(cx.listener(|_, _, window, cx| {
                        let _ = one_core::window_close::close_window_for_reuse(window, cx);
                    })),
            );
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_kind_list(cx))
                    .child(content),
            )
    }
}
