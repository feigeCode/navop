//! Navop's nsh shell application bootstrap.
//!
//! The normal Navop product bootstrap owns paths, single-instance forwarding,
//! tray, updates, window restoration and file/URL startup handling. This crate
//! only supplies the main window root: an nsh `AppView` running the migrated UI.

use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use domain_api::{AppIdentity, Domain, Product, ProductFeatures};
use gpui::{
    AnyView, App, AppContext as _, Context, IntoElement, ParentElement as _, Render, Styled as _,
    Window, div,
};
use navop_nsh::{NavopExtensionInfo, NavopNshServices};
use nsh_core::element::EngineContext;
use nsh_platform_gpui::{App as NshApp, AppOptions, AppView, Components, Root, set_ui_sets};
use one_core::storage::traits::Repository as _;
use one_core::storage::{ConnectionRepository, ConnectionType, GlobalStorageState};

const NSH_APP_ID: &str = "com.onetcli.navop.nsh";

struct NshProduct;

impl Product for NshProduct {
    type Assets = navop_app::AppAssets;

    fn identity(&self) -> &'static AppIdentity {
        &AppIdentity::FULL
    }

    fn features(&self) -> ProductFeatures {
        ProductFeatures::FULL
    }

    fn assets(&self) -> Self::Assets {
        navop_app::AppAssets::new()
    }

    fn domains(&self) -> Vec<Arc<dyn Domain>> {
        vec![
            Arc::new(shell_domain::ShellDomain),
            Arc::new(data_domain::DataDomain),
        ]
    }

    fn mcp_managed_by_app_kit(&self) -> bool {
        false
    }

    fn init(&self, cx: &mut gpui::App) -> anyhow::Result<()> {
        navop_app::init_app(cx)
    }
}

static APP_VERSION: OnceLock<&'static str> = OnceLock::new();

/// Runs Navop with the full product shell and the nsh main window.
pub fn run(version: &'static str) {
    let _ = APP_VERSION.set(version);
    navop_app::run_with_nsh(NshProduct, version, nsh_main_window_root);
}

struct RootError {
    message: String,
}

impl Render for RootError {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_6().child(self.message.clone())
    }
}

fn root_error(cx: &mut App, message: impl Into<String>) -> AnyView {
    AnyView::from(cx.new(|_| RootError {
        message: message.into(),
    }))
}

fn nsh_main_window_root(window: &mut Window, cx: &mut App) -> AnyView {
    let Some(storage) = cx.try_global::<GlobalStorageState>() else {
        return root_error(cx, "Navop storage was not initialized");
    };
    let Some(connections) = storage.storage.get::<ConnectionRepository>() else {
        return root_error(cx, "Connection repository was not registered");
    };
    let notes = notes::NotesStorage::configured_storage().ok().map(Arc::new);
    let credentials = storage
        .storage
        .get::<one_core::storage::CredentialRepository>()
        .map(|repository| (*repository).clone());
    let workspaces = storage
        .storage
        .get::<one_core::storage::WorkspaceRepository>()
        .map(|repository| (*repository).clone());
    let sftp_favorites = storage
        .storage
        .get::<one_core::storage::SftpFavoritePathRepository>()
        .map(|repository| (*repository).clone());
    let resolved_connections = connections
        .list()
        .unwrap_or_default()
        .into_iter()
        .map(|connection| {
            connections
                .resolve_runtime_connection(&connection)
                .unwrap_or(connection)
        })
        .collect();
    let local_terminal =
        terminal::local_config_from_settings(&one_core::settings::AppSettings::current(cx), None)
            .ok();
    let mut db_state = cx
        .try_global::<db::GlobalDbState>()
        .cloned()
        .unwrap_or_default();
    match connections.list() {
        Ok(saved) => {
            for connection in saved
                .into_iter()
                .filter(|connection| connection.connection_type == ConnectionType::Database)
            {
                if let Ok(config) = connection.to_db_connection() {
                    db_state.register_connection(config);
                }
            }
        }
        Err(error) => {
            eprintln!("Failed to load database connection configs: {error:#}");
        }
    }
    cx.set_global(db_state.clone());
    let mongo_state = cx.try_global::<mongodb_view::GlobalMongoState>().cloned();
    let acp_agents = ai_chat_view::build_acp_agent_configs(cx).unwrap_or_default();
    let tokio = one_core::gpui_tokio::Tokio::handle(cx);
    let workspace_root = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let initialized_paths = one_core::app_paths::initialized_paths();
    let data_dir = initialized_paths
        .as_ref()
        .and_then(|paths| paths.data_dir().map(|path| path.display().to_string()))
        .map(Arc::<str>::from);
    let session_logs_dir = initialized_paths
        .as_ref()
        .and_then(|paths| paths.data_dir())
        .map(terminal::recording::session_logs_directory);
    let extensions = extension_runtime::global_catalog(cx)
        .map(|catalog| {
            catalog
                .shell_apps()
                .map(|app| NavopExtensionInfo {
                    id: app.extension_id.clone(),
                    title: app.title.clone(),
                    version: app.extension_version.clone(),
                    description: app.description.clone(),
                    category: app.category.clone(),
                    keywords: app.keywords.clone(),
                    permissions: app.permissions.clone(),
                    modules: app
                        .modules
                        .iter()
                        .map(|module| format!("{module:?}"))
                        .collect(),
                    entry_path: app.entry_path.clone(),
                })
                .collect()
        })
        .unwrap_or_default();
    let teams = one_core::cloud_sync::get_cached_team_options(cx);
    let settings = one_core::settings::AppSettings::current(cx);
    let services = NavopNshServices {
        connections: (*connections).clone(),
        workspaces,
        sftp_favorites,
        resolved_connections,
        db: Some(db_state),
        mongo: mongo_state,
        acp_agents,
        tokio,
        workspace_root,
        notes,
        credentials,
        session_logs_dir,
        extensions,
        teams,
        settings,
        local_terminal,
        known_hosts: ssh::HostKeyVerifier::default(),
        version: Arc::from(
            APP_VERSION
                .get()
                .copied()
                .unwrap_or(env!("CARGO_PKG_VERSION")),
        ),
        data_dir,
    };
    let context = EngineContext::default().with(services);

    set_ui_sets(vec![nsh_base::registry(), navop_nsh::registry()]);
    let components = Components::default();
    components.register(
        "navop.extension",
        Rc::new(|context| Box::new(navop_nsh::ExtensionAppComponent::new(context))),
    );
    components.register(
        "navop.extension_manager",
        Rc::new(|context| Box::new(navop_nsh::ExtensionManagerHost::new(context))),
    );
    components.register(
        "navop.remote_desktop",
        Rc::new(|context| Box::new(navop_nsh::RemoteDesktopHost::new(context))),
    );
    components.register(
        "navop.port_forwarding",
        Rc::new(|context| Box::new(navop_nsh::PortForwardingHost::new(context))),
    );
    components.register(
        "navop.terminal",
        Rc::new(|context| Box::new(navop_nsh::TerminalHost::new(context))),
    );

    cx.on_window_closed(|cx, _| {
        if cx.windows().is_empty() {
            cx.quit();
        }
    })
    .detach();

    let options = AppOptions {
        app_id: NSH_APP_ID.to_owned(),
        title: "Navop".to_owned(),
        size: (1400, 900),
        context,
        components,
        ..AppOptions::default()
    };
    let view = cx.new(|cx| AppView::new(options, window, cx));
    view.read(cx)
        .load(NshApp::Rust(Root::of::<navop_nsh::NavopNshApp>()));
    AnyView::from(view)
}

#[cfg(test)]
mod tests {
    #[test]
    fn nsh_root_starts_the_engine_after_the_view_is_created() {
        let source = include_str!("lib.rs");
        let create = source.find("AppView::new(options, window, cx)").unwrap();
        let load = source.find(".load(NshApp::Rust(Root::of::").unwrap();
        assert!(
            create < load,
            "AppView::new only creates an empty surface; load() starts it"
        );
    }
}
