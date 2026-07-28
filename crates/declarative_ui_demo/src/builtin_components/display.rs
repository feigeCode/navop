use gpui::{IntoElement, ParentElement, div, px};
use gpui_component::{
    Sizable,
    avatar::{Avatar, AvatarGroup},
    description_list::{DescriptionItem, DescriptionList},
};

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext, VNode,
};

use super::{
    bool_attribute_or, parse_non_negative_f32, parse_positive_usize_attribute, parse_size_attribute,
};

const DEFAULT_DESCRIPTION_COLUMNS: usize = 3;
const MAX_DESCRIPTION_COLUMNS: usize = 10;

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema("avatar", avatar_schema(), AvatarComponent)?;
    registry.register_with_schema("avatar-group", avatar_group_schema(), AvatarGroupComponent)?;
    registry.register_with_schema(
        "description-list",
        description_list_schema(),
        DescriptionListComponent,
    )?;
    registry.register_with_schema(
        "description-item",
        description_item_schema(),
        StructuralDisplayComponent,
    )?;
    Ok(())
}

fn avatar_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("name")
        .attribute("src")
        .attribute("size")
}

fn avatar_group_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("limit")
        .attribute("ellipsis")
        .attribute("size")
}

fn description_list_schema() -> ComponentSchema {
    ComponentSchema::new()
        .attribute("layout")
        .attribute("label-width")
        .attribute("bordered")
        .attribute("columns")
        .attribute("size")
}

fn description_item_schema() -> ComponentSchema {
    ComponentSchema::new()
        .required_attribute("label")
        .attribute("span")
}

struct AvatarComponent;

impl ComponentRenderer for AvatarComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        Ok(build_avatar(props, context, false)?.into_any_element())
    }
}

fn build_avatar(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
    inside_group: bool,
) -> Result<Avatar, ComponentError> {
    ensure_no_children(&props)?;
    if inside_group && props.element.attr("size").is_some() {
        return Err(ComponentError::new(
            "<avatar> inside <avatar-group> must inherit `size` from the group",
        ));
    }

    let mut avatar = Avatar::new();
    if let Some(name) = props.element.attr("name") {
        avatar = avatar.name(name.to_owned());
    }
    if let Some(source) = props.element.attr("src") {
        avatar = avatar.src(source.to_owned());
    }
    if let Some(size) = parse_size_attribute(&props.element)? {
        avatar = avatar.with_size(size);
    }
    Ok(context.style(avatar, &props))
}

struct AvatarGroupComponent;

impl ComponentRenderer for AvatarGroupComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let avatars = props
            .element
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let child_props = structural_child_props(&props, index, child, "avatar")?;
                build_avatar(child_props, context, true)
            })
            .collect::<Result<Vec<_>, ComponentError>>()?;

        let mut group = AvatarGroup::new().children(avatars);
        if let Some(limit) = parse_positive_usize_attribute(&props.element, "limit")? {
            group = group.limit(limit);
        }
        if bool_attribute_or(&props.element, "ellipsis", false)? {
            group = group.ellipsis();
        }
        if let Some(size) = parse_size_attribute(&props.element)? {
            group = group.with_size(size);
        }
        Ok(context.style(group, &props).into_any_element())
    }
}

struct DescriptionListComponent;

impl ComponentRenderer for DescriptionListComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut list = match props.element.attr("layout").unwrap_or("horizontal") {
            value if value.eq_ignore_ascii_case("horizontal") => DescriptionList::horizontal(),
            value if value.eq_ignore_ascii_case("vertical") => DescriptionList::vertical(),
            value => {
                return Err(ComponentError::new(format!(
                    "attribute `layout` on <description-list> must be horizontal or vertical, \
                     got `{value}`"
                )));
            }
        };

        let columns = parse_positive_usize_attribute(&props.element, "columns")?
            .unwrap_or(DEFAULT_DESCRIPTION_COLUMNS);
        if columns > MAX_DESCRIPTION_COLUMNS {
            return Err(ComponentError::new(format!(
                "attribute `columns` on <description-list> must be between 1 and \
                 {MAX_DESCRIPTION_COLUMNS}, got `{columns}`"
            )));
        }
        list = list
            .columns(columns)
            .bordered(bool_attribute_or(&props.element, "bordered", true)?);

        if let Some(width) = props.element.attr("label-width") {
            let width = parse_non_negative_f32(&props.element, "label-width", width)?;
            list = list.label_width(px(width));
        }
        if let Some(size) = parse_size_attribute(&props.element)? {
            list = list.with_size(size);
        }

        let items = props
            .element
            .children
            .iter()
            .enumerate()
            .map(|(index, child)| {
                let child_props = structural_child_props(&props, index, child, "description-item")?;
                build_description_item(child_props, columns, context)
            })
            .collect::<Result<Vec<_>, ComponentError>>()?;
        list = list.children(items);

        // DescriptionList does not expose Styled. Apply list classes to a
        // stable wrapper while retaining its native item/grid rendering.
        Ok(context.style(div().child(list), &props).into_any_element())
    }
}

fn build_description_item(
    props: ComponentProps,
    columns: usize,
    context: &mut RenderContext<'_>,
) -> Result<DescriptionItem, ComponentError> {
    let label = props
        .element
        .attr("label")
        .ok_or_else(|| ComponentError::new("<description-item> requires `label`"))?
        .to_owned();
    let span = parse_positive_usize_attribute(&props.element, "span")?.unwrap_or(1);
    if span > columns {
        return Err(ComponentError::new(format!(
            "attribute `span` on <description-item> must not exceed the parent column count \
             ({columns}), got `{span}`"
        )));
    }

    let children = context.render_children(&props);
    // DescriptionItem is an enum without Styled. Item classes therefore
    // refine the value wrapper, which is the only arbitrary element slot.
    let value = context
        .style(div().children(children), &props)
        .into_any_element();
    Ok(DescriptionItem::new(label).value(value).span(span))
}

struct StructuralDisplayComponent;

impl ComponentRenderer for StructuralDisplayComponent {
    fn render(&self, props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        Err(ComponentError::new(format!(
            "<{}> must be rendered inside its structurally valid parent",
            props.element.tag
        )))
    }
}

fn ensure_no_children(props: &ComponentProps) -> Result<(), ComponentError> {
    if props.element.children.is_empty() {
        return Ok(());
    }
    Err(ComponentError::new(format!(
        "<{}> does not accept children",
        props.element.tag
    )))
}

fn structural_child_props(
    parent: &ComponentProps,
    index: usize,
    child: &VNode,
    expected_tag: &str,
) -> Result<ComponentProps, ComponentError> {
    let Some(element) = child.element() else {
        return Err(ComponentError::new(format!(
            "<{}> only accepts direct <{expected_tag}> children",
            parent.element.tag
        )));
    };
    if !element.tag.eq_ignore_ascii_case(expected_tag) {
        return Err(ComponentError::new(format!(
            "<{}> only accepts direct <{expected_tag}> children, found <{}>",
            parent.element.tag, element.tag
        )));
    }
    Ok(ComponentProps::new(
        element.clone(),
        parent.path.child(index),
    ))
}
