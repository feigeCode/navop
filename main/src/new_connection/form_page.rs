use db_view::connection_form_window::{ConnectionFormWindow, ConnectionFormWindowConfig};
use gpui::{AnyView, AppContext, Context, Entity, Window};
use mongodb_view::{MongoFormWindow, MongoFormWindowConfig};
use one_core::cloud_sync::get_cached_team_options;
use one_core::storage::{ConnectionType, DatabaseType};
use redis_view::{RedisFormWindow, RedisFormWindowConfig};
use terminal_view::{SerialFormWindow, SerialFormWindowConfig, SshFormWindow, SshFormWindowConfig};

use crate::home_tab::HomePage;
use crate::new_connection::NewConnectionWindow;
use crate::new_connection::connection_kind::NewConnectionKind;

pub(crate) enum NewConnectionFormResult {
    Form(AnyView),
    Done,
    Blocked,
}

pub(crate) trait NewConnectionFormPage {
    fn build_form_view(
        self,
        parent: Entity<HomePage>,
        window: &mut Window,
        cx: &mut Context<NewConnectionWindow>,
    ) -> NewConnectionFormResult;
}

impl NewConnectionFormPage for NewConnectionKind {
    fn build_form_view(
        self,
        parent: Entity<HomePage>,
        window: &mut Window,
        cx: &mut Context<NewConnectionWindow>,
    ) -> NewConnectionFormResult {
        match self {
            Self::Terminal => open_terminal_tab(parent, window, cx),
            Self::Ssh => build_ssh_form(parent, window, cx),
            Self::Redis => build_redis_form(parent, window, cx),
            Self::MongoDB => build_mongo_form(parent, window, cx),
            Self::Serial => build_serial_form(parent, window, cx),
            Self::Database(db_type) => build_database_form(parent, db_type, window, cx),
        }
    }
}

fn open_terminal_tab(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let _ = parent.update(cx, |home, cx| {
        home.add_terminal_tab(window, cx);
    });
    window.remove_window();
    NewConnectionFormResult::Done
}

fn build_database_form(
    parent: Entity<HomePage>,
    db_type: DatabaseType,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.ensure_master_key_ready_for_new_connection(window, cx) {
            return None;
        }

        let editing_connection = home
            .editing_connection_id
            .and_then(|id| home.connections.iter().find(|c| c.id == Some(id)).cloned());
        home.editing_connection_id = None;
        Some(ConnectionFormWindowConfig {
            db_type,
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| ConnectionFormWindow::new(config, window, cx))
            .into(),
    )
}

fn build_ssh_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.ensure_master_key_ready_for_new_connection(window, cx) {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::SshSftp)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(SshFormWindowConfig {
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(cx.new(|cx| SshFormWindow::new(config, window, cx)).into())
}

fn build_redis_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.ensure_master_key_ready_for_new_connection(window, cx) {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Redis)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(RedisFormWindowConfig {
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(cx.new(|cx| RedisFormWindow::new(config, window, cx)).into())
}

fn build_mongo_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.ensure_master_key_ready_for_new_connection(window, cx) {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::MongoDB)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(MongoFormWindowConfig {
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(cx.new(|cx| MongoFormWindow::new(config, window, cx)).into())
}

fn build_serial_form(
    parent: Entity<HomePage>,
    window: &mut Window,
    cx: &mut Context<NewConnectionWindow>,
) -> NewConnectionFormResult {
    let Some(config) = parent.update(cx, |home, cx| {
        if !home.ensure_master_key_ready_for_new_connection(window, cx) {
            return None;
        }

        let editing_connection = home.editing_connection_id.and_then(|id| {
            home.connections
                .iter()
                .find(|c| c.id == Some(id) && c.connection_type == ConnectionType::Serial)
                .cloned()
        });
        home.editing_connection_id = None;
        Some(SerialFormWindowConfig {
            editing_connection,
            workspaces: home.workspaces.clone(),
            teams: get_cached_team_options(cx),
        })
    }) else {
        return NewConnectionFormResult::Blocked;
    };

    NewConnectionFormResult::Form(
        cx.new(|cx| SerialFormWindow::new(config, window, cx))
            .into(),
    )
}
