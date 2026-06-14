use extension_component::{DbSelectorKind, DbSelectorSource};
use rust_i18n::t;

use crate::db_object_selector::DbObjectSelectorPolicy;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SelectorPart {
    pub suffix: &'static str,
    pub label: String,
    pub value: Option<String>,
    pub kind: DbSelectorKind,
}

#[cfg(test)]
pub(crate) fn selector_parts(kind: &DbSelectorKind) -> Vec<SelectorPart> {
    selector_parts_with_policy(kind, DbObjectSelectorPolicy::generic())
}

pub(crate) fn selector_parts_with_policy(
    kind: &DbSelectorKind,
    policy: DbObjectSelectorPolicy,
) -> Vec<SelectorPart> {
    selector_kinds(kind, policy)
        .into_iter()
        .map(|kind| SelectorPart {
            suffix: selector_suffix(&kind),
            label: selector_label(&kind, policy),
            value: None,
            kind,
        })
        .collect()
}

pub(crate) fn selector_parts_for_source_with_policy(
    source: &DbSelectorSource,
    policy: DbObjectSelectorPolicy,
) -> Vec<SelectorPart> {
    selector_parts_with_policy(&source.kind, policy)
        .into_iter()
        .map(|mut part| {
            part.value = selector_query_value(source, &part.kind);
            part
        })
        .collect()
}

pub(crate) fn selector_source_part(
    base: &DbSelectorSource,
    part: &SelectorPart,
) -> DbSelectorSource {
    DbSelectorSource {
        kind: part.kind.clone(),
        query: base.query.clone(),
    }
}

pub(crate) fn selector_suffix(kind: &DbSelectorKind) -> &'static str {
    match kind {
        DbSelectorKind::Connection => "connection_id",
        DbSelectorKind::Database => "database",
        DbSelectorKind::Schema => "schema",
        DbSelectorKind::Table => "table",
        DbSelectorKind::Column => "column",
    }
}

pub(crate) fn selector_includes(target: &DbSelectorKind, part: DbSelectorKind) -> bool {
    selector_depth(target) >= selector_depth(&part)
}

fn selector_kinds(kind: &DbSelectorKind, policy: DbObjectSelectorPolicy) -> Vec<DbSelectorKind> {
    [
        DbSelectorKind::Connection,
        DbSelectorKind::Database,
        DbSelectorKind::Schema,
        DbSelectorKind::Table,
        DbSelectorKind::Column,
    ]
    .into_iter()
    .filter(|part| selector_includes(kind, part.clone()))
    .filter(|part| *part != DbSelectorKind::Schema || policy.show_schema)
    .collect()
}

fn selector_depth(kind: &DbSelectorKind) -> usize {
    match kind {
        DbSelectorKind::Connection => 0,
        DbSelectorKind::Database => 1,
        DbSelectorKind::Schema => 2,
        DbSelectorKind::Table => 3,
        DbSelectorKind::Column => 4,
    }
}

fn selector_label(kind: &DbSelectorKind, policy: DbObjectSelectorPolicy) -> String {
    if *kind == DbSelectorKind::Database && policy.schema_as_database {
        return t!("DbObjectSelector.schema").to_string();
    }
    match kind {
        DbSelectorKind::Connection => t!("DbObjectSelector.connection").to_string(),
        DbSelectorKind::Database => t!("DbObjectSelector.database").to_string(),
        DbSelectorKind::Schema => t!("DbObjectSelector.schema").to_string(),
        DbSelectorKind::Table => t!("DbObjectSelector.table").to_string(),
        DbSelectorKind::Column => t!("DbObjectSelector.column").to_string(),
    }
}

fn selector_query_value(source: &DbSelectorSource, kind: &DbSelectorKind) -> Option<String> {
    match kind {
        DbSelectorKind::Connection => source.query.connection_id.clone(),
        DbSelectorKind::Database => source.query.database.clone(),
        DbSelectorKind::Schema => source.query.schema.clone(),
        DbSelectorKind::Table => source.query.table.clone(),
        DbSelectorKind::Column => None,
    }
}
