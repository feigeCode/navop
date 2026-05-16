use super::{LargeTextEditor, LargeTextEditorTab};
use gpui::prelude::FluentBuilder;
use gpui::{Context, IntoElement, ParentElement, Render, Styled as _, Window};
use gpui_component::button::Button;
use gpui_component::h_flex;
use gpui_component::input::Input;
use gpui_component::tab::{Tab, TabBar};
use gpui_component::v_flex;
use gpui_component::{IconName, Sizable, Size};

impl Render for LargeTextEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active_tab = self.active_tab;
        let active_index = usize::from(active_tab == LargeTextEditorTab::Json);

        v_flex()
            .size_full()
            .child(render_tab_bar(active_index, active_tab, cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(match active_tab {
                        LargeTextEditorTab::Text => Input::new(&self.text_editor).size_full(),
                        LargeTextEditorTab::Json => Input::new(&self.json_editor).size_full(),
                    }),
            )
    }
}

fn render_tab_bar(
    active_index: usize,
    active_tab: LargeTextEditorTab,
    cx: &mut Context<LargeTextEditor>,
) -> impl IntoElement {
    TabBar::new("editor-tabs")
        .with_size(Size::Small)
        .selected_index(active_index)
        .child(Tab::new().label("Text"))
        .child(Tab::new().label("JSON"))
        .on_click(cx.listener(|this, ix: &usize, window, cx| {
            let tab = if *ix == 0 {
                LargeTextEditorTab::Text
            } else {
                LargeTextEditorTab::Json
            };
            this.switch_tab(tab, window, cx);
        }))
        .suffix(
            h_flex()
                .gap_2()
                .when(active_tab == LargeTextEditorTab::Json, |this| {
                    this.child(
                        Button::new("format-json")
                            .with_size(Size::Small)
                            .label("Format")
                            .icon(IconName::Star)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.format_json(window, cx);
                            })),
                    )
                    .child(
                        Button::new("minify-json")
                            .with_size(Size::Small)
                            .label("Minify")
                            .icon(IconName::File)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.minify_json(window, cx);
                            })),
                    )
                }),
        )
}
