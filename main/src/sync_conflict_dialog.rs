use std::{collections::HashMap, rc::Rc};

use gpui::{
    AnyElement, App, AppContext, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::button::ButtonVariant;
use gpui_component::{
    Sizable, WindowExt,
    button::{Button, ButtonVariants as _},
    h_flex, v_flex,
};
use one_core::cloud_sync::ConflictResolution;

#[derive(Clone)]
pub(crate) struct SyncConflictResolutionOption {
    pub strategy: ConflictResolution,
    pub label: SharedString,
}

pub(crate) struct SyncConflictDialogItem {
    pub id: String,
    pub title: String,
    pub detail: String,
    pub default_strategy: ConflictResolution,
    pub options: Vec<SyncConflictResolutionOption>,
}

pub(crate) fn show_sync_conflict_dialog(
    window: &mut Window,
    cx: &mut App,
    title: String,
    ok_text: String,
    items: Vec<SyncConflictDialogItem>,
    on_apply: impl Fn(Vec<(String, ConflictResolution)>, &mut Window, &mut App) + 'static,
) {
    if items.is_empty() {
        return;
    }

    let strategies = cx.new(|_| default_strategy_map(&items));
    let on_apply = Rc::new(on_apply);
    window.open_dialog(cx, move |dialog, _window, cx| {
        let conflict_items = render_conflict_items(&items, strategies.clone(), cx);
        let strategies_for_ok = strategies.clone();
        let on_apply = on_apply.clone();

        dialog
            .title(title.clone().into_any_element())
            .child(
                v_flex()
                    .id("sync_conflict_items")
                    .gap_3()
                    .max_h(px(400.0))
                    .overflow_y_scroll()
                    .children(conflict_items)
                    .into_any_element(),
            )
            .confirm()
            .button_props(
                gpui_component::dialog::DialogButtonProps::default().ok_text(ok_text.clone()),
            )
            .on_ok(move |_event, window, cx| {
                let selected = strategies_for_ok.read(cx).clone().into_iter().collect();
                on_apply(selected, window, cx);
                window.refresh();
                true
            })
    });
}

fn default_strategy_map(items: &[SyncConflictDialogItem]) -> HashMap<String, ConflictResolution> {
    items
        .iter()
        .map(|item| (item.id.clone(), item.default_strategy))
        .collect()
}

fn render_conflict_items(
    items: &[SyncConflictDialogItem],
    strategies: gpui::Entity<HashMap<String, ConflictResolution>>,
    cx: &mut App,
) -> Vec<AnyElement> {
    items
        .iter()
        .map(|item| render_conflict_item(item, strategies.clone(), cx))
        .collect()
}

fn render_conflict_item(
    item: &SyncConflictDialogItem,
    strategies: gpui::Entity<HashMap<String, ConflictResolution>>,
    cx: &mut App,
) -> AnyElement {
    let current = strategies
        .read(cx)
        .get(&item.id)
        .copied()
        .unwrap_or(item.default_strategy);

    v_flex()
        .gap_2()
        .p_3()
        .bg(gpui::hsla(0.0, 0.0, 0.5, 0.1))
        .rounded_md()
        .child(conflict_title(item))
        .child(conflict_detail(item))
        .child(strategy_buttons(item, current, strategies))
        .into_any_element()
}

fn conflict_title(item: &SyncConflictDialogItem) -> AnyElement {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(item.title.clone())
        .into_any_element()
}

fn conflict_detail(item: &SyncConflictDialogItem) -> AnyElement {
    div()
        .text_xs()
        .text_color(gpui::hsla(0.0, 0.0, 0.5, 1.0))
        .child(item.detail.clone())
        .into_any_element()
}

fn strategy_buttons(
    item: &SyncConflictDialogItem,
    current: ConflictResolution,
    strategies: gpui::Entity<HashMap<String, ConflictResolution>>,
) -> AnyElement {
    h_flex()
        .gap_2()
        .mt_2()
        .children(
            item.options.iter().map(|option| {
                strategy_button(&item.id, option.clone(), current, strategies.clone())
            }),
        )
        .into_any_element()
}

fn strategy_button(
    item_id: &str,
    option: SyncConflictResolutionOption,
    current: ConflictResolution,
    strategies: gpui::Entity<HashMap<String, ConflictResolution>>,
) -> Button {
    let button_id = format!("sync_conflict_{:?}_{}", option.strategy, item_id);
    Button::new(ElementId::Name(SharedString::from(button_id)))
        .label(option.label)
        .with_variant(if current == option.strategy {
            ButtonVariant::Primary
        } else {
            ButtonVariant::Ghost
        })
        .xsmall()
        .on_click({
            let item_id = item_id.to_string();
            let strategy = option.strategy;
            move |_, _, cx| {
                strategies.update(cx, |selected, cx| {
                    selected.insert(item_id.clone(), strategy);
                    cx.notify();
                });
            }
        })
}
