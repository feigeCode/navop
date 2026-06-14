use std::collections::BTreeMap;

use anyhow::{Result, bail};
use extension_component::{
    DbSelectorKind, DbSelectorSource, FieldSource, FieldValue, SelectOption, UiAction, UiField,
    UiFieldKind, UiNode, ViewActionEvent, ViewMode, ViewSpec,
};

use crate::extension_selector_parts::{selector_parts, selector_suffix};

pub use crate::extension_widget_view::{ExtensionWidgetActionHandler, ExtensionWidgetView};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionWidgetModel {
    pub id: String,
    pub title: String,
    pub mode: ViewMode,
    pub text_blocks: Vec<String>,
    pub fields: Vec<ExtensionWidgetField>,
    pub actions: Vec<UiAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionWidgetField {
    pub id: String,
    pub label: String,
    pub kind: UiFieldKind,
    pub required: bool,
    pub source: Option<FieldSource>,
    pub value: Option<String>,
    pub options: Vec<SelectOption>,
}

pub fn build_extension_widget_model(spec: &ViewSpec) -> Result<ExtensionWidgetModel> {
    build_extension_widget_model_with_options(spec, BTreeMap::new())
}

pub fn build_extension_widget_model_with_options(
    spec: &ViewSpec,
    selector_options: BTreeMap<String, Vec<SelectOption>>,
) -> Result<ExtensionWidgetModel> {
    validate_view_spec(spec)?;
    let mut text_blocks = Vec::new();
    let mut fields = Vec::new();
    for node in &spec.nodes {
        match node {
            UiNode::Text { text } => text_blocks.push(text.clone()),
            UiNode::Form {
                fields: form_fields,
            } => fields.extend(
                form_fields
                    .iter()
                    .flat_map(|field| widget_fields(field, &selector_options)),
            ),
        }
    }
    Ok(ExtensionWidgetModel {
        id: spec.id.clone(),
        title: spec.title.clone(),
        mode: spec.mode.clone(),
        text_blocks,
        fields,
        actions: spec.actions.clone(),
    })
}

pub fn default_form_values(model: &ExtensionWidgetModel) -> BTreeMap<String, String> {
    model
        .fields
        .iter()
        .filter_map(|field| {
            if field.kind == UiFieldKind::Checkbox {
                return Some((field.id.clone(), checkbox_value(field.value.as_deref())));
            }
            let value = field
                .value
                .clone()
                .or_else(|| field.options.first().map(|option| option.value.clone()))?;
            Some((field.id.clone(), value))
        })
        .collect()
}

pub fn form_values_to_action_event(
    view_id: &str,
    action_id: &str,
    values: &BTreeMap<String, String>,
) -> ViewActionEvent {
    let mut fields = composite_alias_values(values);
    fields.extend(values.iter().map(|(id, value)| FieldValue {
        id: id.clone(),
        value: value.clone(),
    }));
    ViewActionEvent {
        view_id: view_id.to_string(),
        action_id: action_id.to_string(),
        fields,
    }
}

pub(crate) fn field_source_label(field: &ExtensionWidgetField) -> String {
    if !field.options.is_empty() {
        return format!(
            "{} / {} 个选项",
            field.options[0].label,
            field.options.len()
        );
    }
    match &field.source {
        Some(FieldSource::StaticOptions(options)) => format!("{} 个选项", options.len()),
        Some(FieldSource::DbSelector(source)) => db_selector_label(&source.kind).to_string(),
        None => format!("输入 {}", field.id),
    }
}

pub(crate) fn db_selector_label(kind: &DbSelectorKind) -> &'static str {
    match kind {
        DbSelectorKind::Connection => "选择数据库连接",
        DbSelectorKind::Database => "选择数据库",
        DbSelectorKind::Schema => "选择 Schema",
        DbSelectorKind::Table => "选择表",
        DbSelectorKind::Column => "选择字段",
    }
}

fn validate_view_spec(spec: &ViewSpec) -> Result<()> {
    if spec.id.trim().is_empty() {
        bail!("extension view id is required");
    }
    if spec.title.trim().is_empty() {
        bail!("extension view title is required");
    }
    for node in &spec.nodes {
        if let UiNode::Form { fields } = node {
            validate_fields(fields)?;
        }
    }
    for action in &spec.actions {
        if action.id.trim().is_empty() {
            bail!("extension view action id is required");
        }
    }
    Ok(())
}

fn validate_fields(fields: &[UiField]) -> Result<()> {
    for field in fields {
        if field.id.trim().is_empty() {
            bail!("extension view field id is required");
        }
        if field.label.trim().is_empty() {
            bail!("extension view field label is required");
        }
    }
    Ok(())
}

fn widget_fields(
    field: &UiField,
    selector_options: &BTreeMap<String, Vec<SelectOption>>,
) -> Vec<ExtensionWidgetField> {
    let Some(FieldSource::DbSelector(source)) = &field.source else {
        return vec![widget_field(
            field,
            field.id.clone(),
            field.label.clone(),
            field.source.clone(),
            field.value.clone(),
            selector_options.get(&field.id).cloned(),
        )];
    };
    selector_parts(source)
        .into_iter()
        .map(|part| {
            let id = format!("{}.{}", field.id, part.suffix);
            let options = selector_options.get(&id).cloned().or_else(|| {
                deepest_part(source, part.suffix)
                    .then(|| selector_options.get(&field.id).cloned())
                    .flatten()
            });
            let value = part.value.or_else(|| {
                deepest_part(source, part.suffix)
                    .then(|| field.value.clone())
                    .flatten()
            });
            widget_field(
                field,
                id.clone(),
                part.label.to_string(),
                Some(FieldSource::DbSelector(part.source)),
                value,
                options,
            )
        })
        .collect()
}

fn widget_field(
    field: &UiField,
    id: String,
    label: String,
    source: Option<FieldSource>,
    value: Option<String>,
    loaded_options: Option<Vec<SelectOption>>,
) -> ExtensionWidgetField {
    ExtensionWidgetField {
        id,
        label,
        kind: field.kind.clone(),
        required: field.required,
        source,
        value,
        options: loaded_options.unwrap_or_else(|| field_static_options(field).unwrap_or_default()),
    }
}

fn field_static_options(field: &UiField) -> Option<Vec<SelectOption>> {
    match field.source.as_ref()? {
        FieldSource::StaticOptions(options) => Some(options.clone()),
        FieldSource::DbSelector(_) => None,
    }
}

fn checkbox_value(value: Option<&str>) -> String {
    if value
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            )
        })
        .unwrap_or(false)
    {
        "true".to_string()
    } else {
        "false".to_string()
    }
}

fn deepest_part(source: &DbSelectorSource, suffix: &str) -> bool {
    selector_suffix(&source.kind) == suffix
}

fn composite_alias_values(values: &BTreeMap<String, String>) -> Vec<FieldValue> {
    let mut grouped: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (id, value) in values {
        let Some((base, suffix)) = id.split_once('.') else {
            continue;
        };
        grouped
            .entry(base.to_string())
            .or_default()
            .insert(suffix.to_string(), value.clone());
    }
    grouped
        .into_iter()
        .filter_map(|(base, parts)| {
            deepest_value(&parts).map(|value| FieldValue { id: base, value })
        })
        .collect()
}

fn deepest_value(parts: &BTreeMap<String, String>) -> Option<String> {
    ["column", "table", "schema", "database", "connection_id"]
        .into_iter()
        .find_map(|key| parts.get(key).cloned())
}
