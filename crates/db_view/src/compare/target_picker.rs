use crate::db_object_selector::DbObjectSelectorPolicy;
pub(super) use crate::db_object_selector::{
    StringSelect, TargetConnectionControls, TargetStringControls, clear_string_select,
    load_databases_then, load_schemas_then, selected_string, set_connection_select,
    set_string_select, string_select_state,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CompareTargetCascadeAction {
    LoadDatabases,
    LoadSchemas,
    LoadTables,
}

pub(super) fn initial_compare_target_cascade_actions(
    policy: DbObjectSelectorPolicy,
    has_selected_database: bool,
) -> Vec<CompareTargetCascadeAction> {
    let mut actions = vec![CompareTargetCascadeAction::LoadDatabases];
    if !has_selected_database {
        return actions;
    }
    actions.extend(database_change_cascade_actions(policy));
    actions
}

pub(super) fn database_change_cascade_actions(
    policy: DbObjectSelectorPolicy,
) -> Vec<CompareTargetCascadeAction> {
    if policy.show_schema {
        vec![CompareTargetCascadeAction::LoadSchemas]
    } else {
        vec![CompareTargetCascadeAction::LoadTables]
    }
}

pub(super) fn schema_change_cascade_actions(
    has_selected_schema: bool,
) -> Vec<CompareTargetCascadeAction> {
    if has_selected_schema {
        vec![CompareTargetCascadeAction::LoadTables]
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::db_object_selector::DbObjectSelectorPolicy;

    #[test]
    fn initial_compare_target_cascade_waits_for_database_when_none_selected() {
        assert_eq!(
            super::initial_compare_target_cascade_actions(DbObjectSelectorPolicy::default(), false),
            vec![super::CompareTargetCascadeAction::LoadDatabases]
        );
    }

    #[test]
    fn initial_compare_target_cascade_waits_for_schema_on_schema_connections_with_database() {
        let policy = DbObjectSelectorPolicy {
            show_schema: true,
            schema_as_database: false,
        };

        assert_eq!(
            super::initial_compare_target_cascade_actions(policy, true),
            vec![
                super::CompareTargetCascadeAction::LoadDatabases,
                super::CompareTargetCascadeAction::LoadSchemas,
            ]
        );
    }

    #[test]
    fn initial_compare_target_cascade_loads_tables_without_schema_step() {
        assert_eq!(
            super::initial_compare_target_cascade_actions(DbObjectSelectorPolicy::default(), true),
            vec![
                super::CompareTargetCascadeAction::LoadDatabases,
                super::CompareTargetCascadeAction::LoadTables,
            ]
        );
    }

    #[test]
    fn database_change_waits_for_schema_before_loading_tables() {
        let policy = DbObjectSelectorPolicy {
            show_schema: true,
            schema_as_database: false,
        };

        assert_eq!(
            super::database_change_cascade_actions(policy),
            vec![super::CompareTargetCascadeAction::LoadSchemas]
        );
    }

    #[test]
    fn database_change_loads_tables_for_schema_as_database_connections() {
        let policy = DbObjectSelectorPolicy {
            show_schema: false,
            schema_as_database: true,
        };

        assert_eq!(
            super::database_change_cascade_actions(policy),
            vec![super::CompareTargetCascadeAction::LoadTables]
        );
    }

    #[test]
    fn database_change_loads_tables_for_connections_without_schemas() {
        assert_eq!(
            super::database_change_cascade_actions(DbObjectSelectorPolicy::default()),
            vec![super::CompareTargetCascadeAction::LoadTables]
        );
    }

    #[test]
    fn schema_clear_does_not_load_tables() {
        assert_eq!(super::schema_change_cascade_actions(false), Vec::new());
    }

    #[test]
    fn schema_selection_loads_tables() {
        assert_eq!(
            super::schema_change_cascade_actions(true),
            vec![super::CompareTargetCascadeAction::LoadTables]
        );
    }
}
