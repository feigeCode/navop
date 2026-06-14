use db::{DatabaseCapabilities, GlobalDbState};
use gpui::{App, AppContext, Context, Entity, Window};
use gpui_component::{
    IndexPath,
    input::InputState,
    select::{SearchableVec, SelectState},
};

use crate::compare::window_ui::ConnectionSelectItem;

pub(crate) type StringSelect = Entity<SelectState<SearchableVec<String>>>;

#[derive(Clone)]
pub(crate) struct DbObjectSelectorControls {
    pub connection: TargetConnectionControls,
    pub database: Option<TargetStringControls>,
    pub schema: Option<TargetStringControls>,
    pub table: Option<TargetStringControls>,
    pub column: Option<TargetStringControls>,
    pub policy: DbObjectSelectorPolicy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DbObjectSelectorPolicy {
    pub show_schema: bool,
    pub schema_as_database: bool,
}

impl DbObjectSelectorPolicy {
    pub fn generic() -> Self {
        Self {
            show_schema: true,
            schema_as_database: false,
        }
    }

    pub fn from_capabilities(capabilities: &DatabaseCapabilities) -> Self {
        Self {
            show_schema: capabilities.supports_schema && !capabilities.uses_schema_as_database,
            schema_as_database: capabilities.uses_schema_as_database,
        }
    }
}

pub(crate) fn policy_for_connection<T>(
    connection: &TargetConnectionControls,
    cx: &Context<T>,
) -> DbObjectSelectorPolicy {
    let Some(connection_id) = connection.select.read(cx).selected_value().cloned() else {
        return DbObjectSelectorPolicy::default();
    };
    let Some(db_state) = cx.try_global::<GlobalDbState>() else {
        return DbObjectSelectorPolicy::default();
    };
    let Some(config) = db_state.get_config(&connection_id) else {
        return DbObjectSelectorPolicy::default();
    };
    DbObjectSelectorPolicy::from_capabilities(&db_state.capabilities(&config.database_type))
}

pub(crate) fn effective_database_schema(
    database: String,
    schema: String,
    policy: DbObjectSelectorPolicy,
) -> (String, String) {
    if policy.schema_as_database {
        (database.clone(), database)
    } else if policy.show_schema {
        (database, schema)
    } else {
        (database, String::new())
    }
}

#[derive(Clone)]
pub(crate) struct TargetConnectionControls {
    pub select: Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
}

#[derive(Clone)]
pub(crate) struct TargetStringControls {
    pub select: StringSelect,
    pub fallback: Entity<InputState>,
}

pub(crate) fn string_select_state(
    initial_value: String,
    window: &mut Window,
    cx: &mut App,
) -> StringSelect {
    let selected = (!initial_value.is_empty()).then(|| IndexPath::new(0));
    let items = (!initial_value.is_empty())
        .then_some(vec![initial_value])
        .unwrap_or_default();
    cx.new(|cx| SelectState::new(SearchableVec::new(items), selected, window, cx).searchable(true))
}

pub(crate) fn selected_string(
    select: &StringSelect,
    fallback: &Entity<InputState>,
    cx: &App,
) -> String {
    select
        .read(cx)
        .selected_value()
        .cloned()
        .unwrap_or_else(|| fallback.read(cx).text().to_string())
}

pub(crate) fn set_connection_select<T>(
    select: &Entity<SelectState<SearchableVec<ConnectionSelectItem>>>,
    value: &str,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    select.update(cx, |state, cx| {
        state.set_selected_value(&value.to_string(), window, cx);
    });
}

pub(crate) fn set_string_select<T>(
    select: &StringSelect,
    fallback: &Entity<InputState>,
    value: String,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    fallback.update(cx, |input, cx| {
        input.set_value(value.clone(), window, cx);
    });
    select.update(cx, |state, cx| {
        state.set_selected_value(&value, window, cx);
    });
}

#[cfg(test)]
mod tests {
    use db::DatabaseCapabilities;

    use super::DbObjectSelectorPolicy;

    #[test]
    fn schema_policy_uses_database_capabilities() {
        let normal_schema = DatabaseCapabilities {
            supports_schema: true,
            ..DatabaseCapabilities::default()
        };
        let schema_as_database = DatabaseCapabilities {
            uses_schema_as_database: true,
            ..DatabaseCapabilities::default()
        };
        let no_schema = DatabaseCapabilities::default();

        assert_eq!(
            DbObjectSelectorPolicy {
                show_schema: true,
                schema_as_database: false,
            },
            DbObjectSelectorPolicy::from_capabilities(&normal_schema)
        );
        assert_eq!(
            DbObjectSelectorPolicy {
                show_schema: false,
                schema_as_database: true,
            },
            DbObjectSelectorPolicy::from_capabilities(&schema_as_database)
        );
        assert_eq!(
            DbObjectSelectorPolicy::default(),
            DbObjectSelectorPolicy::from_capabilities(&no_schema)
        );
    }

    #[test]
    fn schema_as_database_selection_uses_database_value_as_schema() {
        let policy = DbObjectSelectorPolicy {
            show_schema: false,
            schema_as_database: true,
        };

        assert_eq!(
            ("APP".to_string(), "APP".to_string()),
            super::effective_database_schema("APP".to_string(), String::new(), policy)
        );
    }

    #[test]
    fn no_schema_selection_clears_schema_value() {
        assert_eq!(
            ("app".to_string(), String::new()),
            super::effective_database_schema(
                "app".to_string(),
                "ignored".to_string(),
                DbObjectSelectorPolicy::default()
            )
        );
    }
}
