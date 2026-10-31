//! 「导入连接」弹窗：从全量产品一键迁移，或导入迁移包（文件 / 粘贴 JSON）。

use std::path::PathBuf;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Styled, Window, div, prelude::*, px,
};
use gpui_component::WindowExt as _;
use gpui_component::input::{Input, InputState, Textarea, TextareaState};
use gpui_component::{
    ActiveTheme as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    notification::Notification,
    v_flex,
};
use one_core::popup_window::{PopupWindowOptions, open_reusable_popup_window};

use super::{detect_full_product, import_bundle, import_from_full_product};

const DIALOG_WIDTH: f32 = 760.0;
const DIALOG_HEIGHT: f32 = 560.0;
const DIALOG_REUSE_KEY: &str = "app-kit-import";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportTab {
    FullProduct,
    Bundle,
}

/// 打开「导入连接」弹窗；重复打开会复用同一个窗口。
pub fn open_import_dialog(window: &mut Window, cx: &mut App) {
    open_reusable_popup_window(
        PopupWindowOptions::new("导入连接").size(DIALOG_WIDTH, DIALOG_HEIGHT),
        DIALOG_REUSE_KEY,
        |window, cx| cx.new(|cx| ImportDialog::new(window, cx)),
        Some(window),
        cx,
    );
}

/// 导入弹窗视图。
pub struct ImportDialog {
    focus_handle: FocusHandle,
    tab: ImportTab,
    detected: Option<PathBuf>,
    path_input: Entity<InputState>,
    json_input: Entity<TextareaState>,
    status: Option<SharedString>,
    status_ok: bool,
}

impl ImportDialog {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let detected = detect_full_product();
        let path_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("迁移包 JSON 文件路径，例如 ~/Desktop/navop-connections.json")
        });
        let json_input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("在这里粘贴迁移包 JSON 内容")
                .rows(10)
        });
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Self {
            focus_handle,
            tab: if detected.is_some() {
                ImportTab::FullProduct
            } else {
                ImportTab::Bundle
            },
            detected,
            path_input,
            json_input,
            status: None,
            status_ok: true,
        }
    }

    fn switch_tab(&mut self, tab: ImportTab, cx: &mut Context<Self>) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        cx.notify();
    }

    fn import_full_product(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match import_from_full_product(cx) {
            Ok(report) => self.finish("已从全量产品导入", &report, window, cx),
            Err(error) => self.fail(error.to_string(), window, cx),
        }
    }

    fn import_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.path_input.read(cx).value().trim().to_string();
        if path.is_empty() {
            self.fail("请先填写迁移包文件路径".to_string(), window, cx);
            return;
        }
        let result = std::fs::read_to_string(&path)
            .map_err(|error| format!("读取「{path}」失败：{error}"))
            .and_then(|text| import_bundle(&text, cx).map_err(|error| error.to_string()));
        match result {
            Ok(report) => self.finish("已从迁移包导入", &report, window, cx),
            Err(message) => self.fail(message, window, cx),
        }
    }

    fn import_json(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.json_input.read(cx).value().to_string();
        if text.trim().is_empty() {
            self.fail("请先粘贴迁移包 JSON 内容".to_string(), window, cx);
            return;
        }
        match import_bundle(&text, cx) {
            Ok(report) => self.finish("已从迁移包导入", &report, window, cx),
            Err(error) => self.fail(error.to_string(), window, cx),
        }
    }

    fn finish(
        &mut self,
        prefix: &str,
        report: &super::ImportReport,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message = format!("{prefix}：{}", report.summary());
        self.status = Some(SharedString::from(message.clone()));
        self.status_ok = true;
        window.push_notification(Notification::success(message), cx);
        cx.notify();
    }

    fn fail(&mut self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        self.status = Some(SharedString::from(message.clone()));
        self.status_ok = false;
        window.push_notification(Notification::error(message), cx);
        cx.notify();
    }

    fn render_tab(
        &self,
        tab: ImportTab,
        label: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.tab == tab;
        div()
            .id(SharedString::from(format!("import-tab-{label}")))
            .h(px(30.0))
            .px_3()
            .flex()
            .items_center()
            .rounded_md()
            .text_sm()
            .when(active, |tab| tab.bg(cx.theme().muted))
            .hover(|tab| tab.bg(cx.theme().muted))
            .on_click(cx.listener(move |this, _, _, cx| this.switch_tab(tab, cx)))
            .child(label)
    }

    fn render_full_product(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hint = match &self.detected {
            Some(path) => format!("已检测到全量产品数据库：{}", path.display()),
            None => "未自动检测到全量产品数据库。若已卸载全量版，可在「迁移包」页选择其 one-hub.db 路径。".to_string(),
        };
        v_flex()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(hint),
            )
            .child(
                Button::new("import-full-product")
                    .primary()
                    .label("从全量产品导入")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.import_full_product(window, cx);
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("只读读取全量产品的连接与工作区，不会修改原数据库；名称重复的连接会自动追加后缀。"),
            )
    }

    fn render_bundle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(div().text_sm().child("迁移包文件路径"))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(Input::new(&self.path_input)))
                    .child(
                        Button::new("import-from-file")
                            .label("从文件导入")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.import_file(window, cx);
                            })),
                    ),
            )
            .child(div().mt_2().text_sm().child("或粘贴迁移包 JSON"))
            .child(Textarea::new(&self.json_input).h(px(180.0)))
            .child(
                Button::new("import-from-json")
                    .label("从 JSON 导入")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.import_json(window, cx);
                    })),
            )
    }
}

impl Focusable for ImportDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ImportDialog {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .child("导入连接"),
            )
            .child(
                Button::new("import-dialog-close")
                    .small()
                    .label("关闭")
                    .on_click(cx.listener(|_, _, window, cx| {
                        let _ = one_core::window_close::close_window_for_reuse(window, cx);
                    })),
            );
        let tabs = h_flex()
            .gap_1()
            .child(self.render_tab(ImportTab::FullProduct, "从全量产品迁移", cx))
            .child(self.render_tab(ImportTab::Bundle, "迁移包", cx));
        let body = match self.tab {
            ImportTab::FullProduct => self.render_full_product(cx).into_any_element(),
            ImportTab::Bundle => self.render_bundle(cx).into_any_element(),
        };
        let status = match &self.status {
            Some(status) => div()
                .text_xs()
                .text_color(if self.status_ok {
                    cx.theme().foreground
                } else {
                    cx.theme().danger
                })
                .child(status.clone()),
            None => div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("支持从全量 Navop 直接迁移，或导入其它产品导出的迁移包。"),
        };
        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().background)
            .child(header)
            .child(v_flex().p_3().gap_3().child(tabs).child(body))
            .child(div().flex_1())
            .child(
                div()
                    .px_3()
                    .pb_3()
                    .child(status)
                    .border_t_1()
                    .border_color(cx.theme().border),
            )
    }
}
