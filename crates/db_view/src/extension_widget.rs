use std::collections::BTreeMap;

use anyhow::{Result, bail};
use extension_component::{
    DbSelectorKind, DbSelectorSource, FieldSource, FieldValue, SelectOption, UiAction, UiField,
    UiFieldKind, UiNode, ViewActionEvent, ViewMode, ViewSpec,
};
use rust_i18n::t;

use crate::db_object_selector::{
    DbObjectSelectorPolicy, selector_parts_for_source_with_policy, selector_source_part,
    selector_suffix,
};

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

pub type ExtensionWidgetSelectorPolicy = DbObjectSelectorPolicy;
pub type ExtensionWidgetSelectorPolicies = BTreeMap<String, ExtensionWidgetSelectorPolicy>;

pub fn build_extension_widget_model(spec: &ViewSpec) -> Result<ExtensionWidgetModel> {
    build_extension_widget_model_with_options(spec, BTreeMap::new())
}

pub fn build_extension_widget_model_with_options(
    spec: &ViewSpec,
    selector_options: BTreeMap<String, Vec<SelectOption>>,
) -> Result<ExtensionWidgetModel> {
    build_extension_widget_model_with_selector_data(spec, selector_options, BTreeMap::new())
}

pub fn build_extension_widget_model_with_selector_data(
    spec: &ViewSpec,
    selector_options: BTreeMap<String, Vec<SelectOption>>,
    selector_policies: ExtensionWidgetSelectorPolicies,
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
                    .flat_map(|field| widget_fields(field, &selector_options, &selector_policies)),
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
        return t!(
            "ExtensionWidget.selected_options_count",
            selected = field.options[0].label.clone(),
            count = field.options.len()
        )
        .to_string();
    }
    match &field.source {
        Some(FieldSource::StaticOptions(options)) => {
            t!("ExtensionWidget.options_count", count = options.len()).to_string()
        }
        Some(FieldSource::DbSelector(source)) => db_selector_label(&source.kind).to_string(),
        None => t!(
            "ExtensionWidget.input_placeholder",
            field = field.id.clone()
        )
        .to_string(),
    }
}

pub(crate) fn db_selector_label(kind: &DbSelectorKind) -> String {
    match kind {
        DbSelectorKind::Connection => t!("DbObjectSelector.select_connection").to_string(),
        DbSelectorKind::Database => t!("DbObjectSelector.select_database").to_string(),
        DbSelectorKind::Schema => t!("DbObjectSelector.select_schema").to_string(),
        DbSelectorKind::Table => t!("DbObjectSelector.select_table").to_string(),
        DbSelectorKind::Column => t!("DbObjectSelector.select_column").to_string(),
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
    selector_policies: &ExtensionWidgetSelectorPolicies,
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
    let policy = selector_policies
        .get(&field.id)
        .copied()
        .unwrap_or_else(DbObjectSelectorPolicy::generic);
    selector_parts_for_source_with_policy(source, policy)
        .into_iter()
        .map(|part| {
            let id = format!("{}.{}", field.id, part.suffix);
            let source_part = selector_source_part(source, &part);
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
                part.label,
                Some(FieldSource::DbSelector(source_part)),
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
