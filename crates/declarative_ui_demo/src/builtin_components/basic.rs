use gpui::{IntoElement, ParentElement, div, img};
use gpui_component::{
    Disableable, Sizable,
    button::{Button, ButtonVariants},
    group_box::{GroupBox, GroupBoxVariant, GroupBoxVariants},
    input::Input,
    label::Label,
    skeleton::Skeleton,
    tag::Tag,
};

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext, html_input_adapter::text_input_mode,
};

use super::{action_event, bool_attribute, parse_size_attribute};

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    for tag in [
        "div", "span", "section", "article", "header", "footer", "main", "nav",
    ] {
        registry.register_with_schema(tag, container_schema(), ContainerComponent)?;
    }
    registry.register_with_schema("button", button_schema(), ButtonComponent)?;
    registry.register_with_schema("input", input_schema(), InputComponent { multiline: false })?;
    registry.register_with_schema(
        "textarea",
        textarea_schema(),
        InputComponent { multiline: true },
    )?;
    registry.register_with_schema(
        "img",
        ComponentSchema::new().required_attribute("src"),
        ImageComponent,
    )?;
    registry.register_with_schema(
        "group-box",
        ComponentSchema::new()
            .attribute("title")
            .attribute("variant"),
        GroupBoxComponent,
    )?;
    registry.register_with_schema(
        "label",
        ComponentSchema::new()
            .attribute("bind")
            .attribute("secondary")
            .attribute("masked"),
        LabelComponent,
    )?;
    registry.register_with_schema(
        "tag",
        ComponentSchema::new()
            .attribute("variant")
            .attribute("outline")
            .attribute("size"),
        TagComponent,
    )?;
    registry.register_with_schema(
        "skeleton",
        ComponentSchema::new().attribute("secondary"),
        SkeletonComponent,
    )?;
    Ok(())
}

fn container_schema() -> ComponentSchema {
    ComponentSchema::new().attribute("bind")
}

fn button_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("label")
        .attribute("action")
        .attribute("variant")
        .attribute("size")
        .attribute("disabled")
        .attribute("outline")
        .attribute("loading")
        .attribute("tooltip")
        .data_attributes()
}

fn input_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("type")
        .attribute("bind")
        .attribute("placeholder")
        .attribute("value")
        .attribute("size")
        .attribute("disabled")
        .attribute("read-only")
        .attribute("cleanable")
}

fn textarea_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("bind")
        .attribute("placeholder")
        .attribute("value")
        .attribute("size")
        .attribute("disabled")
        .attribute("read-only")
        .attribute("cleanable")
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
        let label = props
            .element
            .attr("label")
            .map(str::to_owned)
            .unwrap_or_else(|| props.element.text_content());
        let mut button = Button::new(props.stable_id()).label(label);
        if let Some(variant) = props.element.attr("variant") {
            button = match variant.trim().to_ascii_lowercase().as_str() {
                "primary" => button.primary(),
                "secondary" => button.secondary(),
                "danger" => button.danger(),
                "warning" => button.warning(),
                "success" => button.success(),
                "info" => button.info(),
                "ghost" => button.ghost(),
                "link" => button.link(),
                "text" => button.text(),
                _ => {
                    return Err(ComponentError::new(format!(
                        "attribute `variant` on <button> must be one of primary, secondary, \
                         danger, warning, success, info, ghost, link, or text, got `{variant}`"
                    )));
                }
            };
        }
        if let Some(size) = parse_size_attribute(&props.element)? {
            button = button.with_size(size);
        }
        button = button
            .disabled(bool_attribute(&props.element, "disabled")?)
            .loading(bool_attribute(&props.element, "loading")?);
        if bool_attribute(&props.element, "outline")? {
            button = button.outline();
        }
        if let Some(tooltip) = props.element.attr("tooltip") {
            button = button.tooltip(tooltip.to_owned());
        }
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
        if !self.multiline {
            text_input_mode(&props.element).map_err(ComponentError::new)?;
        }
        let state = context.input_state(&props, self.multiline);
        let mut input = Input::new(&state)
            .disabled(bool_attribute(&props.element, "disabled")?)
            .read_only(bool_attribute(&props.element, "read-only")?)
            .cleanable(bool_attribute(&props.element, "cleanable")?);
        if let Some(size) = parse_size_attribute(&props.element)? {
            input = input.with_size(size);
        }
        Ok(context.style(input, &props).into_any_element())
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

struct GroupBoxComponent;

impl ComponentRenderer for GroupBoxComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut group = GroupBox::new()
            .id(props.stable_id())
            .children(context.render_children(&props));
        if let Some(title) = props.element.attr("title") {
            group = group.title(title.to_owned());
        }
        if let Some(variant) = props.element.attr("variant") {
            group = match variant.trim().to_ascii_lowercase().as_str() {
                "normal" => group.with_variant(GroupBoxVariant::Normal),
                "fill" => group.fill(),
                "outline" => group.outline(),
                _ => {
                    return Err(ComponentError::new(format!(
                        "attribute `variant` on <group-box> must be normal, fill, or outline, \
                         got `{variant}`"
                    )));
                }
            };
        }
        Ok(context.style(group, &props).into_any_element())
    }
}

struct LabelComponent;

impl ComponentRenderer for LabelComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut label = Label::new(props.element.text_content());
        if let Some(secondary) = props.element.attr("secondary") {
            label = label.secondary(secondary.to_owned());
        }
        label = label.masked(bool_attribute(&props.element, "masked")?);
        Ok(context.style(label, &props).into_any_element())
    }
}

struct TagComponent;

impl ComponentRenderer for TagComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut tag = match props
            .element
            .attr("variant")
            .unwrap_or("primary")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "primary" => Tag::primary(),
            "secondary" => Tag::secondary(),
            "danger" => Tag::danger(),
            "success" => Tag::success(),
            "warning" => Tag::warning(),
            "info" => Tag::info(),
            variant => {
                return Err(ComponentError::new(format!(
                    "attribute `variant` on <tag> must be primary, secondary, danger, success, \
                     warning, or info, got `{variant}`"
                )));
            }
        };
        if bool_attribute(&props.element, "outline")? {
            tag = tag.outline();
        }
        if let Some(size) = parse_size_attribute(&props.element)? {
            tag = tag.with_size(size);
        }
        tag = tag.children(context.render_children(&props));
        Ok(context.style(tag, &props).into_any_element())
    }
}

struct SkeletonComponent;

impl ComponentRenderer for SkeletonComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut skeleton = Skeleton::new();
        if bool_attribute(&props.element, "secondary")? {
            skeleton = skeleton.secondary();
        }
        Ok(context.style(skeleton, &props).into_any_element())
    }
}
