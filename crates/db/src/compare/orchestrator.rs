use std::collections::HashSet;

use gpui::AsyncApp;
use one_core::gpui_tokio::Tokio;

use crate::{
    ColumnInfo, DirectTableMetadataRequest, ForeignKeyDefinition, FunctionInfo, GlobalDbState,
    IndexInfo, TableInfo, TableObjectType, TriggerInfo,
};

use super::{
    ColumnSchema, CompareSchemaSide, CompareTaskEvent, ForeignKeySchema, IndexSchema, RoutineKind,
    RoutineSchema, SchemaCompareOptions, SchemaCompareParams, SchemaCompareResult,
    SchemaCompareTableFailure, SchemaObjectType, SchemaSyncPlanOptions, SchemaTypeMappingContext,
    SyncPlan, TableSchema, TriggerSchema, compare_routines, compare_schemas_with_type_mapping,
    compare_triggers, identifier_key,
};

#[derive(Debug, Default)]
struct LoadedSchemaTables {
    schemas: Vec<TableSchema>,
    failures: Vec<SchemaCompareTableFailure>,
}

fn record_schema_table_result(
    loaded: &mut LoadedSchemaTables,
    side: CompareSchemaSide,
    table: String,
    result: anyhow::Result<TableSchema>,
) -> Option<String> {
    match result {
        Ok(schema) => {
            loaded.schemas.push(schema);
            None
        }
        Err(error) => {
            let error = format!("{error:#}");
            loaded.failures.push(SchemaCompareTableFailure {
                side,
                table,
                error: error.clone(),
            });
            Some(error)
        }
    }
}

impl GlobalDbState {
    /// Builds the schema synchronization plan using the target connection's
    /// dialect and the compare-column-order policy selected by the caller.
    pub fn prepare_schema_sync_plan_for_target(
        &self,
        result: &SchemaCompareResult,
        source_connection_id: &str,
        target_connection_id: &str,
        target_database: &str,
        target_schema: Option<&str>,
        compare_column_order: bool,
        type_mapping_overrides: super::TypeMappingOverrides,
    ) -> anyhow::Result<SyncPlan> {
        self.prepare_schema_sync_plan_for_target_with_source_namespace(
            result,
            source_connection_id,
            target_connection_id,
            target_database,
            target_schema,
            None,
            None,
            compare_column_order,
            type_mapping_overrides,
        )
    }

    /// Builds a schema synchronization plan while preserving the source
    /// database/schema context needed to remap source-qualified foreign keys.
    pub fn prepare_schema_sync_plan_for_target_with_source_namespace(
        &self,
        result: &SchemaCompareResult,
        source_connection_id: &str,
        target_connection_id: &str,
        target_database: &str,
        target_schema: Option<&str>,
        source_database: Option<&str>,
        source_schema: Option<&str>,
        compare_column_order: bool,
        type_mapping_overrides: super::TypeMappingOverrides,
    ) -> anyhow::Result<SyncPlan> {
        if result.has_failed_tables() {
            return Ok(super::blocked_schema_sync_plan(result));
        }

        let source_config = self
            .get_config(source_connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {source_connection_id}"))?;
        let target_config = self
            .get_config(target_connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {target_connection_id}"))?;
        let plugin = self.get_plugin(&target_config.database_type)?;

        Ok(
            super::build_schema_sync_plan_with_plugin_options_for_source_namespace(
                result,
                target_database,
                target_schema,
                source_database,
                source_schema,
                &source_config.database_type,
                plugin.as_ref(),
                SchemaSyncPlanOptions {
                    compare_column_order,
                    type_mapping_overrides,
                },
            ),
        )
    }

    /// Loads both schema targets and computes their structural diff.
    ///
    /// Database metadata loading and conversion intentionally live in `db` so
    /// UI clients only provide parameters and translate structured progress
    /// events for presentation.
    pub async fn prepare_schema_compare_from_targets(
        &self,
        cx: &mut AsyncApp,
        params: SchemaCompareParams,
        mut report: impl FnMut(CompareTaskEvent),
    ) -> anyhow::Result<SchemaCompareResult> {
        let SchemaCompareParams {
            source_connection_id,
            source_database,
            source_schema,
            source_tables,
            target_connection_id,
            target_database,
            target_schema,
            target_tables,
            case_sensitive_identifiers,
            compare_views,
            compare_routines: compare_routines_enabled,
            compare_triggers: compare_triggers_enabled,
            compare_indexes,
            compare_foreign_keys,
            ignore_comments,
            ignore_auto_increment,
            ignore_charset_collation,
            ignore_table_options,
            compare_column_order,
            type_mapping_overrides,
        } = params;
        let options = SchemaCompareOptions {
            case_sensitive_identifiers,
            compare_indexes,
            compare_foreign_keys,
            ignore_comments,
            ignore_auto_increment,
            ignore_charset_collation,
            ignore_table_options,
            compare_column_order,
            ..SchemaCompareOptions::default()
        };
        let source_database_type = self
            .get_config(&source_connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {source_connection_id}"))?
            .database_type
            .clone();
        let target_database_type = self
            .get_config(&target_connection_id)
            .ok_or_else(|| anyhow::anyhow!("Connection not found: {target_connection_id}"))?
            .database_type
            .clone();

        let source = load_schema_tables(
            self,
            cx,
            source_connection_id.clone(),
            source_database.clone(),
            source_schema.clone(),
            &source_tables,
            case_sensitive_identifiers,
            compare_views,
            CompareSchemaSide::Source,
            &mut report,
        )
        .await?;
        let target = load_schema_tables(
            self,
            cx,
            target_connection_id.clone(),
            target_database.clone(),
            target_schema.clone(),
            &target_tables,
            case_sensitive_identifiers,
            compare_views,
            CompareSchemaSide::Target,
            &mut report,
        )
        .await?;

        report(CompareTaskEvent::ComparingSchema);
        let mut result = compare_schemas_with_type_mapping(
            source.schemas,
            target.schemas,
            options.clone(),
            Some(if type_mapping_overrides.overrides.is_empty() {
                SchemaTypeMappingContext::new(&source_database_type, &target_database_type)
            } else {
                SchemaTypeMappingContext::with_overrides(
                    &source_database_type,
                    &target_database_type,
                    &type_mapping_overrides,
                )
            }),
        )?;
        if compare_routines_enabled {
            let source_routines = load_schema_routines(
                self,
                cx,
                &source_connection_id,
                &source_database,
                source_schema.as_deref(),
                case_sensitive_identifiers,
            )
            .await?;
            let target_routines = load_schema_routines(
                self,
                cx,
                &target_connection_id,
                &target_database,
                target_schema.as_deref(),
                case_sensitive_identifiers,
            )
            .await?;
            result.routine_diffs = compare_routines(source_routines, target_routines, &options)?;
        }
        if compare_triggers_enabled {
            let source_triggers = load_schema_triggers(
                self,
                cx,
                &source_connection_id,
                &source_database,
                source_schema.as_deref(),
            )
            .await?;
            let target_triggers = load_schema_triggers(
                self,
                cx,
                &target_connection_id,
                &target_database,
                target_schema.as_deref(),
            )
            .await?;
            result.trigger_diffs = compare_triggers(source_triggers, target_triggers, &options)?;
        }
        result.table_failures = source.failures.into_iter().chain(target.failures).collect();
        result.refresh_counts();
        Ok(result)
    }
}

async fn load_schema_routines(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    schema: Option<&str>,
    case_sensitive_identifiers: bool,
) -> anyhow::Result<Vec<RoutineSchema>> {
    let db_state = db_state.clone();
    let connection_id = connection_id.to_string();
    let database = database.to_string();
    let fallback_schema = schema.map(str::to_string);
    let query_schema = fallback_schema.clone();
    let (functions, procedures) = Tokio::spawn_result(cx, async move {
        let functions = db_state
            .list_functions_in_schema_direct(&connection_id, &database, query_schema.clone())
            .await?;
        let procedures = db_state
            .list_procedures_in_schema_direct(&connection_id, &database, query_schema)
            .await?;
        Ok((functions, procedures))
    })
    .await?;

    Ok(functions
        .into_iter()
        .filter_map(|routine| {
            routine_schema_from_metadata(
                RoutineKind::Function,
                routine,
                fallback_schema.as_deref(),
                case_sensitive_identifiers,
            )
        })
        .chain(procedures.into_iter().filter_map(|routine| {
            routine_schema_from_metadata(
                RoutineKind::Procedure,
                routine,
                fallback_schema.as_deref(),
                case_sensitive_identifiers,
            )
        }))
        .collect())
}

async fn load_schema_triggers(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    schema: Option<&str>,
) -> anyhow::Result<Vec<TriggerSchema>> {
    let db_state = db_state.clone();
    let connection_id = connection_id.to_string();
    let database = database.to_string();
    let fallback_schema = schema.map(str::to_string);
    let query_schema = fallback_schema.clone();
    Ok(Tokio::spawn_result(cx, async move {
        db_state
            .list_triggers_in_schema_direct(&connection_id, &database, query_schema)
            .await
    })
    .await?
    .into_iter()
    .map(|trigger| trigger_schema_from_metadata(trigger, fallback_schema.as_deref()))
    .collect())
}

fn routine_schema_from_metadata(
    kind: RoutineKind,
    routine: FunctionInfo,
    fallback_schema: Option<&str>,
    case_sensitive_identifiers: bool,
) -> Option<RoutineSchema> {
    if let (Some(expected), Some(actual)) = (fallback_schema, routine.schema.as_deref()) {
        if identifier_key(expected, case_sensitive_identifiers)
            != identifier_key(actual, case_sensitive_identifiers)
        {
            return None;
        }
    }

    Some(RoutineSchema {
        kind,
        name: routine.name,
        schema: routine
            .schema
            .or_else(|| fallback_schema.map(str::to_string)),
        identity_arguments: routine.identity_arguments,
        object_id: routine.object_id,
        return_type: routine.return_type,
        parameters: routine.parameters,
        definition: routine.definition,
        comment: routine.comment,
    })
}

fn trigger_schema_from_metadata(
    trigger: TriggerInfo,
    fallback_schema: Option<&str>,
) -> TriggerSchema {
    TriggerSchema {
        name: trigger.name,
        schema: fallback_schema.map(str::to_string),
        table_name: trigger.table_name,
        event: trigger.event,
        timing: trigger.timing,
        definition: trigger.definition,
    }
}

async fn load_schema_tables(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: String,
    database: String,
    schema: Option<String>,
    selected_tables: &[String],
    case_sensitive_identifiers: bool,
    compare_views: bool,
    side: CompareSchemaSide,
    report: &mut impl FnMut(CompareTaskEvent),
) -> anyhow::Result<LoadedSchemaTables> {
    report(CompareTaskEvent::LoadingTableList { side });
    let query_state = db_state.clone();
    let query_connection_id = connection_id.clone();
    let query_database = database.clone();
    let query_schema = schema.clone();
    let tables = Tokio::spawn_result(cx, async move {
        query_state
            .list_tables_direct(&query_connection_id, &query_database, query_schema)
            .await
    })
    .await?;
    let tables = filter_schema_tables(
        tables,
        selected_tables,
        case_sensitive_identifiers,
        compare_views,
    );
    let total_tables = tables.len();
    let mut loaded = LoadedSchemaTables {
        schemas: Vec::with_capacity(total_tables),
        failures: Vec::new(),
    };

    for (index, table) in tables.into_iter().enumerate() {
        let table_name = table.name.clone();
        report(CompareTaskEvent::LoadingTableSchema {
            side,
            table: table_name.clone(),
            table_index: index + 1,
            total_tables,
        });
        let result = load_single_table_schema(
            db_state,
            cx,
            &connection_id,
            &database,
            schema.clone(),
            table,
        )
        .await;
        if let Some(message) =
            record_schema_table_result(&mut loaded, side, table_name.clone(), result)
        {
            report(CompareTaskEvent::Error {
                table: Some(table_name),
                message,
            });
        }
    }

    Ok(loaded)
}

fn filter_schema_tables(
    tables: Vec<TableInfo>,
    selected_tables: &[String],
    case_sensitive_identifiers: bool,
    compare_views: bool,
) -> Vec<TableInfo> {
    let selected = selected_tables
        .iter()
        .map(|table| identifier_key(table, case_sensitive_identifiers))
        .collect::<HashSet<_>>();
    tables
        .into_iter()
        .filter(|table| {
            if table.object_type == TableObjectType::View {
                return compare_views;
            }
            selected_tables.is_empty()
                || selected.contains(&identifier_key(&table.name, case_sensitive_identifiers))
        })
        .collect()
}

async fn load_single_table_schema(
    db_state: &GlobalDbState,
    cx: &mut AsyncApp,
    connection_id: &str,
    database: &str,
    schema: Option<String>,
    table: TableInfo,
) -> anyhow::Result<TableSchema> {
    let table_name = table.name.clone();
    let query_state = db_state.clone();
    let connection_id = connection_id.to_string();
    let database = database.to_string();
    let request = DirectTableMetadataRequest {
        connection_id,
        database,
        schema,
        table: table_name,
        include_table_metadata: table.object_type != TableObjectType::View,
    };
    let metadata = Tokio::spawn_result(cx, async move {
        query_state.load_table_metadata_direct(request).await
    })
    .await?;

    Ok(table_schema_from_metadata(
        table,
        metadata.columns,
        metadata.indexes,
        metadata.foreign_keys,
    ))
}

pub fn table_schema_from_columns(table_name: &str, columns: &[ColumnInfo]) -> TableSchema {
    table_schema_from_metadata(
        TableInfo {
            name: table_name.to_string(),
            object_type: TableObjectType::Table,
            schema: None,
            comment: None,
            engine: None,
            create_time: None,
            charset: None,
            collation: None,
        },
        columns.to_vec(),
        Vec::new(),
        Vec::new(),
    )
}

pub fn table_schema_from_metadata(
    table: TableInfo,
    columns: Vec<ColumnInfo>,
    indexes: Vec<IndexInfo>,
    foreign_keys: Vec<ForeignKeyDefinition>,
) -> TableSchema {
    let primary_columns = columns
        .iter()
        .filter(|column| column.is_primary_key)
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let mut indexes = indexes
        .into_iter()
        .map(|index| IndexSchema {
            name: index.name,
            columns: index.columns,
            unique: index.is_unique,
        })
        .collect::<Vec<_>>();
    if !primary_columns.is_empty()
        && !indexes
            .iter()
            .any(|index| index.name.eq_ignore_ascii_case("PRIMARY"))
    {
        indexes.push(IndexSchema {
            name: "PRIMARY".to_string(),
            columns: primary_columns,
            unique: true,
        });
    }

    TableSchema {
        name: table.name,
        object_type: match table.object_type {
            TableObjectType::Table => SchemaObjectType::Table,
            TableObjectType::View => SchemaObjectType::View,
        },
        columns: columns
            .into_iter()
            .map(|column| ColumnSchema {
                name: column.name,
                data_type: column.data_type,
                nullable: column.is_nullable,
                default_value: column.default_value,
                comment: column.comment,
                charset: column.charset,
                collation: column.collation,
            })
            .collect(),
        indexes,
        foreign_keys: foreign_keys
            .into_iter()
            .map(|foreign_key| ForeignKeySchema {
                name: foreign_key.name,
                columns: foreign_key.columns,
                ref_table: foreign_key.ref_table,
                ref_schema: foreign_key.ref_schema,
                ref_columns: foreign_key.ref_columns,
                on_delete: non_empty_string(foreign_key.on_delete),
                on_update: non_empty_string(foreign_key.on_update),
            })
            .collect(),
        comment: table.comment,
        engine: table.engine,
        charset: table.charset,
        collation: table.collation,
    }
}

fn non_empty_string(value: String) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_schema_from_metadata_preserves_columns_indexes_and_foreign_keys() {
        let table = TableInfo {
            name: "orders".to_string(),
            object_type: TableObjectType::Table,
            schema: Some("public".to_string()),
            comment: Some("order table".to_string()),
            engine: None,

            create_time: None,
            charset: None,
            collation: None,
        };
        let columns = vec![ColumnInfo {
            name: "id".to_string(),
            data_type: "int".to_string(),
            is_nullable: false,
            is_primary_key: true,
            default_value: None,
            comment: None,
            charset: None,
            collation: None,
        }];
        let indexes = vec![IndexInfo {
            name: "idx_orders_id".to_string(),
            columns: vec!["id".to_string()],
            is_unique: true,
            is_primary: false,
            index_type: None,
        }];
        let foreign_keys = vec![ForeignKeyDefinition {
            name: "fk_orders_user".to_string(),
            columns: vec!["user_id".to_string()],
            ref_table: "users".to_string(),
            ref_schema: None,
            ref_columns: vec!["id".to_string()],
            on_delete: "CASCADE".to_string(),
            on_update: "NO ACTION".to_string(),
        }];

        let schema = table_schema_from_metadata(table, columns, indexes, foreign_keys);

        assert_eq!(schema.name, "orders");
        assert_eq!(schema.comment.as_deref(), Some("order table"));
        assert_eq!(schema.columns[0].name, "id");
        assert!(!schema.columns[0].nullable);
        assert_eq!(schema.indexes[0].name, "idx_orders_id");
        assert!(schema.indexes[0].unique);
        assert!(schema.indexes.iter().any(|index| index.name == "PRIMARY"
            && index.columns == vec!["id".to_string()]
            && index.unique));
        assert_eq!(schema.foreign_keys[0].name, "fk_orders_user");
        assert_eq!(schema.foreign_keys[0].on_delete.as_deref(), Some("CASCADE"));
        assert_eq!(
            schema.foreign_keys[0].on_update.as_deref(),
            Some("NO ACTION")
        );
    }

    #[test]
    fn table_schema_from_metadata_preserves_view_kind() {
        let schema = table_schema_from_metadata(
            TableInfo {
                name: "active_users".to_string(),
                object_type: TableObjectType::View,
                schema: Some("public".to_string()),
                comment: None,
                engine: None,

                create_time: None,
                charset: None,
                collation: None,
            },
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );

        assert_eq!(schema.object_type, SchemaObjectType::View);
    }

    #[test]
    fn filter_schema_tables_excludes_views_until_selected() {
        let objects = vec![
            TableInfo {
                name: "users".to_string(),
                object_type: TableObjectType::Table,
                schema: None,
                comment: None,
                engine: None,

                create_time: None,
                charset: None,
                collation: None,
            },
            TableInfo {
                name: "active_users".to_string(),
                object_type: TableObjectType::View,
                schema: None,
                comment: None,
                engine: None,

                create_time: None,
                charset: None,
                collation: None,
            },
        ];

        let tables_only = filter_schema_tables(objects.clone(), &[], false, false);
        assert_eq!(
            tables_only
                .iter()
                .map(|table| table.name.as_str())
                .collect::<Vec<_>>(),
            vec!["users"]
        );

        let with_views = filter_schema_tables(objects, &[], false, true);
        assert_eq!(
            with_views
                .iter()
                .map(|table| table.name.as_str())
                .collect::<Vec<_>>(),
            vec!["users", "active_users"]
        );
    }

    #[test]
    fn filter_schema_tables_applies_source_table_selection_without_hiding_views() {
        let objects = vec![
            TableInfo {
                name: "users".to_string(),
                object_type: TableObjectType::Table,
                schema: None,
                comment: None,
                engine: None,

                create_time: None,
                charset: None,
                collation: None,
            },
            TableInfo {
                name: "orders".to_string(),
                object_type: TableObjectType::Table,
                schema: None,
                comment: None,
                engine: None,

                create_time: None,
                charset: None,
                collation: None,
            },
            TableInfo {
                name: "active_users".to_string(),
                object_type: TableObjectType::View,
                schema: None,
                comment: None,
                engine: None,

                create_time: None,
                charset: None,
                collation: None,
            },
        ];

        let filtered = filter_schema_tables(objects, &["Users".to_string()], false, true);

        assert_eq!(
            filtered
                .iter()
                .map(|table| table.name.as_str())
                .collect::<Vec<_>>(),
            vec!["users", "active_users"]
        );
    }

    #[test]
    fn record_schema_table_result_keeps_successes_and_isolates_failures() {
        let mut loaded = LoadedSchemaTables::default();
        let first = TableSchema {
            name: "first".to_string(),
            ..Default::default()
        };
        let last = TableSchema {
            name: "last".to_string(),
            ..Default::default()
        };

        assert_eq!(
            record_schema_table_result(
                &mut loaded,
                CompareSchemaSide::Source,
                first.name.clone(),
                Ok(first),
            ),
            None
        );
        let error = record_schema_table_result(
            &mut loaded,
            CompareSchemaSide::Source,
            "broken".to_string(),
            Err(anyhow::anyhow!("column metadata unavailable")),
        );
        assert_eq!(error.as_deref(), Some("column metadata unavailable"));
        assert_eq!(
            record_schema_table_result(
                &mut loaded,
                CompareSchemaSide::Source,
                last.name.clone(),
                Ok(last),
            ),
            None
        );

        assert_eq!(
            loaded
                .schemas
                .iter()
                .map(|schema| schema.name.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "last"]
        );
        assert_eq!(
            loaded.failures,
            vec![SchemaCompareTableFailure {
                side: CompareSchemaSide::Source,
                table: "broken".to_string(),
                error: "column metadata unavailable".to_string(),
            }]
        );
    }

    #[test]
    fn routine_metadata_uses_fallback_schema_and_preserves_kind() {
        let routine = routine_schema_from_metadata(
            RoutineKind::Procedure,
            FunctionInfo {
                name: "rebuild_cache".to_string(),
                schema: None,
                return_type: None,
                parameters: vec!["force boolean".to_string()],
                identity_arguments: Some("boolean".to_string()),
                object_id: Some("42".to_string()),
                definition: Some("BEGIN NULL; END".to_string()),
                comment: Some("rebuilds cache".to_string()),
            },
            Some("public"),
            false,
        )
        .unwrap();

        assert_eq!(routine.kind, RoutineKind::Procedure);
        assert_eq!(routine.schema.as_deref(), Some("public"));
        assert_eq!(routine.identity_arguments.as_deref(), Some("boolean"));
        assert_eq!(routine.object_id.as_deref(), Some("42"));
    }

    #[test]
    fn routine_metadata_filters_explicit_mismatched_schema() {
        let metadata = FunctionInfo {
            name: "calculate_total".to_string(),
            schema: Some("private".to_string()),
            return_type: Some("numeric".to_string()),
            parameters: Vec::new(),
            identity_arguments: None,
            object_id: None,
            definition: None,
            comment: None,
        };

        assert!(
            routine_schema_from_metadata(
                RoutineKind::Function,
                metadata.clone(),
                Some("public"),
                false,
            )
            .is_none()
        );
        assert!(
            routine_schema_from_metadata(
                RoutineKind::Function,
                FunctionInfo {
                    schema: Some("PUBLIC".to_string()),
                    ..metadata
                },
                Some("public"),
                false,
            )
            .is_some()
        );
    }

    #[test]
    fn trigger_metadata_uses_compare_schema_and_preserves_table_identity() {
        let trigger = trigger_schema_from_metadata(
            TriggerInfo {
                name: "audit_orders".to_string(),
                table_name: "orders".to_string(),
                event: "INSERT".to_string(),
                timing: "AFTER".to_string(),
                definition: Some("EXECUTE FUNCTION audit_order()".to_string()),
            },
            Some("public"),
        );

        assert_eq!(trigger.schema.as_deref(), Some("public"));
        assert_eq!(trigger.table_name, "orders");
        assert_eq!(trigger.name, "audit_orders");
        assert_eq!(trigger.event, "INSERT");
        assert_eq!(trigger.timing, "AFTER");
    }
}
