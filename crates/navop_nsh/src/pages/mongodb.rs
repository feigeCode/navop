use anyhow::{Context as _, bail};
use bson::{Bson, Document};
use mongodb_runtime::{MongoFindOptions, bson_to_pretty_json};
use mongodb_view::{GlobalMongoState, MongoManager};
use nsh_base::{input, text_view, theme};
use nsh_core::prelude::*;
use one_core::storage::{ConnectionType, StoredConnection};

use crate::{services::NavopNshServices, ui};

#[derive(Clone)]
struct MongoSession {
    state: GlobalMongoState,
    id: String,
}

impl PartialEq for MongoSession {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}

pub(crate) fn mongodb() -> impl IntoElement {
    let services = NavopNshServices::current();
    let mongo_state = services
        .as_ref()
        .and_then(|services| services.mongo.clone());
    let connections = mongo_connections(services.as_deref());
    let requested_connection =
        crate::route::take_connection_request(crate::route::Route::Mongo).map(|id| id.to_string());
    let selected_connection = signal(requested_connection.clone());
    let databases = signal(Vec::<String>::new());
    let selected_database = signal(None::<String>);
    let collections = signal(Vec::<String>::new());
    let selected_collection = signal(None::<String>);
    let filter = signal(String::from("{}"));
    let limit = signal(String::from("50"));
    let documents = signal(String::from("Select a collection"));
    let document_list = signal(Vec::<Document>::new());
    let selected_document = signal(None::<usize>);
    let document_editor = signal(String::from("{\n\n}\n"));
    let aggregate_pipeline = signal(String::from("[]"));
    let count = signal(0i64);
    let status = signal(String::from("Choose a MongoDB connection"));
    let session = signal(None::<MongoSession>);
    let busy = signal(false);

    let connect = {
        let connections = connections.clone();
        let mongo_state = mongo_state.clone();
        move |id: String| {
            let Some(connection) = connections
                .iter()
                .find(|connection| connection.id_string() == id)
                .cloned()
            else {
                status.set("MongoDB connection was not found".to_owned());
                return;
            };
            let Some(state) = mongo_state.clone() else {
                status.set("MongoDB runtime is unavailable".to_owned());
                return;
            };
            let config = match MongoManager::config_from_stored(&connection) {
                Ok(config) => config,
                Err(error) => {
                    status.set(format!("Invalid MongoDB connection: {error}"));
                    return;
                }
            };
            let name = connection.name;
            selected_connection.set(Some(id));
            databases.set(Vec::new());
            selected_database.set(None);
            collections.set(Vec::new());
            selected_collection.set(None);
            documents.set("Select a collection".to_owned());
            document_list.set(Vec::new());
            selected_document.set(None);
            document_editor.set("{\n\n}\n".to_owned());
            count.set(0);
            busy.set(true);
            status.set(format!("Connecting to {name}…"));
            spawn(async move {
                let state_for_connect = state.clone();
                let connected = background(async move {
                    let id = state_for_connect.create_connection(config).await?;
                    let connection = state_for_connect
                        .get_connection(&id)
                        .ok_or_else(|| anyhow::anyhow!("MongoDB connection was not cached"))?;
                    let databases = connection.read().await.list_databases().await?;
                    Ok::<(String, Vec<String>), anyhow::Error>((id, databases))
                })
                .await;
                busy.set(false);
                match connected {
                    Ok(Ok((id, found))) => {
                        session.set(Some(MongoSession {
                            state: state.clone(),
                            id,
                        }));
                        databases.set(found);
                        status.set(format!("Connected to {name}"));
                    }
                    Ok(Err(error)) => status.set(format!("Connection failed: {error:#}")),
                    Err(error) => status.set(format!("Connection task failed: {error}")),
                }
            });
        }
    };
    let choose_database = {
        move |database: String| {
            selected_database.set(Some(database.clone()));
            selected_collection.set(None);
            documents.set("Select a collection".to_owned());
            document_list.set(Vec::new());
            selected_document.set(None);
            count.set(0);
            if let Some(session) = session.get_untracked() {
                load_collections(
                    session,
                    database,
                    collections,
                    selected_collection,
                    status,
                    busy,
                );
            }
        }
    };
    if let Some(connection_id) = requested_connection {
        connect(connection_id);
    }
    let run_query = move || {
        let (Some(session), Some(database), Some(collection)) = (
            session.get_untracked(),
            selected_database.get_untracked(),
            selected_collection.get_untracked(),
        ) else {
            status.set("Select a database and collection first".to_owned());
            return;
        };
        let filter = match parse_filter(&filter.get_untracked()) {
            Ok(filter) => filter,
            Err(error) => {
                status.set(format!("Invalid filter: {error:#}"));
                return;
            }
        };
        let limit = match parse_limit(&limit.get_untracked()) {
            Ok(limit) => limit,
            Err(error) => {
                status.set(format!("Invalid limit: {error:#}"));
                return;
            }
        };
        load_documents(
            session,
            database,
            collection,
            filter,
            limit,
            documents,
            document_list,
            selected_document,
            document_editor,
            count,
            status,
            busy,
        );
    };
    let save_document = {
        move || {
            let (Some(session), Some(database), Some(collection)) = (
                session.get_untracked(),
                selected_database.get_untracked(),
                selected_collection.get_untracked(),
            ) else {
                status.set("Select a database and collection first".to_owned());
                return;
            };
            let document = match parse_document(&document_editor.get_untracked()) {
                Ok(document) => document,
                Err(error) => {
                    status.set(format!("Invalid document: {error:#}"));
                    return;
                }
            };
            let editing = selected_document.get_untracked().and_then(|index| {
                document_list.with(|documents| {
                    documents
                        .get(index)
                        .and_then(|document| document.get("_id"))
                        .cloned()
                })
            });
            busy.set(true);
            status.set(if editing.is_some() {
                "Saving document…".to_owned()
            } else {
                "Inserting document…".to_owned()
            });
            spawn(async move {
                let state = session.state;
                let id = session.id;
                let database_for_write = database.clone();
                let collection_for_write = collection.clone();
                let result = background(async move {
                    let connection = state
                        .get_connection(&id)
                        .ok_or_else(|| anyhow::anyhow!("MongoDB connection is not cached"))?;
                    let guard = connection.read().await;
                    if let Some(document_id) = editing {
                        guard
                            .replace_document(
                                &database_for_write,
                                &collection_for_write,
                                document_id,
                                document,
                            )
                            .await
                    } else {
                        guard
                            .insert_document(&database_for_write, &collection_for_write, document)
                            .await
                    }
                    .map_err(|error| anyhow::anyhow!("{error}"))
                })
                .await;
                busy.set(false);
                match result {
                    Ok(Ok(())) => {
                        status.set("Document saved".to_owned());
                        run_query();
                    }
                    Ok(Err(error)) => status.set(format!("Save failed: {error:#}")),
                    Err(error) => status.set(format!("Save task failed: {error}")),
                }
            });
        }
    };
    let delete_document = {
        move || {
            let (Some(session), Some(database), Some(collection), Some(index)) = (
                session.get_untracked(),
                selected_database.get_untracked(),
                selected_collection.get_untracked(),
                selected_document.get_untracked(),
            ) else {
                status.set("Select a document to delete".to_owned());
                return;
            };
            let Some(document_id) = document_list.with(|documents| {
                documents
                    .get(index)
                    .and_then(|document| document.get("_id"))
                    .cloned()
            }) else {
                status.set("The selected document has no _id".to_owned());
                return;
            };
            busy.set(true);
            status.set("Deleting document…".to_owned());
            spawn(async move {
                let state = session.state;
                let id = session.id;
                let result = background(async move {
                    let connection = state
                        .get_connection(&id)
                        .ok_or_else(|| anyhow::anyhow!("MongoDB connection is not cached"))?;
                    connection
                        .read()
                        .await
                        .delete_document(&database, &collection, document_id)
                        .await
                        .map_err(|error| anyhow::anyhow!("{error}"))
                })
                .await;
                busy.set(false);
                match result {
                    Ok(Ok(())) => {
                        status.set("Document deleted".to_owned());
                        run_query();
                    }
                    Ok(Err(error)) => status.set(format!("Delete failed: {error:#}")),
                    Err(error) => status.set(format!("Delete task failed: {error}")),
                }
            });
        }
    };
    let run_aggregate = move || {
        let (Some(session), Some(database), Some(collection)) = (
            session.get_untracked(),
            selected_database.get_untracked(),
            selected_collection.get_untracked(),
        ) else {
            status.set("Select a database and collection first".to_owned());
            return;
        };
        let pipeline = match parse_pipeline(&aggregate_pipeline.get_untracked()) {
            Ok(pipeline) => pipeline,
            Err(error) => {
                status.set(format!("Invalid aggregation pipeline: {error:#}"));
                return;
            }
        };
        busy.set(true);
        status.set("Running aggregation…".to_owned());
        spawn(async move {
            let state = session.state;
            let id = session.id;
            let database_for_query = database.clone();
            let collection_for_query = collection.clone();
            let result = background(async move {
                let connection = state
                    .get_connection(&id)
                    .ok_or_else(|| anyhow::anyhow!("MongoDB connection is not cached"))?;
                connection
                    .read()
                    .await
                    .aggregate_documents(&database_for_query, &collection_for_query, pipeline)
                    .await
                    .map_err(|error| anyhow::anyhow!("{error}"))
            })
            .await;
            busy.set(false);
            match result {
                Ok(Ok(found)) => {
                    count.set(found.len() as i64);
                    documents.set(documents_json(&found));
                    document_list.set(found);
                    selected_document.set(None);
                    status.set("Aggregation complete".to_owned());
                }
                Ok(Err(error)) => status.set(format!("Aggregation failed: {error:#}")),
                Err(error) => status.set(format!("Aggregation task failed: {error}")),
            }
        });
    };
    let show_indexes = move || {
        let (Some(session), Some(database), Some(collection)) = (
            session.get_untracked(),
            selected_database.get_untracked(),
            selected_collection.get_untracked(),
        ) else {
            status.set("Select a database and collection first".to_owned());
            return;
        };
        busy.set(true);
        status.set("Loading indexes…".to_owned());
        spawn(async move {
            let state = session.state;
            let id = session.id;
            let database_for_query = database.clone();
            let collection_for_query = collection.clone();
            let result = background(async move {
                let connection = state
                    .get_connection(&id)
                    .ok_or_else(|| anyhow::anyhow!("MongoDB connection is not cached"))?;
                connection
                    .read()
                    .await
                    .list_indexes(&database_for_query, &collection_for_query)
                    .await
                    .map_err(|error| anyhow::anyhow!("{error}"))
            })
            .await;
            busy.set(false);
            match result {
                Ok(Ok(indexes)) => {
                    documents.set(documents_json(&indexes));
                    document_list.set(Vec::new());
                    selected_document.set(None);
                    status.set(format!("{} index(es)", indexes.len()));
                }
                Ok(Err(error)) => status.set(format!("Index load failed: {error:#}")),
                Err(error) => status.set(format!("Index task failed: {error}")),
            }
        });
    };
    let disconnect = move || {
        let current = session.get_untracked();
        session.set(None);
        if let Some(session) = current {
            let state = session.state;
            let id = session.id;
            let _ = background(async move {
                let _ = state.remove_connection(&id).await;
            });
        }
        databases.set(Vec::new());
        collections.set(Vec::new());
        selected_database.set(None);
        selected_collection.set(None);
        documents.set("Select a collection".to_owned());
        document_list.set(Vec::new());
        selected_document.set(None);
        status.set("Disconnected".to_owned());
    };

    cleanup(move || {
        if let Some(session) = session.get_untracked() {
            let state = session.state;
            let id = session.id;
            let _ = background(async move {
                let _ = state.remove_connection(&id).await;
            });
        }
    });

    view()
        .size_full()
        .child(ui::page_header(
            "MongoDB",
            "nsh document browser over the shared MongoDB runtime",
        ))
        .child(
            view()
                .flex_none()
                .flex_row()
                .items_center()
                .gap_2()
                .px_4()
                .py_3()
                .border_b_1()
                .border_b_color(move || theme().border)
                .child(label("CONNECTIONS"))
                .children(connections.iter().cloned().map({
                    let connect = connect.clone();
                    move |connection| {
                        let id = connection.id_string();
                        let active_id = id.clone();
                        let active = move || {
                            selected_connection.get().as_deref() == Some(active_id.as_str())
                        };
                        let name = connection.name.clone();
                        view()
                            .px_3()
                            .py_1p5()
                            .rounded(move || theme().radius)
                            .role(AccessibilityRole::Button)
                            .label(name.clone())
                            .bg(move || {
                                if active() {
                                    theme().accent
                                } else {
                                    theme().muted
                                }
                            })
                            .on_press({
                                let connect = connect.clone();
                                let id = id.clone();
                                move || connect(id.clone())
                            })
                            .child(text(name).text_xs().text_color(move || theme().foreground))
                    }
                }))
                .child(view().flex_1())
                .child(
                    ui::secondary_button("Disconnect")
                        .disabled(move || session.get().is_none())
                        .on_press(disconnect),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_h_0()
                .flex_row()
                .child(
                    view()
                        .w(px(300.0))
                        .h_full()
                        .min_h_0()
                        .flex_none()
                        .p_3()
                        .gap_2()
                        .border_r_1()
                        .border_r_color(move || theme().border)
                        .child(label("DATABASES"))
                        .child(
                            scroll().max_h(px(220.0)).gap_1().children(
                                databases
                                    .with(|databases| databases.clone())
                                    .into_iter()
                                    .map({
                                        move |database| {
                                            database_row(
                                                database,
                                                selected_database,
                                                choose_database,
                                            )
                                        }
                                    }),
                            ),
                        )
                        .child(label("COLLECTIONS"))
                        .child(
                            scroll().flex_1().min_h_0().gap_1().children(
                                collections
                                    .with(|collections| collections.clone())
                                    .into_iter()
                                    .map({
                                        move |collection| {
                                            collection_row(
                                                collection,
                                                selected_collection,
                                                selected_database,
                                                session,
                                                filter,
                                                limit,
                                                documents,
                                                document_list,
                                                selected_document,
                                                document_editor,
                                                count,
                                                status,
                                                busy,
                                            )
                                        }
                                    }),
                            ),
                        ),
                )
                .child(
                    view()
                        .flex_1()
                        .min_w_0()
                        .min_h_0()
                        .p_4()
                        .gap_2()
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    input()
                                        .value(move || filter.get())
                                        .on_change(move |value: &str| filter.set(value.to_owned()))
                                        .placeholder("Filter JSON, e.g. {\"status\":\"active\"}")
                                        .flex_1()
                                        .px_2()
                                        .py_1p5()
                                        .rounded(move || theme().radius)
                                        .border_1()
                                        .border_color(move || theme().input)
                                        .text_size(px(12.0))
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    input()
                                        .value(move || limit.get())
                                        .on_change(move |value: &str| limit.set(value.to_owned()))
                                        .placeholder("Limit")
                                        .w(px(72.0))
                                        .px_2()
                                        .py_1p5()
                                        .rounded(move || theme().radius)
                                        .border_1()
                                        .border_color(move || theme().input)
                                        .text_size(px(12.0))
                                        .text_color(move || theme().foreground),
                                )
                                .child(
                                    ui::primary_button("Run")
                                        .disabled(move || busy.get())
                                        .on_press(run_query),
                                ),
                        )
                        .child(
                            text(move || status.get())
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text(move || format!("{} document(s)", count.get()))
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .flex_wrap()
                                .gap_2()
                                .child(
                                    ui::secondary_button("New")
                                        .on_press(move || {
                                            selected_document.set(None);
                                            document_editor.set("{\n\n}\n".to_owned());
                                            status.set("Editing a new document".to_owned());
                                        }),
                                )
                                .child(
                                    ui::primary_button("Save")
                                        .disabled(move || busy.get())
                                        .on_press(save_document),
                                )
                                .child(
                                    ui::secondary_button("Delete")
                                        .disabled(move || {
                                            selected_document.get().is_none() || busy.get()
                                        })
                                        .on_press(delete_document),
                                )
                                .child(
                                    ui::secondary_button("Indexes")
                                        .disabled(move || busy.get())
                                        .on_press(show_indexes),
                                ),
                        )
                        .child(
                            view()
                                .flex_1()
                                .min_h_0()
                                .flex_row()
                                .gap_2()
                                .child(
                                    scroll()
                                        .w(px(210.0))
                                        .h_full()
                                        .min_h_0()
                                        .flex_none()
                                        .gap_1()
                                        .children(
                                            document_list
                                                .with(|documents| documents.clone())
                                                .into_iter()
                                                .enumerate()
                                                .map(|(index, document)| {
                                                    document_row(
                                                        index,
                                                        document,
                                                        selected_document,
                                                        document_editor,
                                                        status,
                                                    )
                                                }),
                                        ),
                                )
                                .child(
                                    view()
                                        .flex_1()
                                        .min_w_0()
                                        .min_h_0()
                                        .gap_2()
                                        .child(
                                            scroll()
                                                .flex_1()
                                                .min_h_0()
                                                .p_3()
                                                .rounded(move || theme().radius)
                                                .border_1()
                                                .border_color(move || theme().border)
                                                .bg(move || theme().card)
                                                .child(
                                                    text_view()
                                                        .markdown(move || {
                                                            Some(SharedString::from(format!(
                                                                "```json\n{}\n```",
                                                                documents.get()
                                                            )))
                                                        })
                                                        .text_color(move || {
                                                            theme().foreground
                                                        }),
                                                ),
                                        )
                                        .child(
                                            view()
                                                .h(px(128.0))
                                                .flex_none()
                                                .gap_1()
                                                .child(
                                                    text("Document JSON / aggregation pipeline")
                                                        .text_xs()
                                                        .text_color(move || {
                                                            theme().muted_foreground
                                                        }),
                                                )
                                                .child(
                                                    input()
                                                        .multiline(true)
                                                        .value(move || {
                                                            document_editor.get()
                                                        })
                                                        .on_change(move |value: &str| {
                                                            document_editor
                                                                .set(value.to_owned())
                                                        })
                                                        .placeholder(
                                                            "{\"name\":\"Ada\"} or [{\"$match\":{}}]",
                                                        )
                                                        .flex_1()
                                                        .p_2()
                                                        .rounded(move || theme().radius)
                                                        .border_1()
                                                        .border_color(move || theme().input)
                                                        .text_size(px(12.0))
                                                        .text_color(move || theme().foreground),
                                                ),
                                        )
                                        .child(
                                            view()
                                                .flex_row()
                                                .gap_2()
                                                .child(
                                                    input()
                                                        .value(move || {
                                                            aggregate_pipeline.get()
                                                        })
                                                        .on_change(move |value: &str| {
                                                            aggregate_pipeline
                                                                .set(value.to_owned())
                                                        })
                                                        .placeholder("Aggregation pipeline JSON array")
                                                        .flex_1()
                                                        .px_2()
                                                        .py_1p5()
                                                        .rounded(move || theme().radius)
                                                        .border_1()
                                                        .border_color(move || theme().input)
                                                        .text_size(px(12.0))
                                                        .text_color(move || theme().foreground),
                                                )
                                                .child(
                                                    ui::secondary_button("Aggregate")
                                                        .disabled(move || busy.get())
                                                        .on_press(run_aggregate),
                                                ),
                                        ),
                                ),
                        ),
                ),
        )
}

fn database_row(
    database: String,
    selected: Signal<Option<String>>,
    choose: impl Fn(String) + Clone + 'static,
) -> impl IntoElement {
    let active_name = database.clone();
    let press_name = database.clone();
    view()
        .h(px(28.0))
        .px_2()
        .items_center()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(database.clone())
        .bg(move || {
            if selected.get().as_deref() == Some(active_name.as_str()) {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(move || theme().muted))
        .on_press(move || choose(press_name.clone()))
        .child(
            text(database)
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

fn document_row(
    index: usize,
    document: Document,
    selected: Signal<Option<usize>>,
    editor: Signal<String>,
    status: Signal<String>,
) -> impl IntoElement {
    let label = document
        .get("_id")
        .map(|id| format!("{id:?}"))
        .unwrap_or_else(|| format!("#{}", index + 1));
    let editor_value = bson_to_pretty_json(&Bson::Document(document))
        .unwrap_or_else(|_| format!("{{\"_id\": {label:?}}}"));
    let selected_label = label.clone();
    view()
        .h(px(32.0))
        .px_2()
        .items_center()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(label.clone())
        .bg(move || {
            if selected.get() == Some(index) {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(move || theme().muted))
        .on_press(move || {
            selected.set(Some(index));
            editor.set(editor_value.clone());
            status.set(format!("Selected document {selected_label}"));
        })
        .child(text(label).text_xs().text_color(move || theme().foreground))
}

#[allow(clippy::too_many_arguments)]
fn collection_row(
    collection: String,
    selected_collection: Signal<Option<String>>,
    selected_database: Signal<Option<String>>,
    session: Signal<Option<MongoSession>>,
    filter: Signal<String>,
    limit: Signal<String>,
    documents: Signal<String>,
    document_list: Signal<Vec<Document>>,
    selected_document: Signal<Option<usize>>,
    document_editor: Signal<String>,
    count: Signal<i64>,
    status: Signal<String>,
    busy: Signal<bool>,
) -> impl IntoElement {
    let active_name = collection.clone();
    let press_name = collection.clone();
    view()
        .h(px(28.0))
        .px_2()
        .items_center()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(collection.clone())
        .bg(move || {
            if selected_collection.get().as_deref() == Some(active_name.as_str()) {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(move || theme().muted))
        .on_press(move || {
            selected_collection.set(Some(press_name.clone()));
            let Some(database) = selected_database.get_untracked() else {
                status.set("Select a database first".to_owned());
                return;
            };
            let filter = match parse_filter(&filter.get_untracked()) {
                Ok(filter) => filter,
                Err(error) => {
                    status.set(format!("Invalid filter: {error:#}"));
                    return;
                }
            };
            let limit = match parse_limit(&limit.get_untracked()) {
                Ok(limit) => limit,
                Err(error) => {
                    status.set(format!("Invalid limit: {error:#}"));
                    return;
                }
            };
            let Some(session) = session.get_untracked() else {
                status.set("Connect to MongoDB first".to_owned());
                return;
            };
            load_documents(
                session,
                database,
                press_name.clone(),
                filter,
                limit,
                documents,
                document_list,
                selected_document,
                document_editor,
                count,
                status,
                busy,
            );
        })
        .child(
            text(collection)
                .text_sm()
                .text_color(move || theme().foreground),
        )
}

fn load_collections(
    session: MongoSession,
    database: String,
    collections: Signal<Vec<String>>,
    selected_collection: Signal<Option<String>>,
    status: Signal<String>,
    busy: Signal<bool>,
) {
    busy.set(true);
    selected_collection.set(None);
    status.set(format!("Loading collections from {database}…"));
    spawn(async move {
        let state = session.state;
        let id = session.id;
        let database_for_query = database.clone();
        let loaded = background(async move {
            let connection = state
                .get_connection(&id)
                .ok_or_else(|| anyhow::anyhow!("MongoDB connection is not cached"))?;
            let found = connection
                .read()
                .await
                .list_collections(&database_for_query)
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            Ok::<Vec<String>, anyhow::Error>(found)
        })
        .await;
        busy.set(false);
        match loaded {
            Ok(Ok(found)) => {
                collections.set(found);
                status.set(format!("Collections in {database}"));
            }
            Ok(Err(error)) => status.set(format!("Failed to load collections: {error}")),
            Err(error) => status.set(format!("Collection task failed: {error}")),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn load_documents(
    session: MongoSession,
    database: String,
    collection: String,
    filter: Option<Document>,
    limit: i64,
    documents: Signal<String>,
    document_list: Signal<Vec<Document>>,
    selected_document: Signal<Option<usize>>,
    document_editor: Signal<String>,
    count: Signal<i64>,
    status: Signal<String>,
    busy: Signal<bool>,
) {
    busy.set(true);
    status.set(format!("Querying {database}.{collection}…"));
    spawn(async move {
        let state = session.state;
        let id = session.id;
        let database_for_query = database.clone();
        let collection_for_query = collection.clone();
        let options = MongoFindOptions {
            limit: Some(limit),
            skip: None,
            sort: None,
            projection: None,
        };
        let filter = filter.clone();
        let loaded = background(async move {
            let connection = state
                .get_connection(&id)
                .ok_or_else(|| anyhow::anyhow!("MongoDB connection is not cached"))?;
            let guard = connection.read().await;
            let documents = guard
                .find_documents(
                    &database_for_query,
                    &collection_for_query,
                    filter.clone(),
                    options,
                )
                .await
                .map_err(|error| anyhow::anyhow!("{error}"))?;
            let total = guard
                .count_documents(&database_for_query, &collection_for_query, filter)
                .await
                .unwrap_or(documents.len() as i64);
            Ok::<(Vec<Document>, i64), anyhow::Error>((documents, total))
        })
        .await;
        busy.set(false);
        match loaded {
            Ok(Ok((found, total))) => {
                documents.set(documents_json(&found));
                document_list.set(found);
                selected_document.set(None);
                document_editor.set("{\n\n}\n".to_owned());
                count.set(total);
                status.set("Query complete".to_owned());
            }
            Ok(Err(error)) => status.set(format!("Query failed: {error:#}")),
            Err(error) => status.set(format!("Query task failed: {error}")),
        }
    });
}

fn parse_filter(text: &str) -> anyhow::Result<Option<Document>> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let value: serde_json::Value = serde_json::from_str(text).context("parse JSON")?;
    match Bson::try_from(value).context("convert JSON to BSON")? {
        Bson::Document(document) => Ok(Some(document)),
        _ => bail!("filter must be a JSON object"),
    }
}

fn parse_document(text: &str) -> anyhow::Result<Document> {
    let text = text.trim();
    if text.is_empty() {
        bail!("document JSON is empty");
    }
    let value: serde_json::Value = serde_json::from_str(text).context("parse document JSON")?;
    match Bson::try_from(value).context("convert document JSON to BSON")? {
        Bson::Document(document) => Ok(document),
        _ => bail!("document must be a JSON object"),
    }
}

fn parse_pipeline(text: &str) -> anyhow::Result<Vec<Document>> {
    let value: serde_json::Value =
        serde_json::from_str(text.trim()).context("parse aggregation pipeline JSON")?;
    let serde_json::Value::Array(stages) = value else {
        bail!("aggregation pipeline must be a JSON array");
    };
    stages
        .into_iter()
        .enumerate()
        .map(|(index, stage)| {
            match Bson::try_from(stage).context("convert pipeline stage to BSON")? {
                Bson::Document(document) => Ok(document),
                _ => bail!("pipeline stage {} must be an object", index + 1),
            }
        })
        .collect()
}

fn documents_json(documents: &[Document]) -> String {
    let array = Bson::Array(
        documents
            .iter()
            .cloned()
            .map(Bson::Document)
            .collect::<Vec<_>>(),
    );
    bson_to_pretty_json(&array)
        .unwrap_or_else(|error| format!("Failed to format documents: {error}"))
}

fn parse_limit(text: &str) -> anyhow::Result<i64> {
    let limit = text.trim().parse::<i64>().context("parse integer")?;
    if limit <= 0 {
        bail!("limit must be greater than zero");
    }
    Ok(limit.min(500))
}

fn mongo_connections(services: Option<&NavopNshServices>) -> Vec<StoredConnection> {
    services
        .map(|services| services.resolved_connections.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|connection| connection.connection_type == ConnectionType::MongoDB)
        .collect()
}

trait ConnectionId {
    fn id_string(&self) -> String;
}

impl ConnectionId for StoredConnection {
    fn id_string(&self) -> String {
        self.id.unwrap_or_default().to_string()
    }
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
    fn filter_requires_a_json_object() {
        assert!(parse_filter("{}").unwrap().is_some());
        assert!(parse_filter("").unwrap().is_none());
        assert!(parse_filter("[]").is_err());
        assert!(parse_filter("{").is_err());
    }

    #[test]
    fn limits_are_positive_and_bounded() {
        assert_eq!(parse_limit("25").unwrap(), 25);
        assert_eq!(parse_limit("10000").unwrap(), 500);
        assert!(parse_limit("0").is_err());
    }

    #[test]
    fn document_and_pipeline_json_require_the_expected_shapes() {
        assert_eq!(
            parse_document("{\"name\":\"Ada\"}")
                .unwrap()
                .get_str("name")
                .unwrap(),
            "Ada"
        );
        assert!(parse_document("[]").is_err());
        assert_eq!(
            parse_pipeline("[{\"$match\":{\"active\":true}}]")
                .unwrap()
                .len(),
            1
        );
        assert!(parse_pipeline("{}").is_err());
        assert!(parse_pipeline("[1]").is_err());
    }
}
