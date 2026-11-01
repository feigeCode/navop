//! 「导入连接」弹窗：从全量产品迁移，或导入迁移包（文件 / 粘贴 JSON）。
//!
//! 自动探测只是便利：全量产品可能装在便携目录、被换过 `--data-dir`、或干脆已
//! 卸载，因此两个页签都提供「选择文件…」原生选择器，输入框也允许手写路径
//! （`~` 与 `%VAR%` 会被展开）。

use std::path::PathBuf;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable, IntoElement, PathPromptOptions,
    Render, SharedString, Styled, Window, div, prelude::*, px,
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

use super::{
    detect_full_product, expand_source_path, import_bundle, import_from_database,
    import_from_full_product, import_from_source_file, is_sqlite_database,
};

const DIALOG_WIDTH: f32 = 760.0;
const DIALOG_HEIGHT: f32 = 560.0;
const DIALOG_REUSE_KEY: &str = "app-kit-import";
const DB_PICKER_PROMPT: &str = "选择全量产品的 one-hub.db";
const BUNDLE_PICKER_PROMPT: &str = "选择迁移包文件（.json 或 one-hub.db）";

#[derive(Clone, Copy, PartialEq, Eq)]
enum ImportTab {
    FullProduct,
    Bundle,
}

/// 打开「导入连接」弹窗；重复打开会复用同一个窗口（每次都会重新探测源库）。
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
    /// 全量产品数据库路径：探测成功时预填，也可以由用户选择或手写。
    db_input: Entity<InputState>,
    /// 用户点过「换一个文件…」（便携版/多套数据目录）后展开手动路径输入框。
    manual_db: bool,
    /// 迁移包文件路径（也接受直接选 `one-hub.db`，按文件头判定）。
    path_input: Entity<InputState>,
    json_input: Entity<TextareaState>,
    status: Option<SharedString>,
    status_ok: bool,
}

impl ImportDialog {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let detected = detect_full_product();
        let db_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("全量产品的 one-hub.db 路径，例如 ~/.config/navop/one-hub.db");
            if let Some(path) = detected.as_ref() {
                state.set_value(path.to_string_lossy().to_string(), window, cx);
            }
            state
        });
        let path_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder("迁移包 JSON 路径，也可以直接选 one-hub.db")
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
            // 始终落在「从全量产品迁移」页：即使没探测到，这一页现在也能选文件。
            tab: ImportTab::FullProduct,
            detected,
            db_input,
            manual_db: false,
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

    /// 从全量产品数据库导入：优先用输入框（探测结果已预填），清空时重新探测。
    fn import_full_product(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.db_input.read(cx).value().trim().to_string();
        if typed.is_empty() {
            // 输入框被清空 = 想重新探测一次（探测结果是打开弹窗时抓的快照）。
            return match import_from_full_product(cx) {
                Ok(report) => self.finish("已从全量产品导入", &report, window, cx),
                Err(error) => self.fail(error.to_string(), window, cx),
            };
        }
        let path = expand_source_path(&typed);
        if !is_sqlite_database(&path) {
            self.fail(
                format!(
                    "「{}」不是 SQLite 数据库；迁移包 JSON 请到「迁移包」页导入",
                    path.display()
                ),
                window,
                cx,
            );
            return;
        }
        match import_from_database(&path, cx) {
            Ok(report) => self.finish("已从全量产品导入", &report, window, cx),
            Err(error) => self.fail(error.to_string(), window, cx),
        }
    }

    /// 从文件导入：SQLite 库走全量产品迁移，其它按迁移包 JSON 解析。
    fn import_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.path_input.read(cx).value().trim().to_string();
        if typed.is_empty() {
            self.fail("请先选择或填写迁移包文件路径".to_string(), window, cx);
            return;
        }
        let path = expand_source_path(&typed);
        let sqlite_source = is_sqlite_database(&path);
        match import_from_source_file(&path, cx) {
            Ok(report) => {
                let prefix = if sqlite_source {
                    "已从全量产品数据库导入"
                } else {
                    "已从迁移包导入"
                };
                self.finish(prefix, &report, window, cx);
            }
            Err(error) => self.fail(error.to_string(), window, cx),
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

    /// 原生文件选择器；选中后把路径回填到对应输入框并提示识别结果。
    ///
    /// `reveal_db_input` 为真时（全量产品源库的「换一个文件…」）顺带把手动路径
    /// 输入框展开，让用户看到选中的是哪个文件。
    fn choose_file(
        input: Entity<InputState>,
        prompt: &'static str,
        reveal_db_input: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target_window = window.window_handle();
        let dialog = cx.entity().downgrade();
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(prompt.into()),
        });
        window
            .spawn(cx, async move |cx| {
                if let Ok(Ok(Some(paths))) = future.await
                    && let Some(path) = paths.into_iter().next()
                {
                    let value = path.to_string_lossy().to_string();
                    let note = if is_sqlite_database(&path) {
                        format!("已选择数据库：{value}")
                    } else {
                        format!("已选择文件：{value}")
                    };
                    let _ = cx.update(|_view, cx: &mut App| {
                        let _ = cx.update_window(target_window, |_, window, cx| {
                            input.update(cx, |state, cx| {
                                state.set_value(value.clone(), window, cx);
                            });
                            window.refresh();
                        });
                        let _ = dialog.update(cx, |this, cx| {
                            if reveal_db_input {
                                this.manual_db = true;
                            }
                            this.status = Some(SharedString::from(note));
                            this.status_ok = true;
                            cx.notify();
                        });
                    });
                }
                Ok::<(), anyhow::Error>(())
            })
            .detach();
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
        // 默认和全量产品自己一样：直接用自动探测到的库，不要求用户选路径。
        // 只有便携版（数据在安装目录 data/config 下）或有多套数据目录时才手选。
        let detected = self.detected.clone();
        let manual = self.manual_db || detected.is_none();
        let (hint, secondary) = match (&detected, self.manual_db) {
            (Some(path), false) => (
                format!("已自动识别全量产品数据库：{}", path.display()),
                "直接点下面的「从全量产品导入」即可。便携版（数据在安装目录 data/config 下）或有多套数据目录时，再用「换一个文件…」指定。".to_string(),
            ),
            (_, true) => (
                "已选好要迁移的源库。".to_string(),
                "便携版请选全量产品安装目录下的 data/config/one-hub.db。".to_string(),
            ),
            (None, false) => (
                "未在标准位置找到全量产品的 one-hub.db。".to_string(),
                "常规安装通常在这个位置：~/.config/navop/one-hub.db（Windows 为 %APPDATA%\\navop\\one-hub.db）；便携版请点「选择文件…」选中安装目录下的 data/config/one-hub.db。".to_string(),
            ),
        };
        v_flex()
            .gap_3()
            .child(div().text_sm().child(hint))
            .when(!manual, |el| {
                el.child(
                    Button::new("import-choose-db-another")
                        .label("换一个文件…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            Self::choose_file(
                                this.db_input.clone(),
                                DB_PICKER_PROMPT,
                                true,
                                window,
                                cx,
                            );
                        })),
                )
            })
            .when(manual, |el| {
                el.child(div().text_sm().child("全量产品数据库路径"))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.db_input)))
                            .child(
                                Button::new("import-choose-db-file")
                                    .label("选择文件…")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        Self::choose_file(
                                            this.db_input.clone(),
                                            DB_PICKER_PROMPT,
                                            true,
                                            window,
                                            cx,
                                        );
                                    })),
                            ),
                    )
            })
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
                    .child(secondary),
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
                        Button::new("import-choose-bundle-file")
                            .label("选择文件…")
                            .on_click(cx.listener(|this, _, window, cx| {
                                Self::choose_file(
                                    this.path_input.clone(),
                                    BUNDLE_PICKER_PROMPT,
                                    false,
                                    window,
                                    cx,
                                );
                            })),
                    )
                    .child(
                        Button::new("import-from-file")
                            .label("从文件导入")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.import_file(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("支持迁移包 .json，也支持直接选择全量产品的 one-hub.db（按文件头自动识别）。"),
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
