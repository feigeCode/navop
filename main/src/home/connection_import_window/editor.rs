use db_view::connection_form_window::{
    ConnectionFormPostSaveAction, ConnectionFormWindow, ConnectionFormWindowConfig,
};
use gpui::{App, AppContext, Context, Window};
use mongodb_view::{MongoFormSavedCallback, MongoFormWindow, MongoFormWindowConfig};
use one_core::cloud_sync::get_cached_team_options;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use one_core::storage::{ConnectionType, StoredConnection};
use redis_view::{RedisFormSavedCallback, RedisFormWindow, RedisFormWindowConfig};
use rust_i18n::t;
use std::sync::Arc;
use terminal_view::{SshFormPostSaveAction, SshFormWindow, SshFormWindowConfig};

use super::ConnectionImportWindow;
use crate::home_tab::HomePage;

impl ConnectionImportWindow {
    pub(crate) fn edit_row(&mut self, record_id: String, cx: &mut Context<Self>) {
        let Some(draft) = self.model.draft(&record_id) else {
            return;
        };
        let connection = match draft.to_editor_connection() {
            Ok(connection) => connection,
            Err(error) => {
                self.model.mark_failed(&record_id, error);
                cx.notify();
                return;
            }
        };
        self.open_editor_for_connection(record_id, connection, cx);
    }

    fn open_editor_for_connection(
        &self,
        record_id: String,
        connection: StoredConnection,
        cx: &mut Context<Self>,
    ) {
        match connection.connection_type {
            ConnectionType::Database => self.open_database_editor(record_id, connection, cx),
            ConnectionType::Redis => self.open_redis_editor(record_id, connection, cx),
            ConnectionType::MongoDB => self.open_mongodb_editor(record_id, connection, cx),
            ConnectionType::SshSftp => self.open_ssh_editor(record_id, connection, cx),
            _ => {}
        }
    }

    fn open_database_editor(
        &self,
        record_id: String,
        connection: StoredConnection,
        cx: &mut Context<Self>,
    ) {
        let Ok(config) = connection.to_db_connection() else {
            return;
        };
        let (workspaces, ssh_connections, external_driver_registry) =
            self.parent.read(cx).import_editor_context();
        let form_config = ConnectionFormWindowConfig {
            db_type: config.database_type.clone(),
            external_driver_id: None,
            external_driver_registry,
            editing_connection: None,
            initial_connection: Some(connection),
            on_saved: Some(self.database_editor_saved_callback(record_id, cx)),
            workspaces,
            teams: get_cached_team_options(cx),
            ssh_connections,
        };
        open_popup_window(
            PopupWindowOptions::new(t!("Home.import").to_string()).size(700.0, 650.0),
            move |window, cx| cx.new(|cx| ConnectionFormWindow::new(form_config, window, cx)),
            cx,
        );
    }

    fn open_redis_editor(
        &self,
        record_id: String,
        connection: StoredConnection,
        cx: &mut Context<Self>,
    ) {
        let (workspaces, ssh_connections, _) = self.parent.read(cx).import_editor_context();
        let form_config = RedisFormWindowConfig {
            editing_connection: None,
            initial_connection: Some(connection),
            on_saved: Some(self.redis_editor_saved_callback(record_id, cx)),
            workspaces,
            teams: get_cached_team_options(cx),
            ssh_connections,
        };
        open_popup_window(
            PopupWindowOptions::new(t!("Home.import").to_string()).size(700.0, 650.0),
            move |window, cx| cx.new(|cx| RedisFormWindow::new(form_config, window, cx)),
            cx,
        );
    }

    fn open_mongodb_editor(
        &self,
        record_id: String,
        connection: StoredConnection,
        cx: &mut Context<Self>,
    ) {
        let (workspaces, ssh_connections, _) = self.parent.read(cx).import_editor_context();
        let form_config = MongoFormWindowConfig {
            editing_connection: None,
            initial_connection: Some(connection),
            on_saved: Some(self.mongodb_editor_saved_callback(record_id, cx)),
            workspaces,
            teams: get_cached_team_options(cx),
            ssh_connections,
        };
        open_popup_window(
            PopupWindowOptions::new(t!("Home.import").to_string()).size(700.0, 520.0),
            move |window, cx| cx.new(|cx| MongoFormWindow::new(form_config, window, cx)),
            cx,
        );
    }

    fn open_ssh_editor(
        &self,
        record_id: String,
        connection: StoredConnection,
        cx: &mut Context<Self>,
    ) {
        let (workspaces, _, _) = self.parent.read(cx).import_editor_context();
        let form_config = SshFormWindowConfig {
            editing_connection: None,
            initial_connection: Some(connection),
            on_saved: Some(self.ssh_editor_saved_callback(record_id, cx)),
            workspaces,
            teams: get_cached_team_options(cx),
        };
        open_popup_window(
            PopupWindowOptions::new(t!("Home.import").to_string()).size(700.0, 650.0),
            move |window, cx| cx.new(|cx| SshFormWindow::new(form_config, window, cx)),
            cx,
        );
    }

    fn database_editor_saved_callback(
        &self,
        record_id: String,
        cx: &mut Context<Self>,
    ) -> Arc<
        dyn Fn(StoredConnection, ConnectionFormPostSaveAction, &mut Window, &mut App)
            + Send
            + Sync
            + 'static,
    > {
        let import_window = cx.entity();
        Arc::new(move |saved_connection, action, _, cx| {
            let record_id = record_id.clone();
            let import_window = import_window.clone();
            let _ = import_window.update(cx, |this, cx| {
                this.handle_editor_saved(
                    record_id,
                    saved_connection.id,
                    matches!(action, ConnectionFormPostSaveAction::Continue),
                    cx,
                );
            });
        })
    }

    fn redis_editor_saved_callback(
        &self,
        record_id: String,
        cx: &mut Context<Self>,
    ) -> RedisFormSavedCallback {
        let import_window = cx.entity();
        Arc::new(move |saved_connection, cx| {
            let record_id = record_id.clone();
            let import_window = import_window.clone();
            let _ = import_window.update(cx, |this, cx| {
                this.handle_editor_saved(record_id, saved_connection.id, false, cx);
            });
        })
    }

    fn mongodb_editor_saved_callback(
        &self,
        record_id: String,
        cx: &mut Context<Self>,
    ) -> MongoFormSavedCallback {
        let import_window = cx.entity();
        Arc::new(move |saved_connection, cx| {
            let record_id = record_id.clone();
            let import_window = import_window.clone();
            let _ = import_window.update(cx, |this, cx| {
                this.handle_editor_saved(record_id, saved_connection.id, false, cx);
            });
        })
    }

    fn ssh_editor_saved_callback(
        &self,
        record_id: String,
        cx: &mut Context<Self>,
    ) -> Arc<
        dyn Fn(StoredConnection, SshFormPostSaveAction, &mut Window, &mut App)
            + Send
            + Sync
            + 'static,
    > {
        let import_window = cx.entity();
        Arc::new(move |saved_connection, action, _, cx| {
            let record_id = record_id.clone();
            let import_window = import_window.clone();
            let _ = import_window.update(cx, |this, cx| {
                this.handle_editor_saved(
                    record_id,
                    saved_connection.id,
                    matches!(action, SshFormPostSaveAction::Continue),
                    cx,
                );
            });
        })
    }

    fn handle_editor_saved(
        &mut self,
        record_id: String,
        connection_id: Option<i64>,
        continue_to_next: bool,
        cx: &mut Context<Self>,
    ) {
        self.model.mark_saved(&record_id, connection_id);
        if continue_to_next
            && let Some(next_record_id) = self.model.next_save_candidate_row_id_after(&record_id)
        {
            self.edit_row(next_record_id, cx);
        }
        cx.notify();
    }
}

trait ImportEditorContext {
    fn import_editor_context(
        &self,
    ) -> (
        Vec<one_core::storage::Workspace>,
        Vec<StoredConnection>,
        db::ipc::IpcDriverRegistry,
    );
}

impl ImportEditorContext for HomePage {
    fn import_editor_context(
        &self,
    ) -> (
        Vec<one_core::storage::Workspace>,
        Vec<StoredConnection>,
        db::ipc::IpcDriverRegistry,
    ) {
        let ssh_connections = self
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .cloned()
            .collect();
        (
            self.workspaces.clone(),
            ssh_connections,
            self.external_driver_registry.clone(),
        )
    }
}
