use std::sync::Arc;

use nsh_base::{input, theme};
use nsh_core::prelude::*;
use one_core::storage::{ConnectionType, StoredConnection};
use redis_runtime::{KeyValueContent, KeyValueDetail, RedisConnection, RedisConnectionImpl};
use redis_view::RedisManager;
use tokio::sync::RwLock;

use crate::{services::NavopNshServices, ui};

#[derive(Clone)]
struct RedisSession(Arc<RwLock<Box<dyn RedisConnection>>>);

impl RedisSession {
    fn connection(&self) -> Arc<RwLock<Box<dyn RedisConnection>>> {
        self.0.clone()
    }
}

impl PartialEq for RedisSession {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

pub(crate) fn redis() -> impl IntoElement {
    let services = NavopNshServices::current();
    let connections = redis_connections(services.as_deref());
    let requested_connection =
        crate::route::take_connection_request(crate::route::Route::Redis).map(|id| id.to_string());
    let selected_connection = signal(requested_connection.clone());
    let selected_key = signal(None::<String>);
    let keys = signal(Vec::<String>::new());
    let pattern = signal(String::from("*"));
    let cursor = signal(0u64);
    let has_more = signal(false);
    let db_index = signal(0u8);
    let db_input = signal(String::from("0"));
    let detail = signal(String::from("Select a key to inspect its value"));
    let cli_command = signal(String::new());
    let cli_output = signal(String::from("Redis CLI output will appear here"));
    let status = signal(String::from("Choose a Redis connection"));
    let session = signal(None::<RedisSession>);
    let busy = signal(false);

    let connect = {
        let connections = connections.clone();
        move |id: String| {
            let Some(connection) = connections
                .iter()
                .find(|connection| connection.id_string() == id)
                .cloned()
            else {
                status.set("Redis connection was not found".to_owned());
                return;
            };
            let config = match RedisManager::config_from_stored(&connection) {
                Ok(config) => config,
                Err(error) => {
                    status.set(format!("Invalid Redis connection: {error}"));
                    return;
                }
            };
            let name = connection.name;
            selected_connection.set(Some(id));
            selected_key.set(None);
            keys.set(Vec::new());
            detail.set("Select a key to inspect its value".to_owned());
            db_index.set(config.db_index);
            busy.set(true);
            status.set(format!("Connecting to {name}…"));
            spawn(async move {
                let connected = background(async move {
                    let mut connection = RedisConnectionImpl::new(config);
                    connection.connect().await?;
                    Ok::<RedisSession, anyhow::Error>(RedisSession(Arc::new(RwLock::new(
                        Box::new(connection),
                    ))))
                })
                .await;
                busy.set(false);
                match connected {
                    Ok(Ok(redis_session)) => {
                        session.set(Some(redis_session.clone()));
                        status.set(format!("Connected to {name}"));
                        scan_keys(
                            redis_session,
                            db_index.get_untracked(),
                            0,
                            pattern.get_untracked(),
                            keys,
                            cursor,
                            has_more,
                            status,
                            busy,
                            false,
                        );
                    }
                    Ok(Err(error)) => status.set(format!("Connection failed: {error:#}")),
                    Err(error) => status.set(format!("Connection task failed: {error}")),
                }
            });
        }
    };
    let rescan = move || {
        let Some(redis_session) = session.get_untracked() else {
            status.set("Connect to Redis first".to_owned());
            return;
        };
        selected_key.set(None);
        scan_keys(
            redis_session,
            db_index.get_untracked(),
            0,
            pattern.get_untracked(),
            keys,
            cursor,
            has_more,
            status,
            busy,
            false,
        );
    };
    if let Some(connection_id) = requested_connection {
        connect(connection_id);
    }
    let load_more = move || {
        let Some(redis_session) = session.get_untracked() else {
            return;
        };
        if has_more.get_untracked() && !busy.get_untracked() {
            scan_keys(
                redis_session,
                db_index.get_untracked(),
                cursor.get_untracked(),
                pattern.get_untracked(),
                keys,
                cursor,
                has_more,
                status,
                busy,
                true,
            );
        }
    };
    let switch_db = move || {
        let Some(redis_session) = session.get_untracked() else {
            status.set("Connect to Redis first".to_owned());
            return;
        };
        let next_db = match parse_db_index(&db_input.get_untracked()) {
            Ok(index) => index,
            Err(error) => {
                status.set(error);
                return;
            }
        };
        db_index.set(next_db);
        db_input.set(next_db.to_string());
        selected_key.set(None);
        detail.set("Select a key to inspect its value".to_owned());
        scan_keys(
            redis_session,
            next_db,
            0,
            pattern.get_untracked(),
            keys,
            cursor,
            has_more,
            status,
            busy,
            false,
        );
    };
    let delete_selected = move || {
        let (Some(redis_session), Some(key)) =
            (session.get_untracked(), selected_key.get_untracked())
        else {
            status.set("Select a key to delete".to_owned());
            return;
        };
        busy.set(true);
        status.set(format!("Deleting {key}…"));
        spawn(async move {
            let connection = redis_session.connection();
            let db = db_index.get_untracked();
            let key_for_delete = key.clone();
            let deleted = background(async move {
                connection
                    .read()
                    .await
                    .del_in_db(db, &[key_for_delete.as_str()])
                    .await
            })
            .await;
            busy.set(false);
            match deleted {
                Ok(Ok(count)) => {
                    selected_key.set(None);
                    detail.set("Select a key to inspect its value".to_owned());
                    status.set(format!("Deleted {count} key(s)"));
                    scan_keys(
                        redis_session,
                        db,
                        0,
                        pattern.get_untracked(),
                        keys,
                        cursor,
                        has_more,
                        status,
                        busy,
                        false,
                    );
                }
                Ok(Err(error)) => status.set(format!("Delete failed: {error}")),
                Err(error) => status.set(format!("Delete task failed: {error}")),
            }
        });
    };
    let run_cli = move || {
        let Some(redis_session) = session.get_untracked() else {
            status.set("Connect to Redis first".to_owned());
            return;
        };
        let command = cli_command.get_untracked().trim().to_owned();
        if command.is_empty() {
            return;
        }
        busy.set(true);
        status.set(format!("Running {command}…"));
        spawn(async move {
            let connection = redis_session.connection();
            let db = db_index.get_untracked();
            let command_for_run = command.clone();
            let result = background(async move {
                connection
                    .read()
                    .await
                    .execute_command_in_db(db, &command_for_run)
                    .await
            })
            .await;
            busy.set(false);
            match result {
                Ok(Ok(value)) => {
                    cli_output.set(value.to_display_string());
                    status.set(format!("Executed in DB {db}"));
                }
                Ok(Err(error)) => {
                    cli_output.set(format!("ERR {error}"));
                    status.set("Command failed".to_owned());
                }
                Err(error) => {
                    cli_output.set(format!("Task failed: {error}"));
                    status.set("Command task failed".to_owned());
                }
            }
        });
    };
    let disconnect = move || {
        let current = session.get_untracked();
        session.set(None);
        if let Some(session) = current {
            let connection = session.connection();
            let _ = background(async move {
                let mut connection = connection.write().await;
                let _ = connection.disconnect().await;
            });
        }
        keys.set(Vec::new());
        selected_key.set(None);
        status.set("Disconnected".to_owned());
    };

    cleanup(move || {
        if let Some(session) = session.get_untracked() {
            let connection = session.connection();
            let _ = background(async move {
                let mut connection = connection.write().await;
                let _ = connection.disconnect().await;
            });
        }
    });

    view()
        .size_full()
        .child(ui::page_header(
            "Redis",
            "nsh key browser over the shared Redis runtime",
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
                    text("DB")
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    input()
                        .value(move || db_input.get())
                        .on_change(move |value: &str| db_input.set(value.to_owned()))
                        .on_submit(move |_: &str| switch_db())
                        .w(px(54.0))
                        .px_2()
                        .py_1()
                        .rounded(move || theme().radius)
                        .border_1()
                        .border_color(move || theme().input)
                        .text_size(px(12.0))
                        .text_color(move || theme().foreground),
                )
                .child(
                    ui::secondary_button("Switch DB")
                        .disabled(move || session.get().is_none())
                        .on_press(switch_db),
                )
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
                        .w(px(360.0))
                        .h_full()
                        .min_h_0()
                        .flex_none()
                        .p_3()
                        .gap_2()
                        .border_r_1()
                        .border_r_color(move || theme().border)
                        .child(
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(
                                    input()
                                        .value(move || pattern.get())
                                        .on_change(move |value: &str| pattern.set(value.to_owned()))
                                        .on_submit(move |_: &str| rescan())
                                        .placeholder("SCAN pattern, e.g. user:*")
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
                                    ui::primary_button("Scan")
                                        .disabled(move || busy.get())
                                        .on_press(rescan),
                                ),
                        )
                        .child(scroll().flex_1().min_h_0().gap_1().children(
                            keys.with(|keys| keys.clone()).into_iter().map({
                                move |key| {
                                    key_row(
                                        key,
                                        selected_key,
                                        session,
                                        db_index,
                                        detail,
                                        status,
                                        busy,
                                    )
                                }
                            }),
                        ))
                        .child(
                            ui::secondary_button("Load more")
                                .disabled(move || !has_more.get() || busy.get())
                                .on_press(load_more),
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
                                .gap_3()
                                .child(label("VALUE"))
                                .child(
                                    text(move || status.get())
                                        .flex_1()
                                        .text_xs()
                                        .text_color(move || theme().muted_foreground),
                                )
                                .child(
                                    ui::secondary_button("Delete key")
                                        .disabled(move || {
                                            selected_key.get().is_none() || busy.get()
                                        })
                                        .on_press(delete_selected),
                                ),
                        )
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
                                    text(move || detail.get())
                                        .text_size(px(13.0))
                                        .text_color(move || theme().foreground),
                                ),
                        )
                        .child(
                            view()
                                .h(px(150.0))
                                .flex_none()
                                .gap_1()
                                .child(
                                    view()
                                        .flex_row()
                                        .items_center()
                                        .gap_2()
                                        .child(label("CLI"))
                                        .child(
                                            input()
                                                .value(move || cli_command.get())
                                                .on_change(move |value: &str| {
                                                    cli_command.set(value.to_owned())
                                                })
                                                .on_submit(move |_: &str| run_cli())
                                                .placeholder("Redis command, e.g. INFO server")
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
                                            ui::primary_button("Run")
                                                .disabled(move || busy.get())
                                                .on_press(run_cli),
                                        ),
                                )
                                .child(
                                    scroll()
                                        .flex_1()
                                        .min_h_0()
                                        .p_2()
                                        .rounded(move || theme().radius)
                                        .border_1()
                                        .border_color(move || theme().border)
                                        .child(
                                            text(move || cli_output.get())
                                                .text_size(px(12.0))
                                                .text_color(move || theme().muted_foreground),
                                        ),
                                ),
                        ),
                ),
        )
}

#[allow(clippy::too_many_arguments)]
fn key_row(
    key: String,
    selected_key: Signal<Option<String>>,
    session: Signal<Option<RedisSession>>,
    db_index: Signal<u8>,
    detail: Signal<String>,
    status: Signal<String>,
    busy: Signal<bool>,
) -> impl IntoElement {
    let active_key = key.clone();
    let press_key = key.clone();
    view()
        .h(px(30.0))
        .px_2()
        .items_center()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(key.clone())
        .bg(move || {
            if selected_key.get().as_deref() == Some(active_key.as_str()) {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(move || theme().muted))
        .on_press(move || {
            let Some(session) = session.get_untracked() else {
                return;
            };
            selected_key.set(Some(press_key.clone()));
            load_key_detail(
                session,
                db_index.get_untracked(),
                press_key.clone(),
                detail,
                status,
                busy,
            );
        })
        .child(text(key).text_sm().text_color(move || theme().foreground))
}

#[allow(clippy::too_many_arguments)]
fn scan_keys(
    session: RedisSession,
    db: u8,
    scan_cursor: u64,
    pattern: String,
    keys: Signal<Vec<String>>,
    cursor: Signal<u64>,
    has_more: Signal<bool>,
    status: Signal<String>,
    busy: Signal<bool>,
    append: bool,
) {
    busy.set(true);
    if !append {
        keys.set(Vec::new());
    }
    status.set(format!("SCAN {pattern}…"));
    spawn(async move {
        let connection = session.connection();
        let result = background(async move {
            connection
                .read()
                .await
                .scan_in_db(db, scan_cursor, &pattern, 200)
                .await
        })
        .await;
        busy.set(false);
        match result {
            Ok(Ok(result)) => {
                if append {
                    keys.update(|existing| existing.extend(result.keys));
                } else {
                    keys.set(result.keys);
                }
                cursor.set(result.cursor);
                has_more.set(!result.finished);
                status.set(format!("{} key(s) loaded", keys.with(|keys| keys.len())));
            }
            Ok(Err(error)) => status.set(format!("SCAN failed: {error}")),
            Err(error) => status.set(format!("SCAN task failed: {error}")),
        }
    });
}

fn load_key_detail(
    session: RedisSession,
    db: u8,
    key: String,
    detail: Signal<String>,
    status: Signal<String>,
    busy: Signal<bool>,
) {
    busy.set(true);
    status.set(format!("Loading {key}…"));
    spawn(async move {
        let connection = session.connection();
        let key_for_query = key.clone();
        let loaded = background(async move {
            connection
                .read()
                .await
                .get_key_value_detail_in_db(db, &key_for_query)
                .await
        })
        .await;
        busy.set(false);
        match loaded {
            Ok(Ok(value)) => {
                detail.set(format_detail(&value));
                status.set(key);
            }
            Ok(Err(error)) => status.set(format!("Failed to load key: {error}")),
            Err(error) => status.set(format!("Key task failed: {error}")),
        }
    });
}

fn redis_connections(services: Option<&NavopNshServices>) -> Vec<StoredConnection> {
    services
        .map(|services| services.resolved_connections.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|connection| connection.connection_type == ConnectionType::Redis)
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

fn format_detail(detail: &KeyValueDetail) -> String {
    let mut output = format!(
        "Key: {}\nType: {}\nTTL: {}\nSize: {}\n\n",
        detail.key_info.name,
        detail.key_info.key_type.display_name(),
        detail.key_info.ttl_display(),
        detail
            .key_info
            .size
            .map(|size| size.to_string())
            .unwrap_or_else(|| "?".to_owned())
    );
    match &detail.value {
        KeyValueContent::String(bytes) => output.push_str(&lossy(bytes)),
        KeyValueContent::List(values) => {
            for (index, value) in values.iter().enumerate() {
                output.push_str(&format!("{index}: {}\n", lossy(value)));
            }
        }
        KeyValueContent::Set(values) => {
            for value in values {
                output.push_str(&format!("{}\n", lossy(value)));
            }
        }
        KeyValueContent::ZSet(values) => {
            for value in values {
                output.push_str(&format!("{}  {}\n", value.score, lossy(&value.member)));
            }
        }
        KeyValueContent::Hash(fields) => {
            for field in fields {
                output.push_str(&format!(
                    "{} = {}\n",
                    lossy(&field.field),
                    lossy(&field.value)
                ));
            }
        }
        KeyValueContent::Stream(entries) => {
            for entry in entries {
                output.push_str(&format!("{}\n", entry.id));
                for field in &entry.fields {
                    output.push_str(&format!(
                        "  {} = {}\n",
                        lossy(&field.field),
                        lossy(&field.value)
                    ));
                }
            }
        }
        KeyValueContent::None => output.push_str("(nil)"),
    }
    if detail.truncated {
        output.push_str("\n\n… truncated by the Redis runtime budget");
    }
    output
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn parse_db_index(value: &str) -> Result<u8, String> {
    value
        .trim()
        .parse::<u8>()
        .map_err(|_| "Redis database must be between 0 and 255".to_owned())
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
    use redis_runtime::{HashField, KeyInfo, RedisKeyType, ZSetMember};

    #[test]
    fn hash_details_are_rendered_as_readable_lines() {
        let detail = KeyValueDetail {
            key_info: KeyInfo {
                name: "user:1".to_owned(),
                key_type: RedisKeyType::Hash,
                ttl: -1,
                size: Some(2),
                memory_usage: None,
            },
            value: KeyValueContent::Hash(vec![
                HashField {
                    field: b"name".to_vec(),
                    value: b"Ada".to_vec(),
                },
                HashField {
                    field: b"role".to_vec(),
                    value: b"admin".to_vec(),
                },
            ]),
            truncated: false,
        };
        let text = format_detail(&detail);
        assert!(text.contains("user:1"));
        assert!(text.contains("name = Ada"));
        assert!(text.contains("role = admin"));
    }

    #[test]
    fn sorted_sets_include_scores() {
        let detail = KeyValueDetail {
            key_info: KeyInfo {
                name: "rank".to_owned(),
                key_type: RedisKeyType::ZSet,
                ttl: -1,
                size: Some(1),
                memory_usage: None,
            },
            value: KeyValueContent::ZSet(vec![ZSetMember {
                member: b"alice".to_vec(),
                score: 42.5,
            }]),
            truncated: false,
        };
        assert!(format_detail(&detail).contains("42.5  alice"));
    }

    #[test]
    fn database_indexes_are_validated_as_u8() {
        assert_eq!(parse_db_index(" 15 ").unwrap(), 15);
        assert!(parse_db_index("256").is_err());
        assert!(parse_db_index("-1").is_err());
    }
}
