use std::{rc::Rc, sync::Arc};

use anyhow::{Result, anyhow};
use gpui::{App, Window};
use gpui_shell::{Capabilities, LoadedScriptView, ViewLoadOptions, policy::Policy};

use super::{
    PreparedShellView, ShellPluginHost, blob::blob_module, context::context_module,
    event::event_module, job::job_module, log::log_module, resource::resource_module,
    runtime::runtime_module, session::ShellMountSession,
};

pub(crate) struct LoadedShellView {
    loaded: LoadedScriptView,
    session: Arc<ShellMountSession>,
}

pub(crate) struct ShellLoadError {
    pub(crate) error: anyhow::Error,
    pub(crate) session: Arc<ShellMountSession>,
}

impl LoadedShellView {
    pub(crate) fn view(&self) -> &gpui::Entity<gpui_shell::ScriptView> {
        self.loaded.view()
    }
    pub(crate) fn unload(&mut self, cx: &mut App) {
        self.loaded.unload(cx);
    }
    pub(crate) fn session(&self) -> Arc<ShellMountSession> {
        Arc::clone(&self.session)
    }
}

impl ShellPluginHost {
    pub(crate) fn load(
        &self,
        prepared: PreparedShellView,
        window: &mut Window,
        cx: &mut App,
    ) -> std::result::Result<LoadedShellView, ShellLoadError> {
        let session = Arc::new(ShellMountSession::new(
            self.service.clone(),
            prepared.contribution.backends.clone(),
            self.tokio.clone(),
        ));
        let result = load_with_session(self, prepared, Arc::clone(&session), window, cx);
        result
            .map(|loaded| LoadedShellView {
                loaded,
                session: Arc::clone(&session),
            })
            .map_err(|error| ShellLoadError { error, session })
    }
}

fn load_with_session(
    host: &ShellPluginHost,
    prepared: PreparedShellView,
    session: Arc<ShellMountSession>,
    window: &mut Window,
    cx: &mut App,
) -> Result<LoadedScriptView> {
    let connection = prepared
        .connection
        .map(|connection| connection.adopt(&session))
        .transpose()?;
    let mut policy = base_policy(&prepared.contribution)?;
    let modules = &prepared.contribution.modules;
    if modules.contains(&extension_runtime::extension::manifest::ShellHostModule::Context) {
        policy = policy.with_host_module(context_module(&prepared.contribution, connection))?;
    }
    if modules.contains(&extension_runtime::extension::manifest::ShellHostModule::Resource) {
        policy = policy.with_host_module(resource_module(Arc::clone(&session)))?;
    }
    if modules.contains(&extension_runtime::extension::manifest::ShellHostModule::Blob) {
        policy = policy.with_host_module(blob_module(Arc::clone(&session)))?;
    }
    if modules.contains(&extension_runtime::extension::manifest::ShellHostModule::Job) {
        policy = policy.with_host_module(job_module(Arc::clone(&session)))?;
    }
    if modules.contains(&extension_runtime::extension::manifest::ShellHostModule::Event) {
        policy = policy.with_host_module(event_module(Arc::clone(&session)))?;
    }
    if modules.contains(&extension_runtime::extension::manifest::ShellHostModule::Runtime) {
        policy = policy.with_host_module(runtime_module(Arc::clone(&session)))?;
    }
    if modules.contains(&extension_runtime::extension::manifest::ShellHostModule::Log) {
        policy = policy.with_host_module(log_module(&prepared.contribution))?;
    }
    let options = ViewLoadOptions::new(
        &prepared.contribution.extension_root,
        entry_relative_path(&prepared.contribution)?,
        Rc::new(policy),
    );
    host.runtime.load_view(options, window, cx)
}

fn base_policy(
    contribution: &extension_runtime::RegisteredShellViewContribution,
) -> Result<Policy> {
    let mut policy = Policy::new()
        .with_application(&contribution.view_key)
        .with_capabilities(Capabilities::new().storage(contribution.singleton));
    if contribution.singleton {
        policy = policy.with_storage_path(shell_storage_path(contribution)?);
    }
    Ok(policy)
}

pub(super) fn validate_entry_path(
    contribution: &extension_runtime::RegisteredShellViewContribution,
) -> Result<()> {
    let root = contribution.extension_root.canonicalize()?;
    let entry = contribution.entry_path.canonicalize()?;
    anyhow::ensure!(
        entry.starts_with(root),
        "shell entry escaped extension root"
    );
    Ok(())
}

fn entry_relative_path(
    contribution: &extension_runtime::RegisteredShellViewContribution,
) -> Result<&str> {
    contribution
        .entry_path
        .strip_prefix(&contribution.extension_root)?
        .to_str()
        .ok_or_else(|| anyhow!("shell entry path is not UTF-8"))
}

fn shell_storage_path(
    contribution: &extension_runtime::RegisteredShellViewContribution,
) -> Result<std::path::PathBuf> {
    let root = one_core::app_dirs::data_dir()
        .map(Ok)
        .unwrap_or_else(one_core::app_dirs::config_dir)?;
    let directory = root
        .join("extensions")
        .join("shell")
        .join(&contribution.extension_id)
        .join(&contribution.id);
    std::fs::create_dir_all(&directory)?;
    Ok(directory.join("store.json"))
}
