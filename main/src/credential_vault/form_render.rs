use gpui::{InteractiveElement, IntoElement, ParentElement, Render, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::Input,
    popover::Popover,
    scroll::ScrollableElement,
    switch::Switch,
    tab::{Tab, TabBar},
    v_flex,
};

use super::form::{CREDENTIAL_KIND_OPTIONS, CredentialForm, ordered_credential_kinds};

impl Render for CredentialForm {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let active_tab = self.active_tab;

        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .justify_center()
                    .px_4()
                    .pt_3()
                    .child(
                        TabBar::new("credential-form-tabs")
                            .with_size(Size::Small)
                            .underline()
                            .selected_index(active_tab)
                            .on_click(cx.listener(|form, index: &usize, _, cx| {
                                form.active_tab = *index;
                                cx.notify();
                            }))
                            .child(Tab::new().label("基本信息"))
                            .child(Tab::new().label("SSH 密钥"))
                            .child(Tab::new().label("同步设置")),
                    ),
            )
            .child(
                div()
                    .id("credential-form-content")
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .flex_1()
                    .overflow_hidden()
                    .child(div().size_full().p_4().overflow_y_scrollbar().child(
                        match active_tab {
                            0 => self.render_basic_tab(cx).into_any_element(),
                            1 => self.render_ssh_key_tab(cx).into_any_element(),
                            2 => self.render_sync_tab(cx).into_any_element(),
                            _ => div().into_any_element(),
                        },
                    )),
            )
    }
}

impl CredentialForm {
    fn render_basic_tab(&self, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(info_panel(
                "账号与密码",
                "这里保存的用户名和密码可以被 SSH、数据库、Redis、MongoDB、RDP/VNC 等连接引用。",
                cx,
            ))
            .child(form_field(
                "名称",
                "用于在所有连接表单中识别这条凭据。",
                Input::new(&self.name_input).w_full(),
                cx,
            ))
            .child(form_field(
                "类型",
                "支持多选，用于分类和搜索；不会限制可引用这条凭据的连接类型。",
                self.render_kind_picker(cx),
                cx,
            ))
            .child(form_field(
                "用户名",
                "可选；引用凭据时会自动填入支持用户名的连接。",
                Input::new(&self.username_input).w_full(),
                cx,
            ))
            .child(form_field(
                "密码",
                "留空表示不保存密码；编辑时清空会删除原密码。",
                Input::new(&self.password_input).w_full().mask_toggle(),
                cx,
            ))
    }

    fn render_kind_picker(&self, cx: &mut gpui::Context<Self>) -> gpui::AnyElement {
        let selected_kinds = self.selected_kinds.clone();
        let ordered_selected = ordered_credential_kinds(&selected_kinds);
        let trigger_label = match ordered_selected.as_slice() {
            [] => "请选择类型".to_string(),
            [kind] => kind.clone(),
            [first, second] => format!("{first}、{second}"),
            [first, second, ..] => {
                format!("{first}、{second} 等 {} 项", ordered_selected.len())
            }
        };
        let mut options = CREDENTIAL_KIND_OPTIONS
            .iter()
            .map(|(kind, description)| ((*kind).to_string(), (*description).to_string()))
            .collect::<Vec<_>>();
        options.extend(
            ordered_selected
                .iter()
                .filter(|kind| {
                    !CREDENTIAL_KIND_OPTIONS
                        .iter()
                        .any(|(option, _)| option == &kind.as_str())
                })
                .map(|kind| (kind.clone(), "现有凭据中的自定义类型".to_string())),
        );

        let form = cx.entity();
        Popover::new("credential-kind-picker")
            .open(self.kind_picker_open)
            .on_open_change(cx.listener(|form, open, _, cx| {
                form.kind_picker_open = *open;
                cx.notify();
            }))
            .trigger(
                Button::new("credential-kind-picker-trigger")
                    .label(trigger_label)
                    .dropdown_caret(true)
                    .w_full(),
            )
            .content(move |_, window, cx| {
                let selected_count = selected_kinds.len();
                let content_width =
                    px((window.viewport_size().width.as_f32() - 32.0).clamp(0.0, 360.0));
                v_flex()
                    .w(content_width)
                    .max_h(px(440.0))
                    .gap_2()
                    .child(
                        h_flex()
                            .w_full()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                v_flex()
                                    .gap_0p5()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child("选择适用类型"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!("已选择 {selected_count} 项")),
                                    ),
                            )
                            .child({
                                let form = form.clone();
                                Button::new("credential-kind-clear")
                                    .small()
                                    .ghost()
                                    .label("清空")
                                    .on_click(move |_, _, cx| {
                                        form.update(cx, |form, cx| {
                                            form.selected_kinds.clear();
                                            cx.notify();
                                        });
                                    })
                            }),
                    )
                    .child(div().border_t_1().border_color(cx.theme().border))
                    .child(
                        div()
                            .w_full()
                            .max_h(px(340.0))
                            .overflow_y_scrollbar()
                            .child(v_flex().w_full().gap_1().children(options.iter().map(
                                |(kind, description)| {
                                    let checked = selected_kinds.contains(kind);
                                    let kind_for_click = kind.clone();
                                    let form = form.clone();
                                    v_flex()
                                        .w_full()
                                        .gap_0p5()
                                        .rounded_md()
                                        .px_2()
                                        .py_1p5()
                                        .hover(|this| this.bg(cx.theme().muted.opacity(0.4)))
                                        .child(
                                            Checkbox::new(format!("credential-kind-option-{kind}"))
                                                .checked(checked)
                                                .label(kind.clone())
                                                .on_click(move |checked, _, cx| {
                                                    form.update(cx, |form, cx| {
                                                        if *checked {
                                                            form.selected_kinds
                                                                .insert(kind_for_click.clone());
                                                        } else {
                                                            form.selected_kinds
                                                                .remove(&kind_for_click);
                                                        }
                                                        cx.notify();
                                                    });
                                                }),
                                        )
                                        .child(
                                            div()
                                                .ml_6()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(description.clone()),
                                        )
                                },
                            ))),
                    )
            })
            .into_any_element()
    }

    fn render_ssh_key_tab(&self, cx: &gpui::App) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(info_panel(
                "SSH 私钥",
                "可以只保存本机私钥路径，也可以将 PEM/OpenSSH 私钥内容加密保存到钥匙串。",
                cx,
            ))
            .child(form_field(
                "私钥路径",
                "仅保存本机路径，不会作为密钥内容同步到其他设备。",
                Input::new(&self.private_key_path_input).w_full(),
                cx,
            ))
            .child(form_field(
                "私钥内容",
                "可粘贴 PEM/OpenSSH 私钥。内容只在打开编辑窗口后显示。",
                div().w_full().h(px(150.0)).child(
                    Input::new(&self.private_key_content_input)
                        .w_full()
                        .h_full(),
                ),
                cx,
            ))
            .child(form_field(
                "私钥密码",
                "用于解锁加密私钥，可独立被连接字段引用。",
                Input::new(&self.passphrase_input).w_full().mask_toggle(),
                cx,
            ))
    }

    fn render_sync_tab(&self, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .gap_4()
            .child(info_panel(
                "加密与同步",
                "密码、私钥内容和私钥密码只在此窗口打开期间以明文驻留内存；保存时会使用主密钥加密。",
                cx,
            ))
            .child(
                v_flex()
                    .gap_3()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .p_4()
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .gap_1()
                                    .child(
                                        div()
                                            .font_weight(gpui::FontWeight::MEDIUM)
                                            .child("允许同步"),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(
                                                "允许此凭据使用主密钥端到端加密并参与个人同步。",
                                            ),
                                    ),
                            )
                            .child(
                                Switch::new("credential-sync-enabled")
                                    .checked(self.sync_enabled)
                                    .on_click(cx.listener(|form, checked, _, cx| {
                                        form.sync_enabled = *checked;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .rounded_md()
                            .bg(cx.theme().muted.opacity(0.35))
                            .p_3()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                "启用后，密码、已导入的私钥内容和私钥密码会使用主密钥端到端加密并参与个人同步；本地私钥路径始终只保存在本机。",
                            ),
                    ),
            )
    }
}

fn info_panel(title: &'static str, description: &'static str, cx: &gpui::App) -> impl IntoElement {
    v_flex()
        .gap_1()
        .rounded_md()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted.opacity(0.35))
        .p_3()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(title),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description),
        )
}

fn form_field(
    label: &'static str,
    description: &'static str,
    input: impl IntoElement,
    cx: &gpui::App,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(label),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(description),
        )
        .child(input)
}

#[cfg(test)]
mod tests {
    #[test]
    fn credential_form_uses_grouped_tabs_with_a_bounded_scroll_region() {
        let render = include_str!("form_render.rs");
        let window = include_str!("form_window.rs");

        assert!(render.contains("TabBar::new(\"credential-form-tabs\")"));
        assert!(render.contains("Tab::new().label(\"基本信息\")"));
        assert!(render.contains("Tab::new().label(\"SSH 密钥\")"));
        assert!(render.contains("Tab::new().label(\"同步设置\")"));
        assert!(render.contains(".id(\"credential-form-content\")"));
        assert!(render.contains(".min_h_0()"));
        assert!(render.contains(".overflow_hidden()"));
        assert!(render.contains(".overflow_y_scrollbar()"));
        assert!(window.contains(".size_full()"));
        assert!(window.contains(".min_h_0()"));
        assert!(window.contains(".overflow_hidden()"));
    }

    #[test]
    fn credential_fields_are_split_across_focused_tabs() {
        let source = include_str!("form_render.rs");
        let basic = source
            .split("fn render_basic_tab")
            .nth(1)
            .and_then(|source| source.split("fn render_ssh_key_tab").next())
            .expect("basic tab");
        let ssh_key = source
            .split("fn render_ssh_key_tab")
            .nth(1)
            .and_then(|source| source.split("fn render_sync_tab").next())
            .expect("SSH key tab");
        let sync = source
            .split("fn render_sync_tab")
            .nth(1)
            .and_then(|source| source.split("fn info_panel").next())
            .expect("sync tab");

        for field in [
            "self.name_input",
            "self.render_kind_picker",
            "self.username_input",
            "self.password_input",
        ] {
            assert!(basic.contains(field));
        }
        assert!(source.contains("Popover::new(\"credential-kind-picker\")"));
        assert!(source.contains("Checkbox::new(format!("));
        assert!(source.contains(".dropdown_caret(true)"));
        assert!(source.contains("window.viewport_size().width.as_f32()"));
        assert!(source.contains(".overflow_y_scrollbar()"));
        assert!(!basic.contains("self.private_key_content_input"));

        for field in [
            "self.private_key_path_input",
            "self.private_key_content_input",
            "self.passphrase_input",
        ] {
            assert!(ssh_key.contains(field));
        }
        assert!(sync.contains("credential-sync-enabled"));
    }
}
