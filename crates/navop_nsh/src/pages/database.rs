use db::{
    ColumnInfo, ExecOptions, ForeignKeyDefinition, GlobalDbState, IndexInfo, QueryResult,
    SqlResult, TableDataRequest, TableDataResponse,
};
use nsh_base::{DataTableColumn, DataTableRows, data_table, input, pagination, tab, tabs, theme};
use nsh_core::prelude::*;
use one_core::storage::traits::Repository as _;
use one_core::storage::{ConnectionType, StoredConnection};

use crate::{services::NavopNshServices, ui};

const TABLE_PAGE_SIZE: usize = 100;

#[derive(Clone, PartialEq)]
struct ConnectionChoice {
    id: String,
    name: String,
    database_type: String,
}

#[derive(Clone, PartialEq)]
struct TableChoice {
    name: String,
    kind: String,
    schema: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum DatabaseView {
    #[default]
    Query,
    Data,
    Structure,
}

#[derive(Clone)]
struct TableView {
    columns: Vec<DataTableColumn>,
    rows: DataTableRows,
    status: String,
}

pub(crate) fn database() -> impl IntoElement {
    let services = NavopNshServices::current();
    let db = services.as_ref().and_then(|services| services.db.clone());
    let connections = database_connections(services.as_deref());
    let requested_connection = crate::route::take_connection_request(crate::route::Route::Database)
        .map(|id| id.to_string());
    let selected_connection = signal(requested_connection.clone());
    let databases = signal(Vec::<String>::new());
    let selected_database = signal(None::<String>);
    let schemas = signal(Vec::<String>::new());
    let selected_schema = signal(None::<String>);
    let tables = signal(Vec::<TableChoice>::new());
    let selected_table = signal(None::<TableChoice>);
    let active_view = signal(DatabaseView::Query);
    let page = signal(1usize);
    let total_count = signal(0usize);
    let sql = signal(String::from("SELECT 1"));
    let status = signal(String::from("Choose a database connection"));
    let running = signal(false);
    let columns = signal(Vec::<DataTableColumn>::new());
    let rows = signal(DataTableRows::default());
    let result_status = signal(String::from("Run a query or open a table"));
    let structure_status = signal(String::new());

    let choose_connection = {
        let db = db.clone();
        let connections = connections.clone();
        move |id: String| {
            selected_connection.set(Some(id.clone()));
            databases.set(Vec::new());
            selected_database.set(None);
            schemas.set(Vec::new());
            selected_schema.set(None);
            tables.set(Vec::new());
            selected_table.set(None);
            active_view.set(DatabaseView::Query);
            columns.set(Vec::new());
            rows.set(DataTableRows::default());
            structure_status.set(String::new());
            let Some(db) = db.clone() else {
                status.set("Database runtime is unavailable".to_owned());
                return;
            };
            if let Some(connection) = connections.iter().find(|connection| connection.id == id) {
                status.set(format!(
                    "Loading {} catalog from {}",
                    connection.database_type, connection.name
                ));
            }
            load_databases(
                db,
                id,
                databases,
                selected_database,
                schemas,
                selected_schema,
                tables,
                status,
            );
        }
    };
    let choose_database = {
        let db = db.clone();
        move |database_name: String| {
            selected_database.set(Some(database_name.clone()));
            selected_schema.set(None);
            selected_table.set(None);
            active_view.set(DatabaseView::Query);
            if let (Some(db), Some(connection_id)) =
                (db.clone(), selected_connection.get_untracked())
            {
                load_schemas_and_tables(
                    db,
                    connection_id,
                    database_name,
                    schemas,
                    selected_schema,
                    tables,
                    status,
                );
            }
        }
    };
    if let Some(connection_id) = requested_connection {
        choose_connection(connection_id);
    }
    let choose_schema = {
        let db = db.clone();
        move |schema: Option<String>| {
            selected_schema.set(schema.clone());
            selected_table.set(None);
            active_view.set(DatabaseView::Query);
            if let (Some(db), Some(connection_id), Some(database_name)) = (
                db.clone(),
                selected_connection.get_untracked(),
                selected_database.get_untracked(),
            ) {
                load_tables(db, connection_id, database_name, schema, tables, status);
            }
        }
    };
    let choose_table = {
        let db = db.clone();
        move |table: TableChoice| {
            selected_table.set(Some(table.clone()));
            active_view.set(DatabaseView::Data);
            page.set(1);
            let (Some(db), Some(connection_id), Some(database_name)) = (
                db.clone(),
                selected_connection.get_untracked(),
                selected_database.get_untracked(),
            ) else {
                status.set("Choose a database connection first".to_owned());
                return;
            };
            let schema = table.schema.clone().or(selected_schema.get_untracked());
            load_table_page(
                db,
                connection_id,
                database_name,
                table,
                schema,
                1,
                columns,
                rows,
                total_count,
                result_status,
                running,
            );
        }
    };
    let reopen_table = {
        let db = db.clone();
        move || {
            let (Some(db), Some(connection_id), Some(database_name), Some(table)) = (
                db.clone(),
                selected_connection.get_untracked(),
                selected_database.get_untracked(),
                selected_table.get_untracked(),
            ) else {
                result_status.set("Open a table from the catalog first".to_owned());
                return;
            };
            let schema = table.schema.clone().or(selected_schema.get_untracked());
            load_table_page(
                db,
                connection_id,
                database_name,
                table,
                schema,
                page.get_untracked(),
                columns,
                rows,
                total_count,
                result_status,
                running,
            );
        }
    };
    let open_structure = {
        let db = db.clone();
        move || {
            let (Some(db), Some(connection_id), Some(database_name), Some(table)) = (
                db.clone(),
                selected_connection.get_untracked(),
                selected_database.get_untracked(),
                selected_table.get_untracked(),
            ) else {
                structure_status.set("Open a table from the catalog first".to_owned());
                return;
            };
            let schema = table.schema.clone().or(selected_schema.get_untracked());
            load_structure(
                db,
                connection_id,
                database_name,
                table,
                schema,
                columns,
                rows,
                structure_status,
                running,
            );
        }
    };
    let run_query = {
        let db = db.clone();
        move || {
            let (Some(db), Some(connection_id)) = (db.clone(), selected_connection.get_untracked())
            else {
                status.set("Choose a database connection first".to_owned());
                return;
            };
            let script = sql.get_untracked();
            if script.trim().is_empty() {
                status.set("Enter a SQL statement".to_owned());
                return;
            }
            let database_name = selected_database.get_untracked();
            let schema = selected_schema.get_untracked();
            running.set(true);
            status.set("Running query…".to_owned());
            spawn(async move {
                let execution = background(async move {
                    db.execute_script_direct(
                        &connection_id,
                        &script,
                        database_name,
                        schema,
                        Some(ExecOptions {
                            max_rows: Some(500),
                            ..ExecOptions::default()
                        }),
                    )
                    .await
                })
                .await;
                running.set(false);
                match execution {
                    Ok(Ok(results)) => {
                        let table = table_view(&results);
                        columns.set(table.columns);
                        rows.set(table.rows);
                        result_status.set(table.status.clone());
                        status.set(table.status);
                    }
                    Ok(Err(error)) => clear_with_error(
                        format!("Query failed: {error:#}"),
                        columns,
                        rows,
                        result_status,
                        status,
                    ),
                    Err(error) => clear_with_error(
                        format!("Query task failed: {error}"),
                        columns,
                        rows,
                        result_status,
                        status,
                    ),
                }
            });
        }
    };

    view()
        .size_full()
        .child(ui::page_header(
            "Database",
            "nsh database workspace over the shared Navop database runtime",
        ))
        .child(
            view()
                .flex_1()
                .min_h_0()
                .flex_row()
                .child(database_catalog(
                    connections,
                    selected_connection,
                    databases,
                    selected_database,
                    schemas,
                    selected_schema,
                    tables,
                    selected_table,
                    choose_connection,
                    choose_database,
                    choose_schema,
                    choose_table,
                ))
                .child(
                    view()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .p_4()
                        .gap_3()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_3()
                                .child(
                                    text(move || database_title(selected_table))
                                        .text_sm()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    text(move || status.get())
                                        .flex_1()
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                ),
                        )
                        .child(database_tabs(active_view))
                        .when(active_view.get() == DatabaseView::Query, |workspace| {
                            workspace
                                .child(
                                    input()
                                        .multiline(true)
                                        .value(move || sql.get())
                                        .on_change(move |value: &str| sql.set(value.to_owned()))
                                        .placeholder("SELECT * FROM …")
                                        .h(px(150.0))
                                        .p_3()
                                        .rounded(move || theme().radius)
                                        .border_1()
                                        .border_color(move || theme().input)
                                        .bg(move || theme().card)
                                        .text_size(px(13.0))
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(label("RESULT"))
                                        .child(
                                            text(move || result_status.get())
                                                .flex_1()
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        )
                                        .child(
                                            ui::primary_button("Run")
                                                .disabled(move || running.get())
                                                .on_press(run_query),
                                        ),
                                )
                                .child(result_table(columns, rows, running))
                        })
                        .when(active_view.get() == DatabaseView::Data, |workspace| {
                            workspace
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            text(move || result_status.get())
                                                .flex_1()
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        )
                                        .child(
                                            ui::secondary_button("Refresh")
                                                .disabled(move || running.get())
                                                .on_press(reopen_table),
                                        ),
                                )
                                .child(
                                    pagination()
                                        .page(move || page.get().max(1) as u32)
                                        .total_pages(move || page_count(total_count.get()))
                                        .visible_pages(5)
                                        .previous_accessibility_label("Previous page")
                                        .next_accessibility_label("Next page")
                                        .on_change({
                                            let db = db.clone();
                                            move |next: &u32| {
                                                page.set((*next as usize).max(1));
                                                let (
                                                    Some(db),
                                                    Some(connection_id),
                                                    Some(database_name),
                                                    Some(table),
                                                ) = (
                                                    db.clone(),
                                                    selected_connection.get_untracked(),
                                                    selected_database.get_untracked(),
                                                    selected_table.get_untracked(),
                                                )
                                                else {
                                                    return;
                                                };
                                                let schema = table
                                                    .schema
                                                    .clone()
                                                    .or(selected_schema.get_untracked());
                                                load_table_page(
                                                    db,
                                                    connection_id,
                                                    database_name,
                                                    table,
                                                    schema,
                                                    *next as usize,
                                                    columns,
                                                    rows,
                                                    total_count,
                                                    result_status,
                                                    running,
                                                );
                                            }
                                        }),
                                )
                                .child(result_table(columns, rows, running))
                        })
                        .when(active_view.get() == DatabaseView::Structure, |workspace| {
                            workspace
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            text(move || structure_status.get())
                                                .flex_1()
                                                .text_xs()
                                                .text_color(move || theme().muted_foreground),
                                        )
                                        .child(
                                            ui::secondary_button("Reload structure")
                                                .disabled(move || running.get())
                                                .on_press(open_structure),
                                        ),
                                )
                                .child(result_table(columns, rows, running))
                        }),
                ),
        )
}

#[allow(clippy::too_many_arguments)]
fn database_catalog(
    connections: Vec<ConnectionChoice>,
    selected_connection: Signal<Option<String>>,
    databases: Signal<Vec<String>>,
    selected_database: Signal<Option<String>>,
    schemas: Signal<Vec<String>>,
    selected_schema: Signal<Option<String>>,
    tables: Signal<Vec<TableChoice>>,
    selected_table: Signal<Option<TableChoice>>,
    choose_connection: impl Fn(String) + Clone + 'static,
    choose_database: impl Fn(String) + Clone + 'static,
    choose_schema: impl Fn(Option<String>) + Clone + 'static,
    choose_table: impl Fn(TableChoice) + Clone + 'static,
) -> impl IntoElement {
    view()
        .w(px(280.0))
        .h_full()
        .min_h_0()
        .flex_none()
        .border_r_1()
        .border_r_color(move || theme().border)
        .child(
            view()
                .p_3()
                .gap_2()
                .border_b_1()
                .border_b_color(move || theme().border)
                .child(label("CONNECTIONS"))
                .child(
                    scroll()
                        .max_h(px(180.0))
                        .gap_1()
                        .children(connections.into_iter().map({
                            let choose_connection = choose_connection.clone();
                            move |connection| {
                                connection_row(
                                    connection,
                                    selected_connection,
                                    choose_connection.clone(),
                                )
                            }
                        })),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_h_0()
                .p_3()
                .gap_2()
                .child(label("CATALOG"))
                .child(
                    scroll()
                        .flex_1()
                        .min_h_0()
                        .gap_1()
                        .child(label("Databases"))
                        .children(
                            databases
                                .with(|databases| databases.clone())
                                .into_iter()
                                .map({
                                    let choose_database = choose_database.clone();
                                    move |database_name| {
                                        catalog_row(
                                            database_name,
                                            selected_database,
                                            choose_database.clone(),
                                        )
                                    }
                                }),
                        )
                        .when(schemas.with(|schemas| !schemas.is_empty()), |catalog| {
                            catalog
                                .child(label("Schemas"))
                                .child(catalog_row("All schemas".to_owned(), selected_schema, {
                                    let choose_schema = choose_schema.clone();
                                    move |_| choose_schema(None)
                                }))
                                .children(schemas.with(|schemas| schemas.clone()).into_iter().map(
                                    {
                                        let choose_schema = choose_schema.clone();
                                        move |schema| {
                                            catalog_row(schema.clone(), selected_schema, {
                                                let choose_schema = choose_schema.clone();
                                                move |selected| choose_schema(Some(selected))
                                            })
                                        }
                                    },
                                ))
                        })
                        .child(label("Tables"))
                        .children(tables.with(|tables| tables.clone()).into_iter().map({
                            let choose_table = choose_table.clone();
                            move |table| table_row(table, selected_table, choose_table.clone())
                        })),
                ),
        )
}

fn database_tabs(active_view: Signal<DatabaseView>) -> impl IntoElement {
    tabs()
        .flex_row()
        .gap_1()
        .child(database_tab("Query", DatabaseView::Query, active_view))
        .child(database_tab("Data", DatabaseView::Data, active_view))
        .child(database_tab(
            "Structure",
            DatabaseView::Structure,
            active_view,
        ))
}

fn database_tab(
    label_text: &'static str,
    view_kind: DatabaseView,
    active_view: Signal<DatabaseView>,
) -> impl IntoElement {
    let selected = move || active_view.get() == view_kind;
    tab()
        .selected(selected)
        .px_3()
        .py_1p5()
        .rounded(move || theme().radius)
        .bg(move || {
            if selected() {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|style| style.bg(move || theme().muted))
        .on_press(move || active_view.set(view_kind))
        .child(
            text(label_text)
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

fn result_table(
    columns: Signal<Vec<DataTableColumn>>,
    rows: Signal<DataTableRows>,
    running: Signal<bool>,
) -> impl IntoElement {
    data_table()
        .flex_1()
        .min_h_0()
        .columns(move || columns.get())
        .data(move || rows.get())
        .loading(move || running.get())
        .row_height(px(30.0))
        .font_size(px(12.0))
}

fn database_connections(services: Option<&NavopNshServices>) -> Vec<ConnectionChoice> {
    services
        .and_then(|services| services.connections.list().ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|connection| connection.connection_type == ConnectionType::Database)
        .filter_map(connection_choice)
        .collect()
}

fn connection_choice(connection: StoredConnection) -> Option<ConnectionChoice> {
    let config = connection.to_db_connection().ok()?;
    Some(ConnectionChoice {
        id: config.id,
        name: connection.name,
        database_type: format!("{:?}", config.database_type),
    })
}

fn connection_row(
    connection: ConnectionChoice,
    selected: Signal<Option<String>>,
    choose: impl Fn(String) + Clone + 'static,
) -> impl IntoElement {
    let id = connection.id.clone();
    let active_id = id.clone();
    let active = move || selected.get().as_deref() == Some(active_id.as_str());
    let active_state = active.clone();
    let active_bg = active;
    view()
        .h(px(42.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(connection.name.clone())
        .accessibility_state(move || AccessibilityState {
            selected: active_state(),
            ..AccessibilityState::default()
        })
        .bg(move || {
            if active_bg() {
                theme().accent
            } else {
                theme().sidebar
            }
        })
        .hover(|style| style.bg(move || theme().muted))
        .on_press(move || choose(id.clone()))
        .child(
            view()
                .size(px(26.0))
                .items_center()
                .justify_center()
                .rounded(px(7.0))
                .bg(move || theme().muted)
                .child(
                    text("DB")
                        .text_size(px(10.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w_0()
                .gap(px(1.0))
                .child(
                    text(connection.name)
                        .text_sm()
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(connection.database_type)
                        .text_size(px(10.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

fn catalog_row(
    value: String,
    selected: Signal<Option<String>>,
    choose: impl Fn(String) + Clone + 'static,
) -> impl IntoElement {
    let press_value = value.clone();
    let active_value = value.clone();
    let active = move || selected.get().as_deref() == Some(active_value.as_str());
    let active_state = active.clone();
    let active_bg = active;
    view()
        .h(px(28.0))
        .flex_row()
        .items_center()
        .px_2()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(value.clone())
        .accessibility_state(move || AccessibilityState {
            selected: active_state(),
            ..AccessibilityState::default()
        })
        .bg(move || {
            if active_bg() {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|style| style.bg(move || theme().muted))
        .on_press(move || choose(press_value.clone()))
        .child(text(value).text_sm().text_color(move || theme().foreground))
}

fn table_row(
    table: TableChoice,
    selected: Signal<Option<TableChoice>>,
    choose: impl Fn(TableChoice) + Clone + 'static,
) -> impl IntoElement {
    let press_table = table.clone();
    let active_table = table.clone();
    let active = move || selected.get().as_ref() == Some(&active_table);
    let active_state = active.clone();
    let active_bg = active;
    view()
        .h(px(30.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(table.name.clone())
        .accessibility_state(move || AccessibilityState {
            selected: active_state(),
            ..AccessibilityState::default()
        })
        .bg(move || {
            if active_bg() {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|style| style.bg(move || theme().muted))
        .on_press(move || choose(press_table.clone()))
        .child(
            text(if table.kind == "View" { "V" } else { "T" })
                .text_size(px(9.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_1()
                .min_w_0()
                .child(
                    text(table.name)
                        .text_sm()
                        .text_color(move || theme().foreground),
                )
                .when_some(table.schema, |row, schema| {
                    row.child(
                        text(schema)
                            .text_size(px(10.0))
                            .text_color(move || theme().muted_foreground),
                    )
                }),
        )
}

#[allow(clippy::too_many_arguments)]
fn load_databases(
    db: GlobalDbState,
    connection_id: String,
    databases: Signal<Vec<String>>,
    selected_database: Signal<Option<String>>,
    schemas: Signal<Vec<String>>,
    selected_schema: Signal<Option<String>>,
    tables: Signal<Vec<TableChoice>>,
    status: Signal<String>,
) {
    spawn(async move {
        let state = db.clone();
        let id = connection_id.clone();
        let result = background(async move { state.list_databases_direct(id).await }).await;
        match result {
            Ok(Ok(databases_found)) => {
                let first = databases_found.first().cloned();
                databases.set(databases_found);
                if let Some(first) = first {
                    selected_database.set(Some(first.clone()));
                    load_schemas_and_tables(
                        db,
                        connection_id,
                        first,
                        schemas,
                        selected_schema,
                        tables,
                        status,
                    );
                } else {
                    status.set("No databases returned".to_owned());
                }
            }
            Ok(Err(error)) => status.set(format!("Failed to load databases: {error:#}")),
            Err(error) => status.set(format!("Database task failed: {error}")),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn load_schemas_and_tables(
    db: GlobalDbState,
    connection_id: String,
    database_name: String,
    schemas: Signal<Vec<String>>,
    selected_schema: Signal<Option<String>>,
    tables: Signal<Vec<TableChoice>>,
    status: Signal<String>,
) {
    status.set(format!("Loading schemas from {database_name}…"));
    spawn(async move {
        let schema_state = db.clone();
        let schema_connection_id = connection_id.clone();
        let schema_database = database_name.clone();
        let schema_result = background(async move {
            schema_state
                .list_schemas_direct(schema_connection_id, schema_database)
                .await
        })
        .await;
        let schemas_found = match schema_result {
            Ok(Ok(schemas)) => schemas,
            Ok(Err(_)) | Err(_) => Vec::new(),
        };
        schemas.set(schemas_found);
        selected_schema.set(None);
        load_tables(db, connection_id, database_name, None, tables, status);
    });
}

#[allow(clippy::too_many_arguments)]
fn load_tables(
    db: GlobalDbState,
    connection_id: String,
    database_name: String,
    schema: Option<String>,
    tables: Signal<Vec<TableChoice>>,
    status: Signal<String>,
) {
    let scope = schema.as_deref().map_or_else(
        || database_name.clone(),
        |schema| format!("{database_name}.{schema}"),
    );
    status.set(format!("Loading tables from {scope}…"));
    spawn(async move {
        let state = db.clone();
        let result = background(async move {
            state
                .list_tables_direct(&connection_id, &database_name, schema)
                .await
        })
        .await;
        match result {
            Ok(Ok(tables_found)) => {
                let count = tables_found.len();
                tables.set(
                    tables_found
                        .into_iter()
                        .map(|table| TableChoice {
                            name: table.name,
                            kind: format!("{:?}", table.object_type),
                            schema: table.schema,
                        })
                        .collect(),
                );
                status.set(format!("{count} tables in {scope}"));
            }
            Ok(Err(error)) => status.set(format!("Failed to load tables: {error:#}")),
            Err(error) => status.set(format!("Table task failed: {error}")),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn load_table_page(
    db: GlobalDbState,
    connection_id: String,
    database_name: String,
    table: TableChoice,
    schema: Option<String>,
    page: usize,
    columns: Signal<Vec<DataTableColumn>>,
    rows: Signal<DataTableRows>,
    total_count: Signal<usize>,
    result_status: Signal<String>,
    running: Signal<bool>,
) {
    running.set(true);
    result_status.set(format!("Loading {}…", table.name));
    spawn(async move {
        let state = db.clone();
        let table_name = table.name.clone();
        let mut request =
            TableDataRequest::new(&database_name, &table_name).with_page(page, TABLE_PAGE_SIZE);
        if let Some(schema) = schema {
            request = request.with_schema(schema);
        }
        let response =
            background(async move { state.query_table_data_direct(&connection_id, request).await })
                .await;
        running.set(false);
        match response {
            Ok(Ok(response)) => {
                let view = table_data_view(&response);
                total_count.set(response.total_count);
                columns.set(view.columns);
                rows.set(view.rows);
                result_status.set(view.status);
            }
            Ok(Err(error)) => clear_with_error(
                format!("Table query failed: {error:#}"),
                columns,
                rows,
                result_status,
                result_status,
            ),
            Err(error) => clear_with_error(
                format!("Table query task failed: {error}"),
                columns,
                rows,
                result_status,
                result_status,
            ),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn load_structure(
    db: GlobalDbState,
    connection_id: String,
    database_name: String,
    table: TableChoice,
    schema: Option<String>,
    columns: Signal<Vec<DataTableColumn>>,
    rows: Signal<DataTableRows>,
    structure_status: Signal<String>,
    running: Signal<bool>,
) {
    running.set(true);
    structure_status.set(format!("Loading structure for {}…", table.name));
    spawn(async move {
        let state = db.clone();
        let table_name = table.name.clone();
        let schema_for_columns = schema.clone();
        let result = background(async move {
            let columns = state
                .list_columns_direct(
                    &connection_id,
                    &database_name,
                    schema_for_columns,
                    &table_name,
                )
                .await?;
            let indexes = state
                .list_indexes_direct(&connection_id, &database_name, schema.clone(), &table_name)
                .await?;
            let foreign_keys = state
                .list_foreign_keys_direct(&connection_id, &database_name, schema, &table_name)
                .await?;
            Ok::<_, anyhow::Error>((columns, indexes, foreign_keys))
        })
        .await;
        running.set(false);
        match result {
            Ok(Ok((columns_meta, indexes, foreign_keys))) => {
                let view = structure_view(&columns_meta);
                columns.set(view.columns);
                rows.set(view.rows);
                structure_status.set(format!(
                    "{} column(s) · {} index(es) · {} foreign key(s){}",
                    columns_meta.len(),
                    indexes.len(),
                    foreign_keys.len(),
                    structure_detail(&indexes, &foreign_keys)
                ));
            }
            Ok(Err(error)) => clear_with_error(
                format!("Structure query failed: {error:#}"),
                columns,
                rows,
                structure_status,
                structure_status,
            ),
            Err(error) => clear_with_error(
                format!("Structure task failed: {error}"),
                columns,
                rows,
                structure_status,
                structure_status,
            ),
        }
    });
}

fn clear_with_error(
    message: String,
    columns: Signal<Vec<DataTableColumn>>,
    rows: Signal<DataTableRows>,
    result_status: Signal<String>,
    status: Signal<String>,
) {
    columns.set(Vec::new());
    rows.set(DataTableRows::default());
    result_status.set(message.clone());
    status.set(message);
}

fn database_title(selected_table: Signal<Option<TableChoice>>) -> String {
    selected_table
        .get()
        .map(|table| {
            let name = table.name;
            table
                .schema
                .map_or_else(|| name.clone(), |schema| format!("{schema}.{name}"))
        })
        .unwrap_or_else(|| "SQL workspace".to_owned())
}

fn page_count(total_count: usize) -> u32 {
    total_count.div_ceil(TABLE_PAGE_SIZE).max(1) as u32
}

fn table_view(results: &[SqlResult]) -> TableView {
    if let Some(error) = results.iter().find_map(|result| match result {
        SqlResult::Error(error) => Some(error),
        _ => None,
    }) {
        return TableView {
            columns: Vec::new(),
            rows: DataTableRows::default(),
            status: format!("SQL error: {}", error.message),
        };
    }

    if let Some(query) = results.iter().rev().find_map(|result| match result {
        SqlResult::Query(query) => Some(query),
        _ => None,
    }) {
        return query_table_view(query, false);
    }

    if let Some(exec) = results.iter().rev().find_map(|result| match result {
        SqlResult::Exec(exec) => Some(exec),
        _ => None,
    }) {
        return TableView {
            columns: Vec::new(),
            rows: DataTableRows::default(),
            status: format!(
                "{} row(s) affected in {} ms",
                exec.rows_affected, exec.elapsed_ms
            ),
        };
    }

    TableView {
        columns: Vec::new(),
        rows: DataTableRows::default(),
        status: "No result returned".to_owned(),
    }
}

fn table_data_view(response: &TableDataResponse) -> TableView {
    let mut view = query_table_view(&response.query_result, true);
    view.status = format!(
        "{} row(s) total · page {} · {} row(s) · {} ms",
        response.total_count,
        response.page,
        response.query_result.rows.len(),
        response.duration
    );
    view
}

fn query_table_view(query: &QueryResult, skip_rowid: bool) -> TableView {
    let mut columns = Vec::with_capacity(query.columns.len());
    let mut rows = DataTableRows::new(query.rows.len());
    for (index, name) in query.columns.iter().enumerate() {
        if skip_rowid && name == "__rowid__" {
            continue;
        }
        let key = format!("c{index}");
        let mut column = DataTableColumn::default();
        column.key = key.clone().into();
        column.title = Some(name.clone().into());
        column.width = Some(px(150.0));
        columns.push(column);
        rows = rows.texts(
            &key,
            query.rows.iter().map(|row| {
                row.get(index)
                    .and_then(|value| value.as_deref())
                    .unwrap_or("NULL")
            }),
        );
    }
    TableView {
        columns,
        rows,
        status: format!(
            "{} row(s) · {} column(s) · {} ms",
            query.rows.len(),
            query.columns.len(),
            query.elapsed_ms
        ),
    }
}

fn structure_view(columns_meta: &[ColumnInfo]) -> TableView {
    let mut rows = DataTableRows::new(columns_meta.len());
    rows = rows.texts(
        "name",
        columns_meta.iter().map(|column| column.name.as_str()),
    );
    rows = rows.texts(
        "type",
        columns_meta.iter().map(|column| column.data_type.as_str()),
    );
    rows = rows.texts(
        "nullable",
        columns_meta
            .iter()
            .map(|column| if column.is_nullable { "YES" } else { "NO" }),
    );
    rows = rows.texts(
        "primary",
        columns_meta
            .iter()
            .map(|column| if column.is_primary_key { "PK" } else { "" }),
    );
    rows = rows.texts(
        "default",
        columns_meta
            .iter()
            .map(|column| column.default_value.as_deref().unwrap_or("")),
    );
    rows = rows.texts(
        "comment",
        columns_meta
            .iter()
            .map(|column| column.comment.as_deref().unwrap_or("")),
    );

    TableView {
        columns: [
            ("name", "Name", 180.0),
            ("type", "Type", 180.0),
            ("nullable", "Nullable", 80.0),
            ("primary", "Key", 60.0),
            ("default", "Default", 160.0),
            ("comment", "Comment", 220.0),
        ]
        .into_iter()
        .map(|(key, title, width)| {
            let mut column = DataTableColumn::default();
            column.key = key.into();
            column.title = Some(title.into());
            column.width = Some(px(width));
            column
        })
        .collect(),
        rows,
        status: format!("{} column(s)", columns_meta.len()),
    }
}

fn structure_detail(indexes: &[IndexInfo], foreign_keys: &[ForeignKeyDefinition]) -> String {
    let mut detail = String::new();
    if !indexes.is_empty() {
        detail.push_str("\n\nIndexes:\n");
        for index in indexes {
            detail.push_str(&format!(
                "  {} ({}){}{}\n",
                index.name,
                index.columns.join(", "),
                if index.is_primary { " PRIMARY" } else { "" },
                if index.is_unique { " UNIQUE" } else { "" }
            ));
        }
    }
    if !foreign_keys.is_empty() {
        detail.push_str("\nForeign keys:\n");
        for foreign_key in foreign_keys {
            detail.push_str(&format!(
                "  {} ({}) -> {}({})\n",
                foreign_key.name,
                foreign_key.columns.join(", "),
                foreign_key.ref_table,
                foreign_key.ref_columns.join(", ")
            ));
        }
    }
    detail
}

fn label(value: &'static str) -> impl IntoElement {
    text(value)
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().muted_foreground)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_result_becomes_nsh_table_columns_and_rows() {
        let query = QueryResult {
            columns: vec!["id".to_owned(), "name".to_owned()],
            rows: vec![
                vec![Some("1".to_owned()), Some("alpha".to_owned())],
                vec![Some("2".to_owned()), None],
            ],
            ..QueryResult::default()
        };
        let table = table_view(&[SqlResult::Query(query)]);
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[0].key.as_ref(), "c0");
        assert_eq!(table.columns[1].title.as_deref(), Some("name"));
        assert_eq!(table.rows.len(), 2);
        assert!(table.status.contains("2 row(s)"));
    }

    #[test]
    fn sql_errors_do_not_replace_the_table_with_partial_rows() {
        let error = db::SqlErrorInfo {
            sql: "select broken".to_owned(),
            message: "syntax error".to_owned(),
        };
        let table = table_view(&[SqlResult::Error(error)]);
        assert!(table.columns.is_empty());
        assert_eq!(table.rows.len(), 0);
        assert!(table.status.contains("syntax error"));
    }

    #[test]
    fn table_data_hides_rowid_but_keeps_real_columns_aligned() {
        let response = TableDataResponse {
            query_result: QueryResult {
                columns: vec!["__rowid__".to_owned(), "id".to_owned(), "name".to_owned()],
                rows: vec![vec![
                    Some("9".to_owned()),
                    Some("1".to_owned()),
                    Some("Ada".to_owned()),
                ]],
                ..QueryResult::default()
            },
            total_count: 3,
            page: 1,
            page_size: 100,
            duration: 4,
        };
        let table = table_data_view(&response);
        assert_eq!(table.columns.len(), 2);
        assert_eq!(table.columns[0].key.as_ref(), "c1");
        assert!(table.status.contains("3 row(s) total"));
    }

    #[test]
    fn pagination_has_one_page_for_an_empty_table() {
        assert_eq!(page_count(0), 1);
        assert_eq!(page_count(1), 1);
        assert_eq!(page_count(TABLE_PAGE_SIZE), 1);
        assert_eq!(page_count(TABLE_PAGE_SIZE + 1), 2);
    }
}
