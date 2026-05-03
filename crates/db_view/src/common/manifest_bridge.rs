use crate::common::db_connection_form::{
    DbFormConfig, FormField, FormFieldType, FormSelectItem, TabGroup,
};
use db::plugin::DatabasePlugin;
use db::plugin_manifest::{
    DatabaseActionDescriptor, DatabaseFormField, DatabaseFormFieldType, DatabaseFormKind,
    DatabaseFormManifest, DatabaseUiCapabilities, DatabaseUiManifest, FormSelectOption,
};
use one_core::storage::DatabaseType;
use rust_i18n::locale;
use std::collections::HashMap;

use crate::database_view_plugin::{ColumnEditorCapabilities, TableDesignerCapabilities};

pub(crate) fn translate(key: &str) -> String {
    crate::_rust_i18n_translate(locale().as_ref(), key).into_owned()
}

fn translate_connection_form_text(key_or_text: &str) -> String {
    db::translate_or_raw_for_locale(locale().as_ref(), key_or_text)
}

pub(crate) fn find_form(
    manifest: &DatabaseUiManifest,
    kind: DatabaseFormKind,
) -> Option<DatabaseFormManifest> {
    manifest
        .forms
        .iter()
        .find(|form| form.kind == kind)
        .cloned()
}

pub(crate) fn to_connection_form_config(
    db_type: DatabaseType,
    form: &DatabaseFormManifest,
    plugin: &dyn DatabasePlugin,
) -> DbFormConfig {
    let mut default_state = HashMap::new();
    for tab in &form.tabs {
        for field in &tab.fields {
            let value = field.default_value.clone().unwrap_or_default();
            default_state.insert(field.id.clone(), value);
        }
    }

    DbFormConfig {
        db_type,
        title: translate_connection_form_text(&form.title_i18n_key),
        hidden_params: HashMap::new(),
        tab_groups: form
            .tabs
            .iter()
            .map(|tab| TabGroup {
                name: tab.id.clone(),
                label: translate_connection_form_text(&tab.label_i18n_key),
                fields: tab
                    .fields
                    .iter()
                    .map(|field| to_connection_field(field, plugin, &default_state))
                    .collect(),
            })
            .collect(),
    }
}

fn to_connection_field(
    field: &DatabaseFormField,
    plugin: &dyn DatabasePlugin,
    context: &HashMap<String, String>,
) -> FormField {
    let options = resolve_field_options(plugin, field, context)
        .into_iter()
        .map(|option| {
            (
                option.value,
                translate_connection_form_text(&option.label_i18n_key),
            )
        })
        .collect();

    FormField {
        name: field.id.clone(),
        label: translate_connection_form_text(&field.label_i18n_key),
        placeholder: field
            .placeholder_i18n_key
            .as_deref()
            .map(translate_connection_form_text)
            .unwrap_or_default(),
        field_type: to_connection_field_type(field.field_type),
        rows: field.rows.unwrap_or(5) as usize,
        required: field.required,
        default_value: field.default_value.clone().unwrap_or_default(),
        options,
    }
}

fn to_connection_field_type(field_type: DatabaseFormFieldType) -> FormFieldType {
    match field_type {
        DatabaseFormFieldType::Number => FormFieldType::Number,
        DatabaseFormFieldType::Password => FormFieldType::Password,
        DatabaseFormFieldType::TextArea => FormFieldType::TextArea,
        DatabaseFormFieldType::Select => FormFieldType::Select,
        DatabaseFormFieldType::Text
        | DatabaseFormFieldType::Checkbox
        | DatabaseFormFieldType::FilePath => FormFieldType::Text,
    }
}

pub(crate) fn resolve_field_options(
    plugin: &dyn DatabasePlugin,
    field: &DatabaseFormField,
    context: &HashMap<String, String>,
) -> Vec<FormSelectOption> {
    if let Some(kind) = field.options_source {
        let resolved = plugin.resolve_reference_data(kind, context);
        if !resolved.is_empty() {
            return resolved;
        }
    }
    field.options.clone()
}

pub(crate) fn to_select_items(options: Vec<FormSelectOption>) -> Vec<FormSelectItem> {
    options
        .into_iter()
        .map(|option| FormSelectItem::new(option.value, translate(&option.label_i18n_key)))
        .collect()
}

pub(crate) fn field_visible(field: &DatabaseFormField, values: &HashMap<String, String>) -> bool {
    field.visible_when.iter().all(|rule| rule.matches(values))
}

pub(crate) fn default_select_value(
    field: &DatabaseFormField,
    options: &[FormSelectOption],
) -> String {
    let preferred = field.default_value.clone().unwrap_or_default();
    if !preferred.is_empty() && options.iter().any(|option| option.value == preferred) {
        return preferred;
    }

    options
        .first()
        .map(|option| option.value.clone())
        .unwrap_or_default()
}

pub(crate) fn to_table_designer_capabilities(
    capabilities: &DatabaseUiCapabilities,
) -> TableDesignerCapabilities {
    TableDesignerCapabilities {
        supports_engine: capabilities.supports_table_engine,
        supports_charset: capabilities.supports_table_charset,
        supports_collation: capabilities.supports_table_collation,
        supports_auto_increment: capabilities.supports_auto_increment,
        supports_tablespace: capabilities.supports_tablespace,
    }
}

pub(crate) fn to_column_editor_capabilities(
    capabilities: &DatabaseUiCapabilities,
) -> ColumnEditorCapabilities {
    ColumnEditorCapabilities {
        supports_unsigned: capabilities.supports_unsigned,
        supports_enum_values: capabilities.supports_enum_values,
        show_charset_in_detail: capabilities.show_charset_in_column_detail,
        show_collation_in_detail: capabilities.show_collation_in_column_detail,
    }
}

pub(crate) fn matches_node_type(
    action: &DatabaseActionDescriptor,
    node_type: db::DbNodeType,
) -> bool {
    action
        .targets
        .iter()
        .any(|target| target.node_type == node_type)
}

#[cfg(test)]
mod tests {
    use super::translate_connection_form_text;

    #[test]
    fn connection_form_translation_keeps_literal_placeholder() {
        assert_eq!(translate_connection_form_text("28800"), "28800");
    }
}
