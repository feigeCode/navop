use gpui::{IntoElement, ParentElement};
use gpui_component::{
    Sizable,
    table::{
        Table, TableBody, TableCaption, TableCell, TableFooter, TableHead, TableHeader, TableRow,
    },
};

use crate::{
    ComponentError, ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult,
    ComponentSchema, RegistryError, RenderContext, VNode,
};

use super::{parse_positive_usize_attribute, parse_size_attribute};

pub(super) fn register(registry: &mut ComponentRegistry) -> Result<(), RegistryError> {
    registry.register_with_schema(
        "table",
        ComponentSchema::new().attribute("size"),
        TableComponent,
    )?;
    for tag in ["thead", "tbody", "tfoot", "tr", "caption"] {
        registry.register_with_schema(tag, ComponentSchema::new(), StructuralTableComponent)?;
    }
    for tag in ["th", "td"] {
        registry.register_with_schema(
            tag,
            ComponentSchema::new()
                .attribute("colspan")
                .attribute("align"),
            StructuralTableComponent,
        )?;
    }
    Ok(())
}

struct TableComponent;

impl ComponentRenderer for TableComponent {
    fn render(&self, props: ComponentProps, context: &mut RenderContext<'_>) -> ComponentResult {
        let mut table = Table::new();
        if let Some(size) = parse_size_attribute(&props.element)? {
            table = table.with_size(size);
        }

        for (index, child) in props.element.children.iter().enumerate() {
            let child_props = structural_child_props(
                &props,
                index,
                child,
                &["thead", "tbody", "tfoot", "caption"],
            )?;
            table = match child_props.element.tag.as_str() {
                "thead" => table.child(build_header(child_props, context)?),
                "tbody" => table.child(build_body(child_props, context)?),
                "tfoot" => table.child(build_footer(child_props, context)?),
                "caption" => table.child(build_caption(child_props, context)),
                _ => unreachable!("structural_child_props validated the table child"),
            };
        }

        Ok(context.style(table, &props).into_any_element())
    }
}

fn build_header(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<TableHeader, ComponentError> {
    let rows = build_rows(&props, context)?;
    Ok(context.style(TableHeader::new().children(rows), &props))
}

fn build_body(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<TableBody, ComponentError> {
    let rows = build_rows(&props, context)?;
    Ok(context.style(TableBody::new().children(rows), &props))
}

fn build_footer(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<TableFooter, ComponentError> {
    let rows = build_rows(&props, context)?;
    Ok(context.style(TableFooter::new().children(rows), &props))
}

fn build_rows(
    props: &ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<Vec<TableRow>, ComponentError> {
    props
        .element
        .children
        .iter()
        .enumerate()
        .map(|(index, child)| {
            let row_props = structural_child_props(props, index, child, &["tr"])?;
            build_row(row_props, context)
        })
        .collect()
}

fn build_row(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<TableRow, ComponentError> {
    let mut row = TableRow::new();
    for (index, child) in props.element.children.iter().enumerate() {
        let child_props = structural_child_props(&props, index, child, &["th", "td"])?;
        row = match child_props.element.tag.as_str() {
            "th" => row.child(build_head(child_props, context)?),
            "td" => row.child(build_cell(child_props, context)?),
            _ => unreachable!("structural_child_props validated the row child"),
        };
    }
    Ok(context.style(row, &props))
}

fn build_head(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<TableHead, ComponentError> {
    let mut cell = TableHead::new().children(context.render_children(&props));
    if let Some(span) = parse_positive_usize_attribute(&props.element, "colspan")? {
        cell = cell.col_span(span);
    }
    cell = align_head(cell, &props)?;
    Ok(context.style(cell, &props))
}

fn build_cell(
    props: ComponentProps,
    context: &mut RenderContext<'_>,
) -> Result<TableCell, ComponentError> {
    let mut cell = TableCell::new().children(context.render_children(&props));
    if let Some(span) = parse_positive_usize_attribute(&props.element, "colspan")? {
        cell = cell.col_span(span);
    }
    cell = align_cell(cell, &props)?;
    Ok(context.style(cell, &props))
}

fn align_head(cell: TableHead, props: &ComponentProps) -> Result<TableHead, ComponentError> {
    let Some(align) = props.element.attr("align") else {
        return Ok(cell);
    };
    match align.trim().to_ascii_lowercase().as_str() {
        "left" => Ok(cell),
        "center" => Ok(cell.text_center()),
        "right" => Ok(cell.text_right()),
        _ => Err(invalid_alignment(props, align)),
    }
}

fn align_cell(cell: TableCell, props: &ComponentProps) -> Result<TableCell, ComponentError> {
    let Some(align) = props.element.attr("align") else {
        return Ok(cell);
    };
    match align.trim().to_ascii_lowercase().as_str() {
        "left" => Ok(cell),
        "center" => Ok(cell.text_center()),
        "right" => Ok(cell.text_right()),
        _ => Err(invalid_alignment(props, align)),
    }
}

fn invalid_alignment(props: &ComponentProps, value: &str) -> ComponentError {
    ComponentError::new(format!(
        "attribute `align` on <{}> must be left, center, or right, got `{value}`",
        props.element.tag
    ))
}

fn build_caption(props: ComponentProps, context: &mut RenderContext<'_>) -> TableCaption {
    let caption = TableCaption::new().children(context.render_children(&props));
    context.style(caption, &props)
}

struct StructuralTableComponent;

impl ComponentRenderer for StructuralTableComponent {
    fn render(&self, props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        Err(ComponentError::new(format!(
            "<{}> must be rendered inside a structurally valid <table>",
            props.element.tag
        )))
    }
}

fn structural_child_props(
    parent: &ComponentProps,
    index: usize,
    child: &VNode,
    expected_tags: &[&str],
) -> Result<ComponentProps, ComponentError> {
    let expected = expected_tags
        .iter()
        .map(|tag| format!("<{tag}>"))
        .collect::<Vec<_>>()
        .join(", ");
    let Some(element) = child.element() else {
        return Err(ComponentError::new(format!(
            "<{}> only accepts direct {expected} children",
            parent.element.tag
        )));
    };
    if !expected_tags.contains(&element.tag.as_str()) {
        return Err(ComponentError::new(format!(
            "<{}> only accepts direct {expected} children, found <{}>",
            parent.element.tag, element.tag
        )));
    }
    Ok(ComponentProps::new(
        element.clone(),
        parent.path.child(index),
    ))
}
