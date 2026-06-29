use ai_chat_view::{MentionItem, ResourceContext, ResourceId, ResourceScope};
use db::{GlobalDbState, TableInfo};
use gpui::{AsyncApp, Context};

use super::DatabaseSidebar;

pub(super) struct TableMentionLoad {
    pub(super) seq: usize,
    pub(super) connection_id: String,
    pub(super) database: String,
    pub(super) schema: Option<String>,
    pub(super) resources: ResourceContext,
    pub(super) mentions: Vec<MentionItem>,
}

pub(super) struct TableMentionLoadParts {
    pub(super) seq: usize,
    pub(super) connection_id: String,
    pub(super) database: Option<String>,
    pub(super) schema: Option<String>,
    pub(super) resources: ResourceContext,
    pub(super) mentions: Vec<MentionItem>,
}

pub(super) struct SelectedDatabaseScope<'a> {
    pub(super) database: Option<&'a str>,
    pub(super) schema: Option<&'a str>,
}

impl TableMentionLoad {
    pub(super) fn new(parts: TableMentionLoadParts) -> Option<Self> {
        let database = parts.database.filter(|value| !value.is_empty())?;
        Some(Self {
            seq: parts.seq,
            connection_id: parts.connection_id,
            database,
            schema: parts.schema,
            resources: parts.resources,
            mentions: parts.mentions,
        })
    }

    pub(super) fn append_tables(&mut self, tables: Vec<TableInfo>) {
        let scope = TableMentionScope {
            connection_id: &self.connection_id,
            database: &self.database,
            schema: self.schema.as_deref(),
        };
        append_table_mentions(&mut self.mentions, &scope, tables);
    }
}

impl DatabaseSidebar {
    pub(super) fn load_table_mentions(
        &self,
        load: Option<TableMentionLoad>,
        cx: &mut Context<Self>,
    ) {
        let Some(mut load) = load else {
            return;
        };
        let connection_id = load.connection_id.clone();
        let database = load.database.clone();
        let schema = load.schema.clone();
        let db_state = cx.global::<GlobalDbState>().clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            match db_state
                .list_tables(cx, connection_id, database, schema)
                .await
            {
                Ok(tables) => {
                    load.append_tables(tables);
                    let _ = this.update(cx, |sidebar, cx| {
                        if sidebar.table_context_seq == load.seq {
                            sidebar.chat_panel.update(cx, |panel, cx| {
                                panel.set_resource_context(load.resources, load.mentions, cx);
                            });
                            cx.notify();
                        }
                    });
                }
                Err(error) => {
                    tracing::warn!(%error, "Failed to load AI chat table mentions");
                }
            }
        })
        .detach();
    }
}

struct TableMentionScope<'a> {
    connection_id: &'a str,
    database: &'a str,
    schema: Option<&'a str>,
}

pub(super) fn apply_database_scope(
    resources: &mut ResourceContext,
    resource_id: &ResourceId,
    scope: SelectedDatabaseScope<'_>,
) {
    let Some(resource) = resources.get_mut(resource_id) else {
        return;
    };
    if let Some(database) = scope.database.filter(|value| !value.is_empty()) {
        resource.set_scope(ResourceScope::new("database", "Database", database));
    }
    if let Some(schema) = scope.schema.filter(|value| !value.is_empty()) {
        resource.set_scope(ResourceScope::new("schema", "Schema", schema));
    }
}

fn append_table_mentions(
    mentions: &mut Vec<MentionItem>,
    scope: &TableMentionScope<'_>,
    tables: Vec<TableInfo>,
) {
    for table in tables {
        mentions.push(table_mention(scope, table));
    }
}

fn table_mention(scope: &TableMentionScope<'_>, table: TableInfo) -> MentionItem {
    let scope_label = scope
        .schema
        .filter(|value| !value.is_empty())
        .map(|schema| format!("{}.{schema}", scope.database))
        .unwrap_or_else(|| scope.database.to_string());
    let detail = table
        .comment
        .filter(|comment| !comment.is_empty())
        .map(|comment| format!("表 | {scope_label} | {comment}"))
        .unwrap_or_else(|| format!("表 | {scope_label}"));
    let id = scope
        .schema
        .filter(|value| !value.is_empty())
        .map(|schema| {
            format!(
                "table:{}:{}:{schema}:{}",
                scope.connection_id, scope.database, table.name
            )
        })
        .unwrap_or_else(|| {
            format!(
                "table:{}:{}:{}",
                scope.connection_id, scope.database, table.name
            )
        });
    MentionItem::new(id, table.name, detail, "table")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(name: &str, comment: Option<&str>) -> TableInfo {
        TableInfo {
            name: name.to_string(),
            schema: None,
            comment: comment.map(ToString::to_string),
            engine: None,
            row_count: None,
            create_time: None,
            charset: None,
            collation: None,
        }
    }

    #[test]
    fn table_load_adds_table_mentions_without_database_mentions() {
        let mut load = TableMentionLoad::new(TableMentionLoadParts {
            seq: 1,
            connection_id: "7".to_string(),
            database: Some("analytics".to_string()),
            schema: Some("public".to_string()),
            resources: ResourceContext::new(),
            mentions: vec![MentionItem::new("7", "prod", "mysql | 127.0.0.1", "mysql")],
        })
        .expect("database should create a table mention load");

        load.append_tables(vec![table("users", Some("user rows"))]);

        assert_eq!(2, load.mentions.len());
        assert!(load.mentions.iter().all(|item| item.label != "analytics"));
        assert_eq!("users", load.mentions[1].label);
        assert_eq!("table", load.mentions[1].kind);
        assert_eq!("表 | analytics.public | user rows", load.mentions[1].detail);
        assert_eq!("table:7:analytics:public:users", load.mentions[1].id);
    }

    #[test]
    fn table_load_is_absent_when_database_is_empty() {
        assert!(
            TableMentionLoad::new(TableMentionLoadParts {
                seq: 1,
                connection_id: "7".to_string(),
                database: Some(String::new()),
                schema: None,
                resources: ResourceContext::new(),
                mentions: Vec::new(),
            },)
            .is_none()
        );
    }
}
