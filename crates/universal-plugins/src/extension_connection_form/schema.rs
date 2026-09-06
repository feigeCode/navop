use connection_form::declarative::{
    DeclarativeFieldType, DeclarativeFormConfig, DeclarativeFormField, DeclarativeFormTab,
    DeclarativeSelectOption, DeclarativeVisibilityRule,
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
        },
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
                equals: rule.equals.clone(),
            })
            .collect(),
    }
}
