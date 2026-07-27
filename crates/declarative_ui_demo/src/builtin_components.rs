use gpui::{IntoElement, ParentElement, div, img};
use gpui_component::{button::Button, input::Input};

use crate::{
    ActionEvent, ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer,
    ComponentResult, ComponentSchema, RegistryError, RenderContext,
};

pub(crate) fn register_default_components(
    registry: &mut ComponentRegistry,
) -> Result<(), RegistryError> {
    registry.register_with_schema("div", container_schema(), ContainerComponent)?;
    registry.register_with_schema("span", container_schema(), ContainerComponent)?;
    registry.register_with_schema("button", button_schema(), ButtonComponent)?;
    registry.register_with_schema("input", input_schema(), InputComponent { multiline: false })?;
    registry.register_with_schema(
        "textarea",
        input_schema(),
        InputComponent { multiline: true },
    )?;
    registry.register_with_schema(
        "img",
        ComponentSchema::new().required_attribute("src"),
        ImageComponent,
    )?;
    Ok(())
}

fn container_schema() -> ComponentSchema {
    ComponentSchema::new().attribute("bind")
}

fn button_schema() -> ComponentSchema {
    ComponentSchema::new().attribute("action").data_attributes()
}

fn input_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("placeholder")
        .attribute("value")
}

struct ContainerComponent;

impl ComponentRenderer for ContainerComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let children = context.render_children(&props);
        let element = div().children(children);
        Ok(context.style(element, &props).into_any_element())
    }
}

struct ButtonComponent;

impl ComponentRenderer for ButtonComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut button = Button::new(props.stable_id()).label(props.element.text_content());
        if let Some(action) = props.element.attr("action") {
            let event = action_event(action, &props);
            let dispatcher = context.action_dispatcher();
            button = button.on_click(move |_event, _window, cx| {
                dispatcher(event.clone(), cx);
            });
        }
        Ok(context.style(button, &props).into_any_element())
    }
}

struct InputComponent {
    multiline: bool,
}

impl ComponentRenderer for InputComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let state = context.input_state(&props, self.multiline);
        Ok(context.style(Input::new(&state), &props).into_any_element())
    }
}

struct ImageComponent;

impl ComponentRenderer for ImageComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let source = props
            .element
            .attr("src")
            .ok_or_else(|| ComponentError::new("<img> requires `src`"))?
            .to_owned();
        Ok(context.style(img(source), &props).into_any_element())
    }
}

fn action_event(action: &str, props: &ComponentProps) -> ActionEvent {
    let payload = props
        .element
        .attrs
        .iter()
        .filter_map(|(name, value)| {
            name.strip_prefix("data-")
                .map(|name| (name.to_owned(), value.clone()))
        })
        .collect();
    ActionEvent::new(action, props.stable_id(), props.path.clone()).with_payload(payload)
}
