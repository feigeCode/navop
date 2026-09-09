use connection_form::declarative::{
    DECLARATIVE_TEXTAREA_DEFAULT_ROWS, DeclarativeFieldType, DeclarativeFormConfig,
    DeclarativeFormField, DeclarativeFormTab, DeclarativeSelectOption, DeclarativeVisibilityRule,
};
use extension_runtime::extension::manifest::{ResourceConnectionFieldType, ResourceConnectionForm};

pub(super) fn declarative_config(form: &ResourceConnectionForm) -> DeclarativeFormConfig {
    DeclarativeFormConfig {
        tabs: form
            .tabs
            .iter()
            .map(|tab| DeclarativeFormTab {
                id: tab.id.clone(),
                label: tab.label.clone(),
                fields: tab.fields.iter().map(field_config).collect(),
            })
            .collect(),
    }
}

fn field_config(
    field: &extension_runtime::extension::manifest::ResourceConnectionFormField,
) -> DeclarativeFormField {
    DeclarativeFormField {
        id: field.id.clone(),
        label: field.label.clone(),
        field_type: match field.field_type {
            ResourceConnectionFieldType::Text => DeclarativeFieldType::Text,
            ResourceConnectionFieldType::Number => DeclarativeFieldType::Number,
            ResourceConnectionFieldType::Password => DeclarativeFieldType::Password,
            ResourceConnectionFieldType::TextArea => DeclarativeFieldType::TextArea,
            ResourceConnectionFieldType::Select => DeclarativeFieldType::Select,
            ResourceConnectionFieldType::Checkbox => DeclarativeFieldType::Checkbox,
            ResourceConnectionFieldType::FilePath => DeclarativeFieldType::FilePath,
            ResourceConnectionFieldType::Auth => DeclarativeFieldType::Auth,
        },
        rows: field.rows.unwrap_or(DECLARATIVE_TEXTAREA_DEFAULT_ROWS),
        required: field.required,
        default_value: field.default_value.clone(),
        placeholder: field.placeholder.clone(),
        secret: field.secret,
        options: field
            .options
            .iter()
            .map(|option| DeclarativeSelectOption {
                value: option.value.clone(),
                label: option.label.clone(),
            })
            .collect(),
        visible_when: field
            .visible_when
            .iter()
            .map(|rule| DeclarativeVisibilityRule {
                field: rule.field.clone(),
                equals: Some(rule.equals.clone()),
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_runtime::extension::manifest::ResourceConnectionFormField;

    fn field(field_type: ResourceConnectionFieldType) -> ResourceConnectionFormField {
        ResourceConnectionFormField {
            id: "auth".into(),
            label: "Authentication".into(),
            field_type,
            required: false,
            default_value: None,
            placeholder: None,
            secret: false,
            options: Vec::new(),
            visible_when: Vec::new(),
            rows: None,
        }
    }

    #[test]
    fn auth_field_type_maps_to_declarative_auth() {
        assert_eq!(
            DeclarativeFieldType::Auth,
            field_config(&field(ResourceConnectionFieldType::Auth)).field_type
        );
    }

    #[test]
    fn auth_maps_without_secret_flag() {
        let mapped = field_config(&field(ResourceConnectionFieldType::Auth));
        assert!(!mapped.secret, "Auth 组件由引擎内部管理密码 secret");
        assert!(mapped.options.is_empty());
    }

    #[test]
    fn file_path_and_rows_map_to_declarative_engine() {
        let mapped = field_config(&field(ResourceConnectionFieldType::FilePath));
        assert_eq!(DeclarativeFieldType::FilePath, mapped.field_type);

        let mut form_field = field(ResourceConnectionFieldType::TextArea);
        form_field.rows = Some(12);
        assert_eq!(12, field_config(&form_field).rows);
    }
}
