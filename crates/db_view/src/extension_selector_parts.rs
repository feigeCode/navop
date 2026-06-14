use extension_component::{DbSelectorKind, DbSelectorSource};

#[derive(Clone)]
pub(crate) struct SelectorPart {
    pub suffix: &'static str,
    pub label: &'static str,
    pub value: Option<String>,
    pub source: DbSelectorSource,
}

pub(crate) fn selector_parts(source: &DbSelectorSource) -> Vec<SelectorPart> {
    let mut parts = Vec::new();
    push_selector_part(&mut parts, source, DbSelectorKind::Connection);
    if selector_includes(&source.kind, DbSelectorKind::Database) {
        push_selector_part(&mut parts, source, DbSelectorKind::Database);
    }
    if selector_includes(&source.kind, DbSelectorKind::Schema) {
        push_selector_part(&mut parts, source, DbSelectorKind::Schema);
    }
    if selector_includes(&source.kind, DbSelectorKind::Table) {
        push_selector_part(&mut parts, source, DbSelectorKind::Table);
    }
    if selector_includes(&source.kind, DbSelectorKind::Column) {
        push_selector_part(&mut parts, source, DbSelectorKind::Column);
    }
    parts
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

fn push_selector_part(
    parts: &mut Vec<SelectorPart>,
    base: &DbSelectorSource,
    kind: DbSelectorKind,
) {
    parts.push(SelectorPart {
        suffix: selector_suffix(&kind),
        label: selector_label(&kind),
        value: selector_query_value(base, &kind),
        source: DbSelectorSource {
            kind,
            query: base.query.clone(),
        },
    });
}

fn selector_includes(target: &DbSelectorKind, part: DbSelectorKind) -> bool {
    selector_depth(target) >= selector_depth(&part)
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

fn selector_label(kind: &DbSelectorKind) -> &'static str {
    match kind {
        DbSelectorKind::Connection => "连接",
        DbSelectorKind::Database => "数据库",
        DbSelectorKind::Schema => "Schema",
        DbSelectorKind::Table => "表",
        DbSelectorKind::Column => "字段",
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
