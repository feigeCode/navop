//! 凭据库弹窗的渲染（列表 / 空态 / 加载失败 / 行操作）。
//!
//! 从 `credentials` 拆出来，让流程与渲染各自守住行数上限。

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Window,
    div, px,
};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::input::Input;
use gpui_component::{ActiveTheme as _, Sizable as _, h_flex, scroll::ScrollableElement, v_flex};
use one_core::crypto;
use one_core::storage::CredentialSummary;

use crate::vault::credentials::CredentialVaultView;

impl Render for CredentialVaultView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let unlocked = crypto::has_master_key();
        let body = self.render_body(unlocked, cx);
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .child(self.render_header(unlocked, cx))
            .child(render_hint(unlocked, cx))
            .child(body)
    }
}

impl CredentialVaultView {
    /// 主体：加载失败 / 空态 / 列表三选一
    fn render_body(&self, unlocked: bool, cx: &mut Context<Self>) -> gpui::AnyElement {
        if let Some(error) = self.load_error.clone() {
            return render_load_failed(error, cx);
        }
        let summaries = self.filtered(cx);
        if summaries.is_empty() {
            return render_empty(self.summaries.len(), cx);
        }
        v_flex()
            .flex_1()
            .min_h_0()
            .overflow_y_scrollbar()
            .children(
                summaries
                    .into_iter()
                    .map(|summary| self.render_row(summary, unlocked, cx).into_any_element()),
            )
            .into_any_element()
    }

    /// 顶部：标题 + 锁定状态 + 新建 / 刷新 / 搜索 / 关闭
    fn render_header(&self, unlocked: bool, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w_full()
            .flex_shrink_0()
            .justify_between()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(header_left(unlocked, cx))
            .child(
                div().min_w(px(220.)).max_w(px(360.)).child(
                    Input::new(&self.search_input)
                        .cleanable(true)
                        .small()
                        .w_full(),
                ),
            )
            .child(
                Button::new("credential-vault-close")
                    .small()
                    .label("关闭")
                    .on_click(cx.listener(|_, _, window, cx| {
                        let _ = one_core::window_close::close_window_for_reuse(window, cx);
                    })),
            )
    }

    /// 一行凭据：左侧摘要 + 右侧编辑 / 删除
    fn render_row(
        &self,
        summary: CredentialSummary,
        unlocked: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = summary.id;
        let name = summary.name.clone();
        h_flex()
            .w_full()
            .gap_3()
            .items_center()
            .px_4()
            .py_2()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(render_row_summary(&summary, unlocked, cx))
            .child(render_row_actions(id, name, cx))
    }
}

/// 锁定状态色：未解锁时用警示色提示"需要解锁"
fn lock_color(unlocked: bool, cx: &gpui::App) -> gpui::Hsla {
    if unlocked {
        cx.theme().success
    } else {
        cx.theme().warning
    }
}

fn render_hint(unlocked: bool, cx: &gpui::App) -> impl IntoElement {
    div().flex_shrink_0().px_4().pt_2().child(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(if unlocked {
                "敏感字段使用主密钥加密保存，仅在打开单条凭据进行编辑时解密。"
            } else {
                "摘要仍可查看；编辑包含密码或私钥的凭据前，需要先解锁主密钥。"
            }),
    )
}

fn render_load_failed(error: String, cx: &mut Context<CredentialVaultView>) -> gpui::AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().danger)
                .child(format!("无法加载钥匙串：{error}")),
        )
        .child(
            Button::new("credential-vault-retry")
                .small()
                .label("重试")
                .on_click(cx.listener(|view, _, _, cx| view.reload(cx))),
        )
        .into_any_element()
}

fn render_empty(total: usize, cx: &gpui::App) -> gpui::AnyElement {
    let (title, hint) = if total == 0 {
        (
            "还没有凭据",
            "保存可供连接复用的用户名、密码、私钥和终端自动登录规则。",
        )
    } else {
        (
            "没有匹配的凭据",
            "请尝试使用其他名称、类型或用户名进行搜索。",
        )
    };
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            div()
                .text_base()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(hint),
        )
        .into_any_element()
}

/// 行左侧：名称、用户名 / 同步状态、已保存的敏感字段
fn render_row_summary(
    summary: &CredentialSummary,
    unlocked: bool,
    cx: &gpui::App,
) -> impl IntoElement {
    let (meta, fields) = row_meta(summary);
    v_flex()
        .flex_1()
        .min_w_0()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(summary.name.clone()),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(meta),
        )
        .child(
            div()
                .text_xs()
                .text_color(lock_color(unlocked, cx))
                .child(fields),
        )
}

/// 行右侧：编辑 / 删除
fn render_row_actions(
    id: i64,
    name: String,
    cx: &mut Context<CredentialVaultView>,
) -> impl IntoElement {
    h_flex()
        .gap_1()
        .child(
            Button::new(format!("credential-edit-{id}"))
                .small()
                .ghost()
                .label("编辑")
                .on_click(cx.listener(move |view, _, window, cx| view.open_edit(id, window, cx))),
        )
        .child(
            Button::new(format!("credential-delete-{id}"))
                .small()
                .ghost()
                .label("删除")
                .on_click(cx.listener(move |view, _, window, cx| {
                    view.confirm_delete(id, name.clone(), window, cx)
                })),
        )
}

/// 顶部左侧：标题 + 锁定状态 + 新建 / 刷新
fn header_left(unlocked: bool, cx: &mut Context<CredentialVaultView>) -> impl IntoElement {
    h_flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .text_base()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child("钥匙串"),
        )
        .child(
            div()
                .text_xs()
                .text_color(lock_color(unlocked, cx))
                .child(if unlocked { "已解锁" } else { "已锁定" }),
        )
        .child(
            Button::new("credential-vault-add")
                .small()
                .primary()
                .label("新建凭据")
                .on_click(cx.listener(|view, _, window, cx| view.open_create(window, cx))),
        )
        .child(
            Button::new("credential-vault-refresh")
                .small()
                .ghost()
                .label("刷新")
                .on_click(cx.listener(|view, _, _, cx| view.reload(cx))),
        )
}

/// 行的副标题（用户名 · 同步状态）与已保存敏感字段清单
fn row_meta(summary: &CredentialSummary) -> (String, String) {
    let mut capabilities = Vec::new();
    if summary.has_password {
        capabilities.push("密码");
    }
    if summary.has_private_key_path || summary.has_private_key_content {
        capabilities.push("私钥");
    }
    if summary.has_passphrase {
        capabilities.push("私钥密码");
    }
    let meta = format!(
        "{}  ·  {}",
        summary.username.as_deref().unwrap_or("未设置用户名"),
        if summary.sync_enabled {
            "允许同步"
        } else {
            "仅本地"
        }
    );
    let fields = if capabilities.is_empty() {
        "无敏感字段".to_string()
    } else {
        capabilities.join(" · ")
    };
    (meta, fields)
}
