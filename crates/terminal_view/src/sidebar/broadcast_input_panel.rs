use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, div, px,
};
use gpui_component::{
    Disableable as _, IndexPath, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    list::{List, ListDelegate, ListItem, ListState},
    switch::Switch,
    v_flex,
};
use rust_i18n::t;

use crate::broadcast_input::{BroadcastInputSnapshot, BroadcastTarget};
use crate::broadcast_registry::BroadcastInputRegistry;
use crate::theme::TerminalColors;

const TARGET_ROW_HEIGHT: f32 = 40.0;

pub(super) struct BroadcastInputPanel {
    registry: Entity<BroadcastInputRegistry>,
    list_state: Entity<ListState<BroadcastTargetListDelegate>>,
    colors: TerminalColors,
    _registry_subscription: Subscription,
}

pub(super) struct BroadcastInputPanelConfig {
    pub(super) registry: Entity<BroadcastInputRegistry>,
    pub(super) colors: TerminalColors,
}

impl BroadcastInputPanel {
    pub(super) fn new(
        config: BroadcastInputPanelConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let BroadcastInputPanelConfig { registry, colors } = config;
        let delegate = BroadcastTargetListDelegate::new(registry.clone(), colors.clone());
        let list_state = cx.new(|cx| ListState::new(delegate, window, cx).selectable(false));
        let observed_list = list_state.clone();
        let subscription = cx.observe(&registry, move |_, _, cx| {
            observed_list.update(cx, |_, cx| cx.notify());
            cx.notify();
        });
        Self {
            registry,
            list_state,
            colors,
            _registry_subscription: subscription,
        }
    }

    pub(super) fn set_colors(&mut self, colors: TerminalColors, cx: &mut Context<Self>) {
        self.colors = colors.clone();
        self.list_state.update(cx, |state, cx| {
            state.delegate_mut().set_colors(colors);
            cx.notify();
        });
        cx.notify();
    }

    fn render_switch(&self, snapshot: &BroadcastInputSnapshot) -> impl IntoElement {
        let registry = self.registry.clone();
        h_flex()
            .items_center()
            .justify_between()
            .child(div().text_sm().child(t!("BroadcastInput.enabled")))
            .child(
                Switch::new("broadcast-input-switch")
                    .checked(snapshot.enabled)
                    .small()
                    .on_click(move |checked: &bool, _, cx| {
                        registry.update(cx, |registry, cx| {
                            registry.set_enabled(*checked, cx);
                        });
                    }),
            )
    }

    fn render_enabled_body(&self, snapshot: &BroadcastInputSnapshot) -> impl IntoElement {
        let selected = snapshot
            .targets
            .iter()
            .filter(|target| target.selected)
            .count();
        let total = snapshot.targets.len();
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .px_3()
            .pb_3()
            .child(
                div()
                    .text_xs()
                    .text_color(self.colors.muted_foreground)
                    .child(t!(
                        "BroadcastInput.selected_count",
                        selected = selected,
                        total = total
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .border_1()
                    .border_color(self.colors.border)
                    .rounded_md()
                    .child(List::new(&self.list_state).size_full()),
            )
            .child(self.render_selection_button(snapshot))
    }

    fn render_selection_button(&self, snapshot: &BroadcastInputSnapshot) -> impl IntoElement {
        let all_selected =
            !snapshot.targets.is_empty() && snapshot.targets.iter().all(|target| target.selected);
        let registry = self.registry.clone();
        Button::new("broadcast-input-select-all")
            .ghost()
            .small()
            .disabled(snapshot.targets.is_empty())
            .label(if all_selected {
                t!("BroadcastInput.deselect_all").to_string()
            } else {
                t!("BroadcastInput.select_all").to_string()
            })
            .on_click(move |_, _, cx| {
                registry.update(cx, |registry, cx| {
                    registry.toggle_all(cx);
                });
            })
    }
}

impl Render for BroadcastInputPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let snapshot = self.registry.read(cx).snapshot();
        v_flex()
            .size_full()
            .min_h_0()
            .bg(self.colors.background)
            .child(
                v_flex()
                    .gap_2()
                    .p_3()
                    .child(self.render_switch(&snapshot))
                    .child(
                        div()
                            .text_xs()
                            .text_color(self.colors.muted_foreground)
                            .child(t!("BroadcastInput.help")),
                    ),
            )
            .when(snapshot.enabled, |this| {
                this.child(self.render_enabled_body(&snapshot))
            })
    }
}

struct BroadcastTargetListDelegate {
    registry: Entity<BroadcastInputRegistry>,
    colors: TerminalColors,
}

impl BroadcastTargetListDelegate {
    fn new(registry: Entity<BroadcastInputRegistry>, colors: TerminalColors) -> Self {
        Self { registry, colors }
    }

    fn set_colors(&mut self, colors: TerminalColors) {
        self.colors = colors;
    }

    fn target(&self, row: usize, cx: &App) -> Option<BroadcastTarget> {
        self.registry.read(cx).snapshot().targets.get(row).cloned()
    }
}

impl ListDelegate for BroadcastTargetListDelegate {
    type Item = ListItem;

    fn items_count(&self, _section: usize, cx: &App) -> usize {
        self.registry.read(cx).snapshot().targets.len()
    }

    fn render_item(
        &mut self,
        ix: IndexPath,
        _window: &mut Window,
        cx: &mut Context<ListState<Self>>,
    ) -> Option<Self::Item> {
        let target = self.target(ix.row, cx)?;
        let registry = self.registry.clone();
        let target_id = target.id;
        let selected = target.selected;
        Some(
            ListItem::new(format!("broadcast-target-row-{target_id}"))
                .h(px(TARGET_ROW_HEIGHT))
                .when(selected, |item| item.bg(self.colors.muted))
                .child(
                    h_flex()
                        .w_full()
                        .items_center()
                        .gap_2()
                        .child(
                            Checkbox::new(format!("broadcast-target-{target_id}"))
                                .checked(selected)
                                .small(),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(target.label),
                        ),
                )
                .on_click(move |_, _, cx| {
                    registry.update(cx, |registry, cx| {
                        registry.toggle_selected(target_id, cx);
                    });
                }),
        )
    }

    fn render_empty(
        &mut self,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) -> impl IntoElement {
        div()
            .size_full()
            .p_3()
            .text_sm()
            .text_color(self.colors.muted_foreground)
            .child(t!("BroadcastInput.no_targets"))
    }

    fn set_selected_index(
        &mut self,
        _ix: Option<IndexPath>,
        _window: &mut Window,
        _cx: &mut Context<ListState<Self>>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn broadcast_controls_live_in_a_shared_list_tool_panel() {
        let panel = include_str!("broadcast_input_panel.rs");
        let settings = include_str!("settings_panel.rs");

        assert!(panel.contains("Switch::new(\"broadcast-input-switch\")"));
        assert!(panel.contains("List::new(&self.list_state)"));
        assert!(panel.contains("cx.observe(&registry"));
        assert!(!settings.contains("broadcast-input-switch"));
    }
}
