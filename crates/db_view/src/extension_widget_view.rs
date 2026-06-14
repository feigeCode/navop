use std::{collections::BTreeMap, sync::Arc};

use extension_component::{SelectOption, UiAction, UiActionStyle, ViewActionEvent, ViewSpec};
use gpui::{
    App, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Window, div,
};
use gpui_component::{
    ActiveTheme, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    h_flex,
    input::InputState,
    notification::Notification,
    scroll::ScrollableElement,
    v_flex,
};

use crate::{
    extension_widget::{
        ExtensionWidgetModel, build_extension_widget_model,
        build_extension_widget_model_with_options, default_form_values,
        form_values_to_action_event,
    },
    extension_widget_view_controls::{
        ExtensionSelectState, build_input_states, build_select_states, focus_first_field,
        render_field_row,
    },
};

pub type ExtensionWidgetActionHandler =
    Arc<dyn Fn(ViewActionEvent, &mut Window, &mut App) + Send + Sync + 'static>;

pub struct ExtensionWidgetView {
    model: ExtensionWidgetModel,
    values: BTreeMap<String, String>,
    input_states: BTreeMap<String, Entity<InputState>>,
    select_states: BTreeMap<String, ExtensionSelectState>,
    action_handler: Option<ExtensionWidgetActionHandler>,
    _subscriptions: Vec<Subscription>,
}

impl ExtensionWidgetView {
    pub fn new(spec: ViewSpec) -> anyhow::Result<Self> {
        let model = build_extension_widget_model(&spec)?;
        Ok(Self::static_model(model))
    }

    pub fn new_with_options(
        window: &mut Window,
        cx: &mut Context<Self>,
        spec: ViewSpec,
        selector_options: BTreeMap<String, Vec<SelectOption>>,
    ) -> anyhow::Result<Self> {
        Self::new_with_options_and_handler(window, cx, spec, selector_options, None)
    }

    pub fn new_with_options_and_handler(
        window: &mut Window,
        cx: &mut Context<Self>,
        spec: ViewSpec,
        selector_options: BTreeMap<String, Vec<SelectOption>>,
        action_handler: Option<ExtensionWidgetActionHandler>,
    ) -> anyhow::Result<Self> {
        let model = build_extension_widget_model_with_options(&spec, selector_options)?;
        let values = default_form_values(&model);
        let (input_states, mut subscriptions) = build_input_states(&model, window, cx);
        let (select_states, select_subscriptions) = build_select_states(&model, window, cx);
        subscriptions.extend(select_subscriptions);
        focus_first_field(&model, &input_states, &select_states, window, cx);
        Ok(Self {
            model,
            values,
            input_states,
            select_states,
            action_handler,
            _subscriptions: subscriptions,
        })
    }

    pub(crate) fn value_for(&self, field_id: &str) -> Option<&String> {
        self.values.get(field_id)
    }

    pub(crate) fn set_value(&mut self, field_id: String, value: String) {
        self.values.insert(field_id, value);
    }

    pub(crate) fn remove_value(&mut self, field_id: &str) {
        self.values.remove(field_id);
    }

    pub(crate) fn input_state(&self, field_id: &str) -> Option<&Entity<InputState>> {
        self.input_states.get(field_id)
    }

    pub(crate) fn select_state(&self, field_id: &str) -> Option<&ExtensionSelectState> {
        self.select_states.get(field_id)
    }

    fn static_model(model: ExtensionWidgetModel) -> Self {
        Self {
            values: default_form_values(&model),
            model,
            input_states: BTreeMap::new(),
            select_states: BTreeMap::new(),
            action_handler: None,
            _subscriptions: Vec::new(),
        }
    }
}

impl Render for ExtensionWidgetView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut fields = v_flex().gap_3();
        for field in self.model.fields.clone() {
            fields = fields.child(render_field_row(self, field, cx));
        }

        let mut actions = h_flex().justify_end().gap_2();
        for action in &self.model.actions {
            actions = actions.child(self.render_action_button(action, cx));
        }

        let mut body = v_flex().size_full().gap_4().p_5().bg(cx.theme().background);
        body = body.child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(cx.theme().foreground)
                .child(self.model.title.clone()),
        );
        for text in &self.model.text_blocks {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(text.clone()),
            );
        }
        body.child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .child(fields),
        )
        .child(actions)
    }
}

impl ExtensionWidgetView {
    fn render_action_button(
        &self,
        action: &UiAction,
        cx: &mut Context<ExtensionWidgetView>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let view_id = self.model.id.clone();
        let action_id = action.id.clone();
        let handler = self.action_handler.clone();
        Button::new(SharedString::from(format!(
            "extension-action-{}",
            action.id
        )))
        .with_variant(action_button_variant(&action.style))
        .label(action.label.clone())
        .on_click(move |_, window, cx| {
            let event = entity.update(cx, |view, _| {
                form_values_to_action_event(&view_id, &action_id, &view.values)
            });
            match &handler {
                Some(handler) => handler(event, window, cx),
                None => push_placeholder_notification(event, window, cx),
            }
        })
    }
}

fn action_button_variant(style: &UiActionStyle) -> ButtonVariant {
    match style {
        UiActionStyle::Primary => ButtonVariant::Primary,
        UiActionStyle::Secondary => ButtonVariant::Secondary,
        UiActionStyle::Danger => ButtonVariant::Danger,
    }
}

pub(crate) fn push_placeholder_notification(
    event: ViewActionEvent,
    window: &mut Window,
    cx: &mut App,
) {
    let summary = if event.fields.is_empty() {
        "{}".to_string()
    } else {
        event
            .fields
            .iter()
            .map(|field| format!("{}={}", field.id, field.value))
            .collect::<Vec<_>>()
            .join(", ")
    };
    window.push_notification(
        Notification::info(format!("{}: {summary}", event.action_id)).autohide(true),
        cx,
    );
}
