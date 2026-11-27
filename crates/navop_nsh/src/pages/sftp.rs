use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

use nsh_base::theme;
use nsh_core::prelude::*;
use one_core::storage::{
    ConnectionType, SftpFavoritePathRepository, StoredConnection, normalize_sftp_favorite_path,
    sftp_favorite_connection_key,
};
use sftp::{
    DirectoryConflictPolicy, FileEntry as RemoteFileEntry, ProgressCallback, RemoteFileClient,
    SharedRemoteFileClient, TransferProgress,
};

use crate::{services::NavopNshServices, ui};

#[derive(Clone)]
struct RemoteSession(SharedRemoteFileClient);

impl RemoteSession {
    fn client(&self) -> SharedRemoteFileClient {
        self.0.clone()
    }
}

impl PartialEq for RemoteSession {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Clone, PartialEq)]
struct LocalEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
}

#[derive(Clone, PartialEq)]
struct RemoteEntry {
    name: String,
    path: String,
    is_dir: bool,
    size: u64,
    modified: SystemTime,
}

impl From<RemoteFileEntry> for RemoteEntry {
    fn from(entry: RemoteFileEntry) -> Self {
        Self {
            name: entry.name,
            path: entry.path,
            is_dir: entry.is_dir,
            size: entry.size,
            modified: entry.modified,
        }
    }
}

#[derive(Clone, Default, PartialEq)]
struct TransferState {
    transferred: u64,
    total: u64,
    speed: f64,
    current_file: Option<String>,
}

#[derive(Clone)]
struct TransferCancel(Arc<AtomicBool>);

impl PartialEq for TransferCancel {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

pub(crate) fn sftp() -> impl IntoElement {
    let services = NavopNshServices::current();
    let connections = remote_connections(services.as_deref());
    let favorites_repository = services
        .as_ref()
        .and_then(|services| services.sftp_favorites.clone());
    let requested_connection =
        crate::route::take_connection_request(crate::route::Route::Sftp).map(|id| id.to_string());
    let selected_connection = signal(requested_connection.clone());
    let local_path = signal(home_directory());
    let local_entries = signal(load_local_entries(&local_path.get_untracked()));
    let selected_local = signal(None::<String>);
    let remote_path = signal(String::from("/"));
    let remote_entries = signal(Vec::<RemoteEntry>::new());
    let selected_remote = signal(None::<String>);
    let session = signal(None::<RemoteSession>);
    let status = signal(String::from("Choose a remote connection"));
    let busy = signal(false);
    let directory_policy = signal(DirectoryConflictPolicy::Merge);
    let favorite_paths = signal(Vec::<String>::new());
    let favorite_key = signal(String::new());
    let favorite_connection_id = signal(None::<i64>);
    let transfer_state = signal(TransferState::default());
    let transfer_cancel = signal(None::<TransferCancel>);
    let transfer_progress = Arc::new(Mutex::new(TransferState::default()));
    install_transfer_progress_pump(transfer_progress.clone(), transfer_state);

    let connect = {
        let connections = connections.clone();
        let favorites_repository = favorites_repository.clone();
        move |id: String| {
            let Some(connection) = connections
                .iter()
                .find(|connection| connection.id_string() == id)
                .cloned()
            else {
                status.set("Connection was not found".to_owned());
                return;
            };
            favorite_paths.set(load_favorite_paths(
                favorites_repository.as_ref(),
                &connection,
            ));
            favorite_key.set(sftp_favorite_connection_key(&connection));
            favorite_connection_id.set(connection.id);
            selected_connection.set(Some(id));
            selected_remote.set(None);
            remote_entries.set(Vec::new());
            busy.set(true);
            status.set(format!("Connecting to {}…", connection.name));
            spawn(async move {
                let connection_name = connection.name.clone();
                let connected =
                    background(
                        async move { sftp_view::connect_remote_file_client(&connection).await },
                    )
                    .await;
                busy.set(false);
                match connected {
                    Ok(Ok(client)) => {
                        let mut locked = client.lock().await;
                        let initial = locked
                            .realpath(".")
                            .await
                            .unwrap_or_else(|_| "/".to_owned());
                        match locked.list_dir(&initial).await {
                            Ok(entries) => {
                                drop(locked);
                                session.set(Some(RemoteSession(client)));
                                remote_path.set(initial);
                                remote_entries.set(entries.into_iter().map(Into::into).collect());
                                status.set(format!("Connected to {connection_name}"));
                            }
                            Err(error) => {
                                let _ = locked.disconnect().await;
                                status.set(format!("Failed to list remote home: {error:#}"));
                            }
                        }
                    }
                    Ok(Err(error)) => status.set(format!("Connection failed: {error:#}")),
                    Err(error) => status.set(format!("Connection task failed: {error}")),
                }
            });
        }
    };
    let refresh_remote = {
        move || {
            if let Some(session) = session.get_untracked() {
                load_remote_directory(
                    session,
                    remote_path.get_untracked(),
                    remote_entries,
                    remote_path,
                    selected_remote,
                    status,
                    busy,
                );
            }
        }
    };
    if let Some(connection_id) = requested_connection {
        connect(connection_id);
    }
    let remote_up = {
        move || {
            let Some(session) = session.get_untracked() else {
                return;
            };
            let parent = remote_parent(&remote_path.get_untracked());
            load_remote_directory(
                session,
                parent,
                remote_entries,
                remote_path,
                selected_remote,
                status,
                busy,
            );
        }
    };
    let local_up = move || {
        let path = PathBuf::from(local_path.get_untracked());
        let Some(parent) = path.parent() else {
            return;
        };
        let parent = parent.display().to_string();
        local_entries.set(load_local_entries(&parent));
        local_path.set(parent);
        selected_local.set(None);
    };
    let refresh_local = move || {
        let path = local_path.get_untracked();
        local_entries.set(load_local_entries(&path));
        selected_local.set(None);
    };
    let upload = {
        let transfer_progress = transfer_progress.clone();
        move || {
            let (Some(session), Some(local_file)) =
                (session.get_untracked(), selected_local.get_untracked())
            else {
                status.set("Select a local file to upload".to_owned());
                return;
            };
            let destination = remote_join(
                &remote_path.get_untracked(),
                Path::new(&local_file)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default(),
            );
            transfer(
                session,
                status,
                busy,
                remote_entries,
                remote_path,
                selected_remote,
                transfer_progress.clone(),
                transfer_cancel,
                true,
                local_file,
                destination,
            );
        }
    };
    let download = {
        let transfer_progress = transfer_progress.clone();
        move || {
            let (Some(session), Some(remote_file)) =
                (session.get_untracked(), selected_remote.get_untracked())
            else {
                status.set("Select a remote file to download".to_owned());
                return;
            };
            let destination = Path::new(&local_path.get_untracked())
                .join(
                    Path::new(&remote_file)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or_default(),
                )
                .display()
                .to_string();
            transfer(
                session,
                status,
                busy,
                remote_entries,
                remote_path,
                selected_remote,
                transfer_progress.clone(),
                transfer_cancel,
                false,
                remote_file,
                destination,
            );
        }
    };
    let upload_directory = {
        let transfer_progress = transfer_progress.clone();
        move || {
            let Some(session) = session.get_untracked() else {
                status.set("Connect to a remote host first".to_owned());
                return;
            };
            let local_directory = local_path.get_untracked();
            let destination =
                upload_directory_destination(&local_directory, &remote_path.get_untracked());
            transfer_directory(
                session,
                status,
                busy,
                remote_entries,
                remote_path,
                selected_remote,
                transfer_progress.clone(),
                transfer_cancel,
                true,
                local_directory,
                destination,
                directory_policy.get_untracked(),
            );
        }
    };
    let download_directory = {
        let transfer_progress = transfer_progress.clone();
        move || {
            let Some(session) = session.get_untracked() else {
                status.set("Connect to a remote host first".to_owned());
                return;
            };
            let remote_directory = remote_path.get_untracked();
            let destination =
                download_directory_destination(&remote_directory, &local_path.get_untracked());
            transfer_directory(
                session,
                status,
                busy,
                remote_entries,
                remote_path,
                selected_remote,
                transfer_progress.clone(),
                transfer_cancel,
                false,
                remote_directory,
                destination,
                directory_policy.get_untracked(),
            );
        }
    };
    let disconnect = {
        move || {
            let current = session.get_untracked();
            session.set(None);
            if let Some(session) = current {
                let client = session.client();
                let _ = background(async move {
                    let mut client = client.lock().await;
                    let _ = client.disconnect().await;
                });
            }
            status.set("Disconnected".to_owned());
            favorite_paths.set(Vec::new());
            favorite_key.set(String::new());
            favorite_connection_id.set(None);
        }
    };
    let cancel_transfer = move || {
        if let Some(cancelled) = transfer_cancel.get_untracked() {
            cancelled.0.store(true, Ordering::Relaxed);
            status.set("Cancelling transfer…".to_owned());
        }
    };
    let toggle_favorite = {
        let repository = favorites_repository.clone();
        move || {
            let (Some(repository), Some(path)) = (
                repository.as_ref(),
                normalize_sftp_favorite_path(&remote_path.get_untracked()),
            ) else {
                status.set("Connect to a remote host first".to_owned());
                return;
            };
            let key = favorite_key.get_untracked();
            if key.is_empty() {
                status.set("Connect to a remote host first".to_owned());
                return;
            }
            let is_favorite = favorite_paths.with(|paths| paths.iter().any(|item| item == &path));
            let result = if is_favorite {
                repository.remove_path(&key, &path)
            } else {
                repository.add_path(favorite_connection_id.get_untracked(), &key, &path)
            };
            match result {
                Ok(_) => {
                    favorite_paths.set(repository.list_paths(&key).unwrap_or_default());
                    status.set(if is_favorite {
                        format!("Removed favorite {path}")
                    } else {
                        format!("Added favorite {path}")
                    });
                }
                Err(error) => status.set(format!("Favorite update failed: {error}")),
            }
        }
    };

    cleanup({
        move || {
            if let Some(cancelled) = transfer_cancel.get_untracked() {
                cancelled.0.store(true, Ordering::Relaxed);
            }
            if let Some(session) = session.get_untracked() {
                let client = session.client();
                let _ = background(async move {
                    let mut client = client.lock().await;
                    let _ = client.disconnect().await;
                });
            }
        }
    });

    view()
        .size_full()
        .child(ui::page_header(
            "SFTP",
            "nsh dual-pane local and remote file browser",
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
                .child(label("CONNECTION"))
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
                .child(
                    ui::secondary_button("Toggle favorite")
                        .disabled(move || favorite_key.get().is_empty())
                        .on_press(toggle_favorite),
                )
                .child(view().flex_1())
                .child(
                    ui::secondary_button("Disconnect")
                        .disabled(move || session.get().is_none())
                        .on_press(disconnect),
                ),
        )
        .child(
            view()
                .flex_none()
                .flex_row()
                .items_center()
                .flex_wrap()
                .gap_1()
                .px_4()
                .py_2()
                .border_b_1()
                .border_b_color(move || theme().border)
                .child(label("FAVORITES"))
                .when(favorite_paths.with(Vec::is_empty), |row| {
                    row.child(
                        text("No saved remote directories")
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                    )
                })
                .children(favorite_paths.with(|paths| paths.clone()).into_iter().map({
                    move |path| {
                        favorite_button(
                            path,
                            remote_path,
                            session,
                            remote_entries,
                            selected_remote,
                            status,
                            busy,
                        )
                    }
                })),
        )
        .child(
            view()
                .flex_1()
                .min_h_0()
                .flex_row()
                .child(file_pane(
                    "LOCAL",
                    local_path,
                    selected_local,
                    local_entries,
                    local_up,
                    refresh_local,
                ))
                .child(
                    view()
                        .w(px(128.0))
                        .h_full()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .gap_3()
                        .border_l_1()
                        .border_r_1()
                        .border_color(move || theme().border)
                        .child(
                            ui::primary_button("Upload →")
                                .disabled(move || busy.get())
                                .on_press(upload),
                        )
                        .child(
                            ui::secondary_button("← Download")
                                .disabled(move || busy.get())
                                .on_press(download),
                        )
                        .child(
                            ui::primary_button("Upload dir")
                                .disabled(move || busy.get())
                                .on_press(upload_directory),
                        )
                        .child(
                            ui::secondary_button("Download dir")
                                .disabled(move || busy.get())
                                .on_press(download_directory),
                        )
                        .child(
                            ui::secondary_button("Cancel")
                                .disabled(move || transfer_cancel.get().is_none())
                                .on_press(cancel_transfer),
                        )
                        .child(
                            text(move || transfer_progress_text(&transfer_state.get()))
                                .text_size(px(10.0))
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            view()
                                .flex_row()
                                .gap_1()
                                .child(policy_button(
                                    "Merge",
                                    DirectoryConflictPolicy::Merge,
                                    directory_policy,
                                ))
                                .child(policy_button(
                                    "Replace",
                                    DirectoryConflictPolicy::Replace,
                                    directory_policy,
                                )),
                        )
                        .child(
                            text(move || status.get())
                                .text_size(px(10.0))
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(remote_pane(
                    remote_path,
                    selected_remote,
                    remote_entries,
                    session,
                    remote_up,
                    refresh_remote,
                    status,
                    busy,
                )),
        )
}

fn file_pane(
    title: &'static str,
    path: Signal<String>,
    selected: Signal<Option<String>>,
    entries: Signal<Vec<LocalEntry>>,
    up: impl Fn() + Clone + 'static,
    refresh: impl Fn() + Clone + 'static,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w_0()
        .h_full()
        .min_h_0()
        .p_3()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(label(title))
                .child(
                    text(move || path.get())
                        .flex_1()
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(ui::secondary_button("Up").on_press(up))
                .child(ui::secondary_button("Refresh").on_press(refresh)),
        )
        .child(
            scroll().flex_1().min_h_0().gap_1().children(
                entries
                    .with(|entries| entries.clone())
                    .into_iter()
                    .map(move |entry| local_entry_row(entry, path, selected, entries)),
            ),
        )
}

#[allow(clippy::too_many_arguments)]
fn remote_pane(
    path: Signal<String>,
    selected: Signal<Option<String>>,
    entries: Signal<Vec<RemoteEntry>>,
    session: Signal<Option<RemoteSession>>,
    up: impl Fn() + Clone + 'static,
    refresh: impl Fn() + Clone + 'static,
    status: Signal<String>,
    busy: Signal<bool>,
) -> impl IntoElement {
    view()
        .flex_1()
        .min_w_0()
        .h_full()
        .min_h_0()
        .p_3()
        .gap_2()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(label("REMOTE"))
                .child(
                    text(move || path.get())
                        .flex_1()
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(ui::secondary_button("Up").on_press(up))
                .child(ui::secondary_button("Refresh").on_press(refresh)),
        )
        .child(
            scroll().flex_1().min_h_0().gap_1().children(
                entries
                    .with(|entries| entries.clone())
                    .into_iter()
                    .map(move |entry| {
                        remote_entry_row(entry, path, selected, entries, session, status, busy)
                    }),
            ),
        )
}

fn local_entry_row(
    entry: LocalEntry,
    current_path: Signal<String>,
    selected: Signal<Option<String>>,
    entries: Signal<Vec<LocalEntry>>,
) -> impl IntoElement {
    let row_path = entry.path.clone();
    let press_path = entry.path.clone();
    let is_dir = entry.is_dir;
    view()
        .h(px(30.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(entry.name.clone())
        .bg(move || {
            if selected.get().as_deref() == Some(row_path.as_str()) {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(move || theme().muted))
        .on_press(move || {
            if is_dir {
                entries.set(load_local_entries(&press_path));
                current_path.set(press_path.clone());
                selected.set(None);
            } else {
                selected.set(Some(press_path.clone()));
            }
        })
        .child(
            text(if entry.is_dir { "DIR" } else { "FILE" })
                .text_size(px(9.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(entry.name)
                .flex_1()
                .text_sm()
                .text_color(move || theme().foreground),
        )
        .child(
            text(if entry.is_dir {
                String::new()
            } else {
                format_size(entry.size)
            })
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
}

fn remote_entry_row(
    entry: RemoteEntry,
    current_path: Signal<String>,
    selected: Signal<Option<String>>,
    entries: Signal<Vec<RemoteEntry>>,
    session: Signal<Option<RemoteSession>>,
    status: Signal<String>,
    busy: Signal<bool>,
) -> impl IntoElement {
    let row_path = entry.path.clone();
    let press_path = entry.path.clone();
    let is_dir = entry.is_dir;
    view()
        .h(px(30.0))
        .flex_row()
        .items_center()
        .gap_2()
        .px_2()
        .rounded(move || theme().radius)
        .role(AccessibilityRole::Button)
        .label(entry.name.clone())
        .bg(move || {
            if selected.get().as_deref() == Some(row_path.as_str()) {
                theme().accent
            } else {
                theme().background
            }
        })
        .hover(|s| s.bg(move || theme().muted))
        .on_press(move || {
            if is_dir {
                if let Some(session) = session.get_untracked() {
                    load_remote_directory(
                        session,
                        press_path.clone(),
                        entries,
                        current_path,
                        selected,
                        status,
                        busy,
                    );
                }
            } else {
                selected.set(Some(press_path.clone()));
            }
        })
        .child(
            text(if entry.is_dir { "DIR" } else { "FILE" })
                .text_size(px(9.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(entry.name)
                .flex_1()
                .text_sm()
                .text_color(move || theme().foreground),
        )
        .child(
            text(if entry.is_dir {
                String::new()
            } else {
                format_size(entry.size)
            })
            .text_xs()
            .text_color(move || theme().muted_foreground),
        )
}

fn load_remote_directory(
    session: RemoteSession,
    path: String,
    entries: Signal<Vec<RemoteEntry>>,
    current_path: Signal<String>,
    selected: Signal<Option<String>>,
    status: Signal<String>,
    busy: Signal<bool>,
) {
    selected.set(None);
    busy.set(true);
    status.set(format!("Loading {path}…"));
    spawn(async move {
        let client = session.client();
        let path_for_query = path.clone();
        let listed = background(async move {
            let mut client = client.lock().await;
            client.list_dir(&path_for_query).await
        })
        .await;
        busy.set(false);
        match listed {
            Ok(Ok(list)) => {
                entries.set(list.into_iter().map(Into::into).collect());
                current_path.set(path);
            }
            Ok(Err(error)) => status.set(format!("Failed to list directory: {error:#}")),
            Err(error) => status.set(format!("Directory task failed: {error}")),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn transfer(
    session: RemoteSession,
    status: Signal<String>,
    busy: Signal<bool>,
    remote_entries: Signal<Vec<RemoteEntry>>,
    remote_path: Signal<String>,
    selected_remote: Signal<Option<String>>,
    progress: Arc<Mutex<TransferState>>,
    cancel: Signal<Option<TransferCancel>>,
    upload: bool,
    source: String,
    destination: String,
) {
    busy.set(true);
    let cancelled = Arc::new(AtomicBool::new(false));
    cancel.set(Some(TransferCancel(cancelled.clone())));
    if let Ok(mut progress) = progress.lock() {
        *progress = TransferState::default();
    }
    status.set(if upload {
        format!("Uploading {source}…")
    } else {
        format!("Downloading {source}…")
    });
    spawn(async move {
        let client = session.client();
        let source_for_transfer = source.clone();
        let destination_for_transfer = destination.clone();
        let cancelled_for_transfer = cancelled.clone();
        let progress_sink = progress.clone();
        let result = background(async move {
            let mut client = client.lock().await;
            let progress: ProgressCallback = Box::new(transfer_progress_callback(progress_sink));
            if upload {
                client
                    .upload_with_progress(
                        &source_for_transfer,
                        &destination_for_transfer,
                        cancelled_for_transfer,
                        progress,
                    )
                    .await
            } else {
                client
                    .download_with_progress(
                        &source_for_transfer,
                        &destination_for_transfer,
                        cancelled_for_transfer,
                        progress,
                    )
                    .await
            }
        })
        .await;
        busy.set(false);
        cancel.set(None);
        match result {
            Ok(Ok(())) => {
                status.set("Transfer complete".to_owned());
                load_remote_directory(
                    session,
                    remote_path.get_untracked(),
                    remote_entries,
                    remote_path,
                    selected_remote,
                    status,
                    busy,
                );
            }
            Ok(Err(error)) => status.set(format!("Transfer failed: {error:#}")),
            Err(error) => status.set(format!("Transfer task failed: {error}")),
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn transfer_directory(
    session: RemoteSession,
    status: Signal<String>,
    busy: Signal<bool>,
    remote_entries: Signal<Vec<RemoteEntry>>,
    remote_path: Signal<String>,
    selected_remote: Signal<Option<String>>,
    progress: Arc<Mutex<TransferState>>,
    cancel: Signal<Option<TransferCancel>>,
    upload: bool,
    source: String,
    destination: String,
    policy: DirectoryConflictPolicy,
) {
    busy.set(true);
    let cancelled = Arc::new(AtomicBool::new(false));
    cancel.set(Some(TransferCancel(cancelled.clone())));
    if let Ok(mut progress) = progress.lock() {
        *progress = TransferState::default();
    }
    status.set(if upload {
        format!("Uploading directory {source}…")
    } else {
        format!("Downloading directory {source}…")
    });
    spawn(async move {
        let client = session.client();
        let source_for_transfer = source.clone();
        let destination_for_transfer = destination.clone();
        let cancelled_for_transfer = cancelled.clone();
        let progress_sink = progress.clone();
        let result = background(async move {
            let mut client = client.lock().await;
            let progress: ProgressCallback = Box::new(transfer_progress_callback(progress_sink));
            if upload {
                client
                    .upload_dir_with_progress(
                        &source_for_transfer,
                        &destination_for_transfer,
                        policy,
                        cancelled_for_transfer,
                        progress,
                    )
                    .await
            } else {
                client
                    .download_dir_with_progress(
                        &source_for_transfer,
                        &destination_for_transfer,
                        cancelled_for_transfer,
                        progress,
                    )
                    .await
            }
        })
        .await;
        busy.set(false);
        cancel.set(None);
        match result {
            Ok(Ok(())) => {
                status.set("Directory transfer complete".to_owned());
                load_remote_directory(
                    session,
                    remote_path.get_untracked(),
                    remote_entries,
                    remote_path,
                    selected_remote,
                    status,
                    busy,
                );
            }
            Ok(Err(error)) => status.set(format!("Directory transfer failed: {error:#}")),
            Err(error) => status.set(format!("Directory transfer task failed: {error}")),
        }
    });
}

fn upload_directory_destination(local_directory: &str, remote_directory: &str) -> String {
    let name = Path::new(local_directory)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("local-directory");
    remote_join(remote_directory, name)
}

fn download_directory_destination(remote_directory: &str, local_directory: &str) -> String {
    let name = Path::new(remote_directory.trim_end_matches('/'))
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("remote-directory");
    Path::new(local_directory).join(name).display().to_string()
}

fn transfer_progress_callback(
    sink: Arc<Mutex<TransferState>>,
) -> impl Fn(TransferProgress) + Send + Sync + 'static {
    move |progress| {
        if let Ok(mut state) = sink.lock() {
            *state = TransferState {
                transferred: progress.transferred,
                total: progress.total,
                speed: progress.speed,
                current_file: progress.current_file,
            };
        }
    }
}

fn install_transfer_progress_pump(
    progress: Arc<Mutex<TransferState>>,
    visible: Signal<TransferState>,
) {
    set_interval(Duration::from_millis(120), move || {
        let Some(snapshot) = progress.lock().ok().map(|progress| progress.clone()) else {
            return;
        };
        if visible.get_untracked() != snapshot {
            visible.set(snapshot);
        }
    });
}

fn transfer_progress_text(state: &TransferState) -> String {
    if state.total == 0 && state.transferred == 0 {
        return "Ready · cancellation available during transfers".to_owned();
    }
    let percent = if state.total == 0 {
        0.0
    } else {
        state.transferred as f64 / state.total as f64 * 100.0
    };
    let file = state
        .current_file
        .as_deref()
        .and_then(|path| Path::new(path).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or("transfer");
    format!(
        "{} · {:.0}% · {}/s",
        file,
        percent.clamp(0.0, 100.0),
        format_size(state.speed.max(0.0).round() as u64)
    )
}

#[allow(clippy::too_many_arguments)]
fn favorite_button(
    path: String,
    remote_path: Signal<String>,
    session: Signal<Option<RemoteSession>>,
    remote_entries: Signal<Vec<RemoteEntry>>,
    selected_remote: Signal<Option<String>>,
    status: Signal<String>,
    busy: Signal<bool>,
) -> impl IntoElement {
    let press_path = path.clone();
    let label = path.clone();
    view()
        .px_2()
        .py_1()
        .rounded(move || theme().radius)
        .bg(move || theme().muted)
        .role(AccessibilityRole::Button)
        .label(path.clone())
        .on_press(move || {
            let Some(session) = session.get_untracked() else {
                status.set("Connect to a remote host first".to_owned());
                return;
            };
            load_remote_directory(
                session,
                press_path.clone(),
                remote_entries,
                remote_path,
                selected_remote,
                status,
                busy,
            );
        })
        .child(
            text(label)
                .text_size(px(10.0))
                .text_color(move || theme().foreground),
        )
}

fn policy_button(
    label_text: &'static str,
    policy: DirectoryConflictPolicy,
    selected: Signal<DirectoryConflictPolicy>,
) -> impl IntoElement {
    view()
        .px_2()
        .py_1()
        .rounded(move || theme().radius)
        .border_1()
        .border_color(move || {
            if selected.get() == policy {
                theme().primary
            } else {
                theme().border
            }
        })
        .bg(move || {
            if selected.get() == policy {
                theme().accent
            } else {
                theme().background
            }
        })
        .role(AccessibilityRole::Button)
        .accessibility_state(move || AccessibilityState {
            selected: selected.get() == policy,
            ..AccessibilityState::default()
        })
        .on_press(move || selected.set(policy))
        .child(
            text(label_text)
                .text_size(px(9.0))
                .text_color(move || theme().foreground),
        )
}

fn remote_connections(services: Option<&NavopNshServices>) -> Vec<StoredConnection> {
    services
        .map(|services| services.resolved_connections.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|connection| {
            matches!(
                connection.connection_type,
                ConnectionType::SshSftp | ConnectionType::Ftp
            )
        })
        .collect()
}

fn load_favorite_paths(
    repository: Option<&SftpFavoritePathRepository>,
    connection: &StoredConnection,
) -> Vec<String> {
    let Some(repository) = repository else {
        return Vec::new();
    };
    repository
        .list_paths(&sftp_favorite_connection_key(connection))
        .unwrap_or_default()
}

trait ConnectionId {
    fn id_string(&self) -> String;
}

impl ConnectionId for StoredConnection {
    fn id_string(&self) -> String {
        self.id.unwrap_or_default().to_string()
    }
}

fn home_directory() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_owned())
}

fn load_local_entries(path: &str) -> Vec<LocalEntry> {
    let Ok(entries) = std::fs::read_dir(path) else {
        return Vec::new();
    };
    let mut entries = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let metadata = entry.metadata().ok()?;
            Some(LocalEntry {
                name: entry.file_name().to_string_lossy().into_owned(),
                path: entry.path().display().to_string(),
                is_dir: metadata.is_dir(),
                size: metadata.len(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|entry| (!entry.is_dir, entry.name.to_lowercase()));
    entries
}

fn remote_join(base: &str, name: &str) -> String {
    if base.ends_with('/') {
        format!("{base}{name}")
    } else {
        format!("{base}/{name}")
    }
}

fn remote_parent(path: &str) -> String {
    let trimmed = path.trim_end_matches('/');
    let parent = trimmed
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    if parent.is_empty() {
        "/".to_owned()
    } else {
        parent.to_owned()
    }
}

fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
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
    fn remote_paths_join_and_walk_up() {
        assert_eq!("/home/user", remote_parent("/home/user/docs"));
        assert_eq!("/", remote_parent("/docs"));
        assert_eq!("/home/user/file.txt", remote_join("/home/user", "file.txt"));
        assert_eq!(
            "/home/user/file.txt",
            remote_join("/home/user/", "file.txt")
        );
    }

    #[test]
    fn directory_transfer_destinations_use_the_current_directory_name() {
        assert_eq!(
            "/srv/upload/project",
            upload_directory_destination("/home/user/project", "/srv/upload")
        );
        assert_eq!(
            "/home/user/project",
            download_directory_destination("/srv/upload/project", "/home/user")
        );
        assert_eq!(
            "/srv/upload/remote-directory",
            download_directory_destination("/", "/srv/upload")
        );
    }

    #[test]
    fn transfer_progress_reports_file_percentage_and_speed() {
        let text = transfer_progress_text(&TransferState {
            transferred: 50,
            total: 100,
            speed: 2048.0,
            current_file: Some("/tmp/project/archive.tar".to_owned()),
        });
        assert!(text.contains("archive.tar"));
        assert!(text.contains("50%"));
        assert!(text.contains("2.0 KiB/s"));
    }
}
