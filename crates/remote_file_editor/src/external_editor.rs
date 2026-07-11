use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, anyhow};
use extension_runtime::extension::manifest::RemoteFileEditorLaunchMode;
use extension_runtime::{GlobalExtensionRuntimeCatalog, RegisteredRemoteFileEditorContribution};
use futures::channel::mpsc;
use gpui::{App, AppContext as _, Context, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use one_core::{gpui_tokio::Tokio, settings::AppSettings};
use rust_i18n::t;
use sftp::{RusshSftpClient, SftpClient};
use tokio::sync::Mutex;

use crate::external_edit_controller::{
    ExternalEditController, ExternalEditControllerConfig, ExternalEditWatchLoop, local_content_hash,
};
use crate::external_editor_confirmation::confirm_external_program;
use crate::external_session::snapshot_from_metadata;
use crate::{
    LaunchTemplateContext, MAX_EDITABLE_FILE_SIZE, RemoteFileSnapshot, launch_external_editor,
    matching_editors, render_args, resolve_editor_program, session_temp_file,
};

static NEXT_SESSION_ID: AtomicU64 = AtomicU64::new(1);

pub struct ExternalEditorOpenRequest {
    pub remote_path: String,
    pub editor_key: String,
    pub client: Arc<Mutex<RusshSftpClient>>,
}

pub(crate) struct ExternalEditLaunch {
    pub(crate) remote_path: String,
    pub(crate) editor_key: String,
    pub(crate) program: String,
    pub(crate) launch_mode: RemoteFileEditorLaunchMode,
    pub(crate) templates: Vec<String>,
    pub(crate) client: Arc<Mutex<RusshSftpClient>>,
    pub(crate) check_conflict: bool,
    pub(crate) auto_upload: bool,
}

pub fn external_editor_menu_label(editor: &str) -> String {
    t!("RemoteFileEditor.action.edit_with", editor = editor).to_string()
}

pub fn external_editors_for_file(
    file_name: &str,
    cx: &App,
) -> Vec<RegisteredRemoteFileEditorContribution> {
    let Some(catalog) = cx
        .try_global::<GlobalExtensionRuntimeCatalog>()
        .and_then(GlobalExtensionRuntimeCatalog::get)
    else {
        return Vec::new();
    };
    let settings = AppSettings::current(cx);
    matching_editors(
        catalog.remote_file_editors(),
        file_name,
        settings
            .remote_file_editor
            .default_external_editor
            .as_deref(),
    )
}

struct PreparedExternalEdit {
    local_path: PathBuf,
    snapshot: RemoteFileSnapshot,
    local_hash: u64,
}

pub fn open_remote_file_external_editor<T: 'static>(
    request: ExternalEditorOpenRequest,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let Some(editor) = find_editor(&request.editor_key, cx) else {
        window.push_notification(
            Notification::error(t!("RemoteFileEditor.notification.external_editor_missing")),
            cx,
        );
        return;
    };
    let settings = AppSettings::current(cx);
    let editor_override = settings
        .remote_file_editor
        .overrides
        .iter()
        .find(|value| value.editor_key == request.editor_key);
    let Some(program) = resolve_editor_program(&editor.command.program_candidates, editor_override)
    else {
        window.push_notification(
            Notification::error(t!("RemoteFileEditor.notification.configure_program")),
            cx,
        );
        return;
    };
    let templates = editor_override
        .filter(|value| !value.args.is_empty())
        .map(|value| value.args.as_slice())
        .unwrap_or(&editor.command.args);
    let check_conflict = settings
        .remote_file_editor
        .check_remote_modified_before_upload;
    let auto_upload = settings.remote_file_editor.auto_upload_external_changes;
    let launch = ExternalEditLaunch {
        remote_path: request.remote_path,
        editor_key: request.editor_key,
        program,
        launch_mode: editor.command.launch_mode,
        templates: templates.to_vec(),
        client: request.client,
        check_conflict,
        auto_upload,
    };
    if editor_override.is_none() {
        confirm_external_program(launch, window, cx);
        return;
    }
    launch.start(window, cx);
}

fn find_editor<T>(
    editor_key: &str,
    cx: &Context<T>,
) -> Option<RegisteredRemoteFileEditorContribution> {
    cx.try_global::<GlobalExtensionRuntimeCatalog>()?
        .get()?
        .remote_file_editors()
        .iter()
        .find(|editor| editor.editor_key == editor_key)
        .cloned()
}

impl ExternalEditLaunch {
    pub(crate) fn start<T: 'static>(self, window: &mut Window, cx: &mut Context<T>) {
        let task = prepare_external_edit(self.remote_path.clone(), self.client.clone(), cx);
        window
            .spawn(cx, async move |cx| match task.await {
                Ok(prepared) => {
                    let result = cx.update(|window, cx| self.launch(prepared, window, cx));
                    if let Err(error) = result {
                        tracing::error!(?error, "failed to start external remote file editor");
                    }
                }
                Err(error) => notify_error(cx, error.to_string()),
            })
            .detach();
    }

    fn launch(
        self,
        prepared: PreparedExternalEdit,
        window: &mut Window,
        cx: &mut gpui::App,
    ) -> Result<()> {
        let args = render_args(
            &self.templates,
            &LaunchTemplateContext {
                file: prepared.local_path.to_string_lossy().into_owned(),
                remote_path: self.remote_path.clone(),
                name: file_name(&self.remote_path),
            },
        );
        launch_external_editor(&self.program, &args, self.launch_mode)?;
        if !self.auto_upload {
            return Ok(());
        }
        let (sender, receiver) = mpsc::unbounded();
        let watcher = watch_local_file(prepared.local_path.clone(), sender)?;
        let config = ExternalEditControllerConfig {
            client: self.client,
            remote_path: self.remote_path,
            local_path: prepared.local_path,
            snapshot: prepared.snapshot,
            initial_local_hash: prepared.local_hash,
            check_conflict: self.check_conflict,
        };
        let controller = cx.new(|_| ExternalEditController::new(config, watcher));
        ExternalEditWatchLoop::new(controller, receiver).run(window, cx);
        Ok(())
    }
}

fn prepare_external_edit<T>(
    remote_path: String,
    client: Arc<Mutex<RusshSftpClient>>,
    cx: &Context<T>,
) -> gpui::Task<Result<PreparedExternalEdit>> {
    let local_path = session_temp_file(
        &std::env::temp_dir().join("onetcli/remote-edit"),
        &next_session_id(),
        &remote_path,
    );
    Tokio::spawn_result(cx, async move {
        let (metadata, bytes) = {
            let mut client = client.lock().await;
            let metadata = client
                .stat(&remote_path)
                .await?
                .ok_or_else(|| anyhow!("Remote file no longer exists"))?;
            let bytes = client
                .read_file(&remote_path, MAX_EDITABLE_FILE_SIZE)
                .await?;
            (metadata, bytes)
        };
        if let Some(parent) = local_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let local_hash = local_content_hash(&bytes);
        tokio::fs::write(&local_path, bytes).await?;
        Ok(PreparedExternalEdit {
            local_path,
            snapshot: snapshot_from_metadata(&metadata),
            local_hash,
        })
    })
}

fn watch_local_file(
    local_path: PathBuf,
    sender: mpsc::UnboundedSender<()>,
) -> Result<RecommendedWatcher> {
    let watched_path = local_path.clone();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event
            && matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_))
            && event.paths.iter().any(|path| path == &watched_path)
        {
            let _ = sender.unbounded_send(());
        }
    })?;
    let parent = local_path
        .parent()
        .context("external editor temp file has no parent directory")?;
    watcher.watch(parent, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

fn notify_error(cx: &mut gpui::AsyncWindowContext, message: String) {
    let _ = cx.update(|window, cx| {
        window.push_notification(Notification::error(message), cx);
    });
}

fn next_session_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let sequence = NEXT_SESSION_ID.fetch_add(1, Ordering::Relaxed);
    format!("{timestamp}-{sequence}")
}

fn file_name(remote_path: &str) -> String {
    remote_path
        .rsplit('/')
        .find(|value| !value.is_empty())
        .unwrap_or("remote-file")
        .to_string()
}
