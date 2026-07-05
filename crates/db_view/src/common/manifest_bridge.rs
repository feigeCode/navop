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
    translate_manifest_text_for_locale(locale().as_ref(), key)
}

fn translate_manifest_text_for_locale(locale: &str, key_or_text: &str) -> String {
    let translated = crate::_rust_i18n_translate(locale, key_or_text).into_owned();
    let missing_with_locale = format!("{locale}.{key_or_text}");

    if translated != key_or_text && translated != missing_with_locale {
        return translated;
    }

    db::translate_or_raw_for_locale(locale, key_or_text)
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
    to_connection_form_config_with_text_resolver(
        db_type,
        form,
        plugin,
        translate_connection_form_text,
    )
}

pub(crate) fn to_connection_form_config_with_text_resolver<F>(
    db_type: DatabaseType,
    form: &DatabaseFormManifest,
    plugin: &dyn DatabasePlugin,
    translate_text: F,
) -> DbFormConfig
where
    F: Fn(&str) -> String,
{
    let mut default_state = HashMap::new();
    for tab in &form.tabs {
        for field in &tab.fields {
            let value = field.default_value.clone().unwrap_or_default();
            default_state.insert(field.id.clone(), value);
        }
    }

    DbFormConfig {
        db_type,
        title: translate_text(&form.title_i18n_key),
        hidden_params: HashMap::new(),
        tab_groups: form
            .tabs
            .iter()
            .map(|tab| TabGroup {
                name: tab.id.clone(),
                label: translate_text(&tab.label_i18n_key),
                fields: tab
                    .fields
                    .iter()
                    .map(|field| {
                        to_connection_field(field, plugin, &default_state, &translate_text)
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn to_connection_field(
    field: &DatabaseFormField,
    plugin: &dyn DatabasePlugin,
    context: &HashMap<String, String>,
    translate_text: &impl Fn(&str) -> String,
) -> FormField {
    let options = resolve_field_options(plugin, field, context)
        .into_iter()
        .map(|option| (option.value, translate_text(&option.label_i18n_key)))
        .collect();

    FormField {
        name: field.id.clone(),
        label: translate_text(&field.label_i18n_key),
        placeholder: field
            .placeholder_i18n_key
            .as_deref()
            .map(translate_text)
            .unwrap_or_default(),
        field_type: to_connection_field_type(field.field_type),
        rows: field.rows.unwrap_or(5) as usize,
        required: field.required,
        default_value: field.default_value.clone().unwrap_or_default(),
        options,
        visible_when: field.visible_when.clone(),
    }
}

fn to_connection_field_type(field_type: DatabaseFormFieldType) -> FormFieldType {
    match field_type {
        DatabaseFormFieldType::Number => FormFieldType::Number,
        DatabaseFormFieldType::Password => FormFieldType::Password,
        DatabaseFormFieldType::TextArea => FormFieldType::TextArea,
        DatabaseFormFieldType::Select => FormFieldType::Select,
        DatabaseFormFieldType::Checkbox => FormFieldType::Checkbox,
        DatabaseFormFieldType::FilePath => FormFieldType::FilePath,
        DatabaseFormFieldType::Text => FormFieldType::Text,
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

pub(crate) fn to_select_items(
    options: Vec<FormSelectOption>,
    text_resolver: &dyn Fn(&str) -> String,
) -> Vec<FormSelectItem> {
    options
        .into_iter()
        .map(|option| FormSelectItem::new(option.value, text_resolver(&option.label_i18n_key)))
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
    use super::*;
    use db::ipc::{ExternalDatabasePlugin, IpcDriverEntry, IpcDriverManifest, IpcDriverTransport};
    use db::plugin_manifest::{DatabaseFormTab, FormValueCondition, FormVisibilityRule};
    use serde_json::json;
    use std::path::PathBuf;

    #[test]
    fn manifest_translation_falls_back_to_db_locale() {
        assert_eq!(
            "名称",
            translate_manifest_text_for_locale("zh-CN", "DatabaseUser.name")
        );
        assert_eq!(
            "28800",
            translate_manifest_text_for_locale("zh-CN", "28800")
        );
    }

    #[test]
    fn select_items_use_supplied_text_resolver() {
        let items = to_select_items(
            vec![FormSelectOption {
                value: "SELECT".into(),
                label_i18n_key: "DatabaseUser.privilege_select".into(),
            }],
            &|key| format!("translated:{key}"),
        );

        assert_eq!("SELECT", items[0].value);
        assert_eq!("translated:DatabaseUser.privilege_select", items[0].label);
    }

    #[test]
    fn connection_form_translation_keeps_literal_placeholder() {
        assert_eq!(translate_connection_form_text("28800"), "28800");
    }

    #[test]
    fn connection_form_config_uses_custom_text_resolver() {
        let form = DatabaseFormManifest {
            kind: DatabaseFormKind::Connection,
            title_i18n_key: "driver.connection.title".into(),
            submit_i18n_key: "submit".into(),
            tabs: vec![DatabaseFormTab {
                id: "general".into(),
                label_i18n_key: "driver.connection.tab.general".into(),
                fields: vec![
                    serde_json::from_value(json!({
                        "id": "mode",
                        "label_i18n_key": "driver.connection.field.mode",
                        "field_type": "Select",
                        "required": false,
                        "default_value": "local",
                        "placeholder_i18n_key": "driver.connection.field.mode.placeholder",
                        "help_i18n_key": null,
                        "options": [
                            {
                                "value": "local",
                                "label_i18n_key": "driver.connection.mode.local"
                            }
                        ],
                        "options_source": null,
                        "visible_when": [],
                        "default_when": [],
                        "disabled_when_editing": false,
                        "rows": null,
                        "min": null,
                        "max": null
                    }))
                    .unwrap(),
                ],
            }],
        };
        let plugin = ExternalDatabasePlugin::for_driver(demo_driver());

        let config = to_connection_form_config_with_text_resolver(
            DatabaseType::external("demo"),
            &form,
            &plugin,
            |key| format!("translated:{key}"),
        );

        assert_eq!("translated:driver.connection.title", config.title);
        assert_eq!(
            "translated:driver.connection.tab.general",
            config.tab_groups[0].label
        );
        let field = &config.tab_groups[0].fields[0];
        assert_eq!(FormFieldType::Select, field.field_type);
        assert_eq!("translated:driver.connection.field.mode", field.label);
        assert_eq!(
            "translated:driver.connection.field.mode.placeholder",
            field.placeholder
        );
        assert_eq!(
            vec![(
                "local".to_string(),
                "translated:driver.connection.mode.local".to_string()
            )],
            field.options
        );
    }

    #[test]
    fn manifest_bridge_preserves_checkbox_and_file_path_fields() {
        let form = DatabaseFormManifest {
            kind: DatabaseFormKind::Connection,
            title_i18n_key: "driver.connection.title".into(),
            submit_i18n_key: "submit".into(),
            tabs: vec![DatabaseFormTab {
                id: "ssh".into(),
                label_i18n_key: "driver.connection.tab.ssh".into(),
                fields: vec![
                    serde_json::from_value(json!({
                        "id": "ssh_tunnel_enabled",
                        "label_i18n_key": "driver.connection.field.ssh_enabled",
                        "field_type": "Checkbox",
                        "required": false,
                        "default_value": "false",
                        "placeholder_i18n_key": null,
                        "help_i18n_key": null,
                        "options": [],
                        "options_source": null,
                        "visible_when": [],
                        "default_when": [],
                        "disabled_when_editing": false,
                        "rows": null,
                        "min": null,
                        "max": null
                    }))
                    .unwrap(),
                    serde_json::from_value(json!({
                        "id": "ssl_ca_file",
                        "label_i18n_key": "driver.connection.field.ssl_ca_file",
                        "field_type": "FilePath",
                        "required": false,
                        "default_value": "",
                        "placeholder_i18n_key": null,
                        "help_i18n_key": null,
                        "options": [],
                        "options_source": null,
                        "visible_when": [],
                        "default_when": [],
                        "disabled_when_editing": false,
                        "rows": null,
                        "min": null,
                        "max": null
                    }))
                    .unwrap(),
                ],
            }],
        };
        let plugin = ExternalDatabasePlugin::for_driver(demo_driver());

        let config = to_connection_form_config(DatabaseType::external("demo"), &form, &plugin);

        assert_eq!(
            FormFieldType::Checkbox,
            config.tab_groups[0].fields[0].field_type
        );
        assert_eq!(
            FormFieldType::FilePath,
            config.tab_groups[0].fields[1].field_type
        );
    }

    #[test]
    fn connection_form_config_preserves_field_visibility_rules() {
        let mut form = DatabaseFormManifest {
            kind: DatabaseFormKind::Connection,
            title_i18n_key: "driver.connection.title".into(),
            submit_i18n_key: "submit".into(),
            tabs: vec![DatabaseFormTab {
                id: "ssl".into(),
                label_i18n_key: "driver.connection.tab.ssl".into(),
                fields: vec![
                    serde_json::from_value(json!({
                        "id": "ssl_enabled",
                        "label_i18n_key": "driver.connection.field.ssl_enabled",
                        "field_type": "Checkbox",
                        "required": false,
                        "default_value": "false",
                        "placeholder_i18n_key": null,
                        "help_i18n_key": null,
                        "options": [],
                        "options_source": null,
                        "visible_when": [],
                        "default_when": [],
                        "disabled_when_editing": false,
                        "rows": null,
                        "min": null,
                        "max": null
                    }))
                    .unwrap(),
                    serde_json::from_value(json!({
                        "id": "ssl_ca_file",
                        "label_i18n_key": "driver.connection.field.ssl_ca_file",
                        "field_type": "FilePath",
                        "required": true,
                        "default_value": "",
                        "placeholder_i18n_key": null,
                        "help_i18n_key": null,
                        "options": [],
                        "options_source": null,
                        "visible_when": [],
                        "default_when": [],
                        "disabled_when_editing": false,
                        "rows": null,
                        "min": null,
                        "max": null
                    }))
                    .unwrap(),
                ],
            }],
        };
        form.tabs[0].fields[1].visible_when = vec![FormVisibilityRule {
            when_field: "ssl_enabled".into(),
            condition: FormValueCondition::Equals("true".into()),
        }];
        let plugin = ExternalDatabasePlugin::for_driver(demo_driver());

        let config = to_connection_form_config(DatabaseType::external("demo"), &form, &plugin);

        assert_eq!(1, config.tab_groups[0].fields[1].visible_when.len());
        assert_eq!(
            "ssl_enabled",
            config.tab_groups[0].fields[1].visible_when[0].when_field
        );
    }

    fn demo_driver() -> IpcDriverManifest {
        IpcDriverManifest {
            id: "demo".into(),
            name: "Demo".into(),
            category: None,
            description: String::new(),
            version: String::new(),
            entry: IpcDriverEntry {
                command: "driver".into(),
                commands: Default::default(),
                args: Vec::new(),
                working_dir: None,
                env_from_config: Default::default(),
            },
            transport: IpcDriverTransport::local_socket("demo.sock"),
            dialect: Default::default(),
            capabilities: None,
            connection: Default::default(),
            methods: Vec::new(),
            ui: Default::default(),
            manifest_dir: PathBuf::from("."),
        }
    }
}
