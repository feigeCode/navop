use std::collections::HashMap;

use db::{
    DatabaseActionDescriptor, DatabaseActionId, DatabaseActionManifest, DatabaseActionPlacement,
    DatabaseActionTarget, DatabaseActionToolbarScope, DatabaseFormField, DatabaseFormFieldType,
    DatabaseFormKind, DatabaseFormManifest, DatabaseFormSubmission, DatabaseFormTab,
    DatabaseUiCapabilities, DatabaseUiManifest, DbNodeType, FormDefaultRule, FormSelectOption,
    FormValueCondition, FormVisibilityRule, ReferenceDataKind, DATABASE_UI_MANIFEST_VERSION,
};

#[test]
fn default_capabilities_are_disabled() {
    let capabilities = DatabaseUiCapabilities::default();

    assert!(!capabilities.supports_schema);
    assert!(!capabilities.uses_schema_as_database);
    assert!(!capabilities.supports_sequences);
    assert!(!capabilities.supports_functions);
    assert!(!capabilities.supports_procedures);
    assert!(!capabilities.supports_triggers);
    assert!(!capabilities.supports_table_engine);
    assert!(!capabilities.supports_table_charset);
    assert!(!capabilities.supports_table_collation);
    assert!(!capabilities.supports_auto_increment);
    assert!(!capabilities.supports_tablespace);
    assert!(!capabilities.supports_unsigned);
    assert!(!capabilities.supports_enum_values);
    assert!(!capabilities.show_charset_in_column_detail);
    assert!(!capabilities.show_collation_in_column_detail);
    assert!(capabilities.table_engines.is_empty());
}

#[test]
fn manifest_types_round_trip_through_serde() {
    let manifest = DatabaseUiManifest {
        schema_version: DATABASE_UI_MANIFEST_VERSION,
        capabilities: DatabaseUiCapabilities {
            supports_schema: true,
            supports_functions: true,
            table_engines: vec!["InnoDB".into()],
            ..DatabaseUiCapabilities::default()
        },
        forms: vec![DatabaseFormManifest {
            kind: DatabaseFormKind::CreateDatabase,
            title_i18n_key: "Database.create.title".into(),
            submit_i18n_key: "Common.create".into(),
            tabs: vec![DatabaseFormTab {
                id: "general".into(),
                label_i18n_key: "Common.general".into(),
                fields: vec![DatabaseFormField {
                    id: "name".into(),
                    label_i18n_key: "Database.name".into(),
                    field_type: DatabaseFormFieldType::Text,
                    required: true,
                    default_value: Some("demo".into()),
                    placeholder_i18n_key: Some("Database.name.placeholder".into()),
                    help_i18n_key: Some("Database.name.help".into()),
                    options: vec![FormSelectOption {
                        value: "utf8mb4".into(),
                        label_i18n_key: "Charset.utf8mb4".into(),
                    }],
                    options_source: Some(ReferenceDataKind::MySqlCharsets),
                    visible_when: vec![FormVisibilityRule {
                        when_field: "mode".into(),
                        condition: FormValueCondition::Equals("advanced".into()),
                    }],
                    default_when: vec![FormDefaultRule {
                        when_field_changes: "charset".into(),
                        via: ReferenceDataKind::MySqlCollations,
                    }],
                    disabled_when_editing: true,
                    rows: Some(4),
                    min: Some(1),
                    max: Some(8),
                }],
            }],
        }],
        actions: DatabaseActionManifest {
            actions: vec![DatabaseActionDescriptor {
                id: DatabaseActionId::CreateDatabase,
                label_i18n_key: "Database.create".into(),
                icon: Some("plus".into()),
                targets: vec![DatabaseActionTarget {
                    node_type: DbNodeType::Connection,
                }],
                placement: DatabaseActionPlacement::Both,
                requires_active_connection: true,
                group: Some("database".into()),
                submenu_of: None,
                toolbar_scope: Some(DatabaseActionToolbarScope::CurrentNode),
            }],
        },
    };
    let submission = DatabaseFormSubmission {
        kind: DatabaseFormKind::CreateDatabase,
        field_values: HashMap::from([("name".into(), "demo".into())]),
    };

    let json = serde_json::to_string(&(manifest.clone(), submission.clone())).unwrap();
    let (decoded_manifest, decoded_submission): (DatabaseUiManifest, DatabaseFormSubmission) =
        serde_json::from_str(&json).unwrap();

    assert_eq!(decoded_manifest, manifest);
    assert_eq!(decoded_submission, submission);
}

#[test]
fn visibility_rule_evaluator_covers_all_conditions() {
    let rule = FormVisibilityRule {
        when_field: "mode".into(),
        condition: FormValueCondition::Equals("advanced".into()),
    };
    let state = HashMap::from([("mode".into(), "advanced".into())]);
    assert!(rule.matches(&state));

    let rule = FormVisibilityRule {
        when_field: "mode".into(),
        condition: FormValueCondition::NotEquals("simple".into()),
    };
    assert!(rule.matches(&state));

    let rule = FormVisibilityRule {
        when_field: "mode".into(),
        condition: FormValueCondition::In(vec!["advanced".into(), "expert".into()]),
    };
    assert!(rule.matches(&state));

    let rule = FormVisibilityRule {
        when_field: "mode".into(),
        condition: FormValueCondition::NotEmpty,
    };
    assert!(rule.matches(&state));
}
