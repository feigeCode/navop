use std::collections::BTreeMap;

use db::{DbNodeType, GlobalDbState};
use extension_component::{
    DbSelectorKind, DbSelectorSource, PermissionSet, SelectOption, SqlAccess, ViewSpec,
};

use crate::db_object_selector::DbObjectSelectorPolicy;
use crate::extension_menu::DbTreeExtensionActionContext;
use crate::extension_widget::ExtensionWidgetSelectorPolicies;

mod parts;

use parts::{
    db_selector_policies, db_selector_sources_with_policies, selector_database, selector_schema,
};

pub type ExtensionSelectorOptions = BTreeMap<String, Vec<SelectOption>>;
pub type ExtensionSelectorPolicies = ExtensionWidgetSelectorPolicies;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionSelectorData {
    pub options: ExtensionSelectorOptions,
    pub policies: ExtensionSelectorPolicies,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorRequest {
    pub connection_id: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub table: Option<String>,
}

pub async fn load_extension_selector_options(
    spec: &ViewSpec,
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    context: &DbTreeExtensionActionContext,
) -> ExtensionSelectorOptions {
    load_extension_selector_data(spec, db_state, permissions, context)
        .await
        .options
}

pub async fn load_extension_selector_data(
    spec: &ViewSpec,
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    context: &DbTreeExtensionActionContext,
) -> ExtensionSelectorData {
    let mut options = BTreeMap::new();
    let policies = db_selector_policies(spec, db_state, context);
    for (field_id, source) in db_selector_sources_with_policies(spec, &policies) {
        let values = load_selector_options(&source, db_state, permissions, context).await;
        options.insert(field_id, values);
    }
    ExtensionSelectorData { options, policies }
}

pub fn selector_request_for_source(
    source: &DbSelectorSource,
    context: &DbTreeExtensionActionContext,
) -> SelectorRequest {
    SelectorRequest {
        connection_id: source
            .query
            .connection_id
            .clone()
            .or_else(|| Some(context.connection_id.clone())),
        database: source.query.database.clone().or_else(|| {
            (context.node_type == DbNodeType::Database).then(|| context.node_name.clone())
        }),
        schema: source.query.schema.clone().or_else(|| {
            (context.node_type == DbNodeType::Schema).then(|| context.node_name.clone())
        }),
        table: source.query.table.clone().or_else(|| {
            (context.node_type == DbNodeType::Table).then(|| context.node_name.clone())
        }),
    }
}

async fn load_selector_options(
    source: &DbSelectorSource,
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    context: &DbTreeExtensionActionContext,
) -> Vec<SelectOption> {
    let request = selector_request_for_source(source, context);
    let policy = request_policy(db_state, &request);
    match source.kind {
        DbSelectorKind::Connection => connection_options(db_state, permissions),
        DbSelectorKind::Database => database_options(db_state, permissions, request, policy).await,
        DbSelectorKind::Schema => schema_options(db_state, permissions, request, policy).await,
        DbSelectorKind::Table => table_options(db_state, permissions, request, policy).await,
        DbSelectorKind::Column => column_options(db_state, permissions, request, policy).await,
    }
}

fn connection_options(db_state: &GlobalDbState, permissions: &PermissionSet) -> Vec<SelectOption> {
    if !permissions.allows_connection_list() {
        return Vec::new();
    }
    db_state
        .list_connection_summaries()
        .into_iter()
        .map(|connection| SelectOption {
            value: connection.id,
            label: connection.name,
        })
        .collect()
}

async fn database_options(
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    request: SelectorRequest,
    policy: DbObjectSelectorPolicy,
) -> Vec<SelectOption> {
    let Some(connection_id) = allowed_connection_id(permissions, request) else {
        return Vec::new();
    };
    if policy.schema_as_database {
        db_state
            .list_schemas_direct(connection_id, String::new())
            .await
            .unwrap_or_default()
            .into_iter()
            .map(named_option)
            .collect()
    } else {
        db_state
            .list_databases_direct(connection_id)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(named_option)
            .collect()
    }
}

async fn schema_options(
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    request: SelectorRequest,
    policy: DbObjectSelectorPolicy,
) -> Vec<SelectOption> {
    if !policy.show_schema {
        return Vec::new();
    }
    let Some(connection_id) = allowed_connection_id(permissions, request.clone()) else {
        return Vec::new();
    };
    let Some(database) = request.database else {
        return Vec::new();
    };
    db_state
        .list_schemas_direct(connection_id, database)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(named_option)
        .collect()
}

async fn table_options(
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    request: SelectorRequest,
    policy: DbObjectSelectorPolicy,
) -> Vec<SelectOption> {
    let Some(connection_id) = allowed_connection_id(permissions, request.clone()) else {
        return Vec::new();
    };
    let Some(database) = selector_database(&request, policy) else {
        return Vec::new();
    };
    let schema = selector_schema(&request, policy);
    db_state
        .list_tables_direct(&connection_id, &database, schema)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|table| named_option(table.name))
        .collect()
}

async fn column_options(
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    request: SelectorRequest,
    policy: DbObjectSelectorPolicy,
) -> Vec<SelectOption> {
    let Some(connection_id) = allowed_connection_id(permissions, request.clone()) else {
        return Vec::new();
    };
    let (Some(database), Some(table)) =
        (selector_database(&request, policy), request.table.clone())
    else {
        return Vec::new();
    };
    let schema = selector_schema(&request, policy);
    db_state
        .list_columns_direct(&connection_id, &database, schema, &table)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|column| SelectOption {
            value: column.name.clone(),
            label: format!("{} ({})", column.name, column.data_type),
        })
        .collect()
}

fn allowed_connection_id(permissions: &PermissionSet, request: SelectorRequest) -> Option<String> {
    let connection_id = request.connection_id?;
    permissions
        .allows_db(SqlAccess::Schema, &connection_id)
        .then_some(connection_id)
}

fn request_policy(db_state: &GlobalDbState, request: &SelectorRequest) -> DbObjectSelectorPolicy {
    let Some(connection_id) = request.connection_id.as_deref() else {
        return DbObjectSelectorPolicy::default();
    };
    let Some(config) = db_state.get_config(connection_id) else {
        return DbObjectSelectorPolicy::default();
    };
    DbObjectSelectorPolicy::from_capabilities(&db_state.capabilities(&config.database_type))
}

fn named_option(name: String) -> SelectOption {
    SelectOption {
        value: name.clone(),
        label: name,
    }
}

#[cfg(test)]
mod tests;
