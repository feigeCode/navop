use gpui::{IntoElement, ParentElement, Styled, div};
use gpui_component::{Selectable, list::ListItem};

use crate::{
    ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult, ComponentSchema,
    RegistryError, RenderContext,
};

use super::{action_event, bool_attribute};

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("list", ComponentSchema::new(), ListComponent)?;
    registry.register_with_schema(
        "list-item",
        ComponentSchema::new()
            .attribute("selected")
            .attribute("secondary-selected")
            .attribute("disabled")
            .attribute("confirmed")
            .attribute("separator")
            .attribute("action")
            .data_attributes(),
        ListItemComponent,
    )?;
    Ok(())
}

struct ListComponent;

impl ComponentRenderer for ListComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let list = div()
            .flex()
            .flex_col()
            .children(context.render_children(&props));
        Ok(context.style(list, &props).into_any_element())
    }
}

struct ListItemComponent;

impl ComponentRenderer for ListItemComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let separator = bool_attribute(&props.element, "separator")?;
        let mut item = ListItem::new(props.stable_id())
            .selected(bool_attribute(&props.element, "selected")?)
            .secondary_selected(bool_attribute(&props.element, "secondary-selected")?)
            .disabled(bool_attribute(&props.element, "disabled")?)
            .confirmed(bool_attribute(&props.element, "confirmed")?)
            .children(context.render_children(&props));
        if separator {
            item = item.separator();
        }

        if let Some(action) = props.element.attr("action") {
            let event = action_event(action, &props);
            let dispatcher = context.action_dispatcher();
            item = item.on_click(move |_event, _window, cx| {
                dispatcher(event.clone(), cx);
            });
        }

        Ok(context.style(item, &props).into_any_element())
    }
}
