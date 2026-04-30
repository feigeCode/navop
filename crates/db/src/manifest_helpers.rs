use crate::plugin_manifest::{
    DatabaseActionDescriptor, DatabaseActionId, DatabaseActionPlacement, DatabaseActionTarget,
    DatabaseActionToolbarScope, DatabaseFormField, DatabaseFormFieldType, DatabaseFormTab,
    FormSelectOption, FormValueCondition, FormVisibilityRule,
};
use crate::DbNodeType;

pub(crate) fn tab(
    id: &str,
    label_i18n_key: &str,
    fields: Vec<ManifestFieldBuilder>,
) -> DatabaseFormTab {
    DatabaseFormTab {
        id: id.into(),
        label_i18n_key: label_i18n_key.into(),
        fields: fields.into_iter().map(Into::into).collect(),
    }
}

pub(crate) fn field(
    id: &str,
    label_i18n_key: &str,
    field_type: DatabaseFormFieldType,
) -> ManifestFieldBuilder {
    ManifestFieldBuilder::new(id, label_i18n_key, field_type)
}

pub(crate) fn ssh_field(id: &str, label_i18n_key: &str) -> ManifestFieldBuilder {
    field(id, label_i18n_key, DatabaseFormFieldType::Text)
        .optional()
        .with_visibility(ssh_enabled_rules())
}

pub(crate) fn ssh_number_field(id: &str, label_i18n_key: &str) -> ManifestFieldBuilder {
    field(id, label_i18n_key, DatabaseFormFieldType::Number)
        .optional()
        .with_visibility(ssh_enabled_rules())
}

pub(crate) fn ssh_password_field(
    id: &str,
    label_i18n_key: &str,
    placeholder: &str,
) -> ManifestFieldBuilder {
    field(id, label_i18n_key, DatabaseFormFieldType::Password)
        .optional()
        .with_placeholder(placeholder)
        .with_visibility(ssh_enabled_rules())
}

pub(crate) fn yes_no_options() -> Vec<FormSelectOption> {
    vec![option("false", "Common.no"), option("true", "Common.yes")]
}

pub(crate) fn option(value: &str, label_i18n_key: &str) -> FormSelectOption {
    FormSelectOption {
        value: value.into(),
        label_i18n_key: label_i18n_key.into(),
    }
}

pub(crate) fn action(
    id: DatabaseActionId,
    label_i18n_key: &str,
    targets: Vec<DbNodeType>,
    placement: DatabaseActionPlacement,
) -> DatabaseActionDescriptor {
    action_with_scope(id, label_i18n_key, targets, placement, true, None)
}

pub(crate) fn action_with_scope(
    id: DatabaseActionId,
    label_i18n_key: &str,
    targets: Vec<DbNodeType>,
    placement: DatabaseActionPlacement,
    requires_active_connection: bool,
    toolbar_scope: Option<DatabaseActionToolbarScope>,
) -> DatabaseActionDescriptor {
    DatabaseActionDescriptor {
        id,
        label_i18n_key: label_i18n_key.into(),
        icon: None,
        targets: targets.into_iter().map(target).collect(),
        placement,
        requires_active_connection,
        group: None,
        submenu_of: None,
        toolbar_scope,
    }
}

pub(crate) fn target(node_type: DbNodeType) -> DatabaseActionTarget {
    DatabaseActionTarget { node_type }
}

pub(crate) fn equals_rule(field: &str, value: &str) -> FormVisibilityRule {
    FormVisibilityRule {
        when_field: field.into(),
        condition: FormValueCondition::Equals(value.into()),
    }
}

pub(crate) fn ssh_enabled_rules() -> Vec<FormVisibilityRule> {
    vec![equals_rule("ssh_tunnel_enabled", "true")]
}

pub(crate) fn ssh_auth_rules(expected_auth_type: &str) -> Vec<FormVisibilityRule> {
    vec![
        equals_rule("ssh_tunnel_enabled", "true"),
        equals_rule("ssh_auth_type", expected_auth_type),
    ]
}

#[derive(Clone)]
pub(crate) struct ManifestFieldBuilder {
    field: DatabaseFormField,
}

impl ManifestFieldBuilder {
    pub(crate) fn new(id: &str, label_i18n_key: &str, field_type: DatabaseFormFieldType) -> Self {
        Self {
            field: DatabaseFormField {
                id: id.into(),
                label_i18n_key: label_i18n_key.into(),
                field_type,
                required: true,
                default_value: None,
                placeholder_i18n_key: None,
                help_i18n_key: None,
                options: Vec::new(),
                options_source: None,
                visible_when: Vec::new(),
                default_when: Vec::new(),
                disabled_when_editing: false,
                rows: None,
                min: None,
                max: None,
            },
        }
    }

    pub(crate) fn optional(mut self) -> Self {
        self.field.required = false;
        self
    }

    pub(crate) fn with_default(mut self, value: &str) -> Self {
        self.field.default_value = Some(value.into());
        self
    }

    pub(crate) fn with_placeholder(mut self, value: &str) -> Self {
        self.field.placeholder_i18n_key = Some(value.into());
        self
    }

    pub(crate) fn with_options(mut self, options: Vec<FormSelectOption>) -> Self {
        self.field.options = options;
        self
    }

    pub(crate) fn with_options_source(
        mut self,
        options_source: crate::plugin_manifest::ReferenceDataKind,
    ) -> Self {
        self.field.options_source = Some(options_source);
        self
    }

    pub(crate) fn with_visibility(mut self, visible_when: Vec<FormVisibilityRule>) -> Self {
        self.field.visible_when = visible_when;
        self
    }

    pub(crate) fn disabled_when_editing(mut self, value: bool) -> Self {
        self.field.disabled_when_editing = value;
        self
    }

    pub(crate) fn with_rows(mut self, rows: u32) -> Self {
        self.field.rows = Some(rows);
        self
    }
}

impl From<ManifestFieldBuilder> for DatabaseFormField {
    fn from(value: ManifestFieldBuilder) -> Self {
        value.field
    }
}

pub(crate) trait DatabaseActionDescriptorExt {
    fn always_enabled(self) -> Self;
    fn with_toolbar_scope(self, toolbar_scope: DatabaseActionToolbarScope) -> Self;
}

impl DatabaseActionDescriptorExt for DatabaseActionDescriptor {
    fn always_enabled(mut self) -> Self {
        self.requires_active_connection = false;
        self
    }

    fn with_toolbar_scope(mut self, toolbar_scope: DatabaseActionToolbarScope) -> Self {
        self.toolbar_scope = Some(toolbar_scope);
        self
    }
}
