use declarative_ui_demo::{
    ComponentProps, ComponentRegistry, ComponentRenderer, ComponentResult, RegistryError,
    RenderContext,
};
use gpui::{IntoElement, ParentElement, div};

struct SqlEditorComponent;

impl ComponentRenderer for SqlEditorComponent {
    fn render(&self, _props: ComponentProps, _context: &mut RenderContext<'_>) -> ComponentResult {
        Ok(div().child("SQL editor").into_any_element())
    }
}

#[test]
fn registry_contains_defaults_and_accepts_custom_components() {
    let mut registry = ComponentRegistry::with_defaults();

    for tag in [
        "div",
        "span",
        "section",
        "article",
        "header",
        "footer",
        "main",
        "nav",
        "button",
        "input",
        "textarea",
        "img",
        "group-box",
        "label",
        "tag",
        "skeleton",
        "form",
        "field",
        "checkbox",
        "switch",
        "radio",
        "table",
        "thead",
        "tbody",
        "tfoot",
        "tr",
        "th",
        "td",
        "caption",
        "list",
        "list-item",
        "alert",
        "badge",
        "progress",
        "spinner",
        "separator",
        "divider",
        "avatar",
        "avatar-group",
        "description-list",
        "description-item",
        "breadcrumb",
        "breadcrumb-item",
        "pagination",
        "rating",
        "tabs",
        "tab",
        "stepper",
        "stepper-item",
        "kbd",
        "slider",
        "accordion",
        "accordion-item",
        "collapsible",
        "collapsible-content",
        "resizable",
        "resizable-panel",
        "scroll",
    ] {
        assert!(registry.contains(tag), "missing default component: {tag}");
    }

    assert!(!registry.contains("sql-editor"));
    registry
        .register("sql-editor", SqlEditorComponent)
        .expect("custom component registration succeeds");
    assert!(registry.contains("sql-editor"));
}

#[test]
fn registry_rejects_empty_and_duplicate_normalized_tags() {
    let mut registry = ComponentRegistry::default();

    assert_eq!(
        Err(RegistryError::EmptyTag),
        registry.register("   ", SqlEditorComponent)
    );
    registry
        .register("SQL-EDITOR", SqlEditorComponent)
        .expect("first normalized tag succeeds");
    assert_eq!(
        Err(RegistryError::AlreadyRegistered {
            tag: "sql-editor".to_owned(),
        }),
        registry.register(" sql-editor ", SqlEditorComponent)
    );
}
