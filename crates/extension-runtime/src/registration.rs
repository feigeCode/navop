use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use db_view::extension_menu::DbTreeExtensionMenuItem;
use serde_json::Value;

use crate::extension::is_active_install_dir_name;
use crate::extension::manifest::{
    HostApiVersions, Manifest, ManifestError, current_host_version, load_and_check,
    required_spawn_permission, validate_shell_views,
};

use super::catalog::ExtensionRuntimeCatalog;
use super::types::{
    ExtensionRuntimeError, RegisteredDbTreeMenuContribution, RegisteredDocumentExporter,
    RegisteredDocumentRenderer, RegisteredHtmlPreviewTransform, RegisteredIpcRuntimeBinding,
    RegisteredKeybindingContribution, RegisteredRemoteFileEditorCommand,
    RegisteredRemoteFileEditorContribution, RegisteredResourceConnectionContribution,
    RegisteredShellViewContribution, WasmRuntimeBinding, command_descriptor, runtime_key,
    slot_item_from_menu,
};

static WASM_REGISTRATION_LOG_KEYS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn validate_resource_connection(
    manifest: &Manifest,
    connection: &crate::extension::manifest::ResourceConnectionContrib,
    ids: &mut HashSet<String>,
) -> Result<(), ExtensionRuntimeError> {
    let connection_id = connection.id.trim();
    if connection_id.is_empty() {
        return invalid_connection("connection id must not be empty");
    }
    if invalid_connection_identifier(connection_id) {
        return invalid_connection("connection id contains reserved path characters");
    }
    if connection.runtime_id.trim().is_empty() || connection.resource_type.trim().is_empty() {
        return invalid_connection("runtimeId and resourceType must not be empty");
    }
    if !ids.insert(connection_id.to_string()) {
        return invalid_connection(format!("duplicate connection id `{connection_id}`"));
    }
    if !manifest
        .runtime
        .ipc
        .iter()
        .any(|runtime| runtime.id == connection.runtime_id)
    {
        return invalid_connection(format!("unknown IPC runtime `{}`", connection.runtime_id));
    }
    if let Some(view_id) = connection.shell_view_id.as_deref() {
        let Some(view) = manifest
            .contributes
            .shell_views
            .iter()
            .find(|view| view.id == view_id)
        else {
            return invalid_connection(format!("unknown shell view `{view_id}`"));
        };
        if view.singleton {
            return invalid_connection("connection shell view must not be singleton");
        }
        if !view
            .modules
            .contains(&crate::extension::manifest::ShellHostModule::Context)
            || !view
                .modules
                .contains(&crate::extension::manifest::ShellHostModule::Resource)
        {
            return invalid_connection(
                "connection shell view requires context and resource modules",
            );
        }
        if !view
            .backends
            .values()
            .any(|runtime_id| runtime_id == &connection.runtime_id)
        {
            return invalid_connection(
                "connection shell view must expose the connection runtime as a backend",
            );
        }
    }
    validate_connection_icon(connection)?;
    validate_connection_form(connection)?;
    let has_secrets = connection
        .form
        .tabs
        .iter()
        .flat_map(|tab| &tab.fields)
        .any(|field| field.secret);
    if has_secrets
        && !manifest
            .permissions
            .iter()
            .any(|permission| permission == "secrets:read:self.*")
    {
        return invalid_connection("secret fields require secrets:read:self.* permission");
    }
    Ok(())
}

fn invalid_connection_identifier(value: &str) -> bool {
    value.contains([':', '/', '\\'])
}

fn validate_connection_icon(
    connection: &crate::extension::manifest::ResourceConnectionContrib,
) -> Result<(), ExtensionRuntimeError> {
    let Some(icon) = connection.icon.as_deref() else {
        return Ok(());
    };
    let path = std::path::Path::new(icon);
    if path.is_absolute()
        || icon.starts_with("\\\\")
        || icon.starts_with("//")
        || icon.as_bytes().get(1) == Some(&b':')
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
    {
        return invalid_connection("connection icon must stay within the extension root");
    }
    Ok(())
}

fn validate_connection_form(
    connection: &crate::extension::manifest::ResourceConnectionContrib,
) -> Result<(), ExtensionRuntimeError> {
    const RESERVED_FIELDS: &[&str] = &["name", "workspace", "remark", "sync_enabled", "team_id"];
    let mut fields = HashSet::new();
    let mut tabs = HashSet::new();
    for tab in &connection.form.tabs {
        if tab.id.trim().is_empty() || tab.label.trim().is_empty() || !tabs.insert(tab.id.clone()) {
            return invalid_connection(format!("duplicate or empty connection tab `{}`", tab.id));
        }
    }
    for field in connection.form.tabs.iter().flat_map(|tab| &tab.fields) {
        if field.id.trim().is_empty()
            || field.label.trim().is_empty()
            || invalid_connection_identifier(&field.id)
            || !fields.insert(field.id.clone())
        {
            return invalid_connection(format!("duplicate connection field `{}`", field.id));
        }
        if RESERVED_FIELDS.contains(&field.id.as_str()) {
            return invalid_connection(format!(
                "connection field `{}` is reserved by the host",
                field.id
            ));
        }
        if field.secret
            && field.field_type != crate::extension::manifest::ResourceConnectionFieldType::Password
        {
            return invalid_connection(format!("secret field `{}` must use Password", field.id));
        }
        validate_field_options(field)?;
        if field.field_type == crate::extension::manifest::ResourceConnectionFieldType::Password
            && !field.secret
        {
            return invalid_connection(format!(
                "password field `{}` must declare secret=true",
                field.id
            ));
        }
        if field.secret
            && field
                .default_value
                .as_deref()
                .is_some_and(|value| !value.is_empty())
        {
            return invalid_connection(format!(
                "secret field `{}` cannot declare a default value",
                field.id
            ));
        }
    }
    for field in connection.form.tabs.iter().flat_map(|tab| &tab.fields) {
        for rule in &field.visible_when {
            if !fields.contains(&rule.field) {
                return invalid_connection(format!(
                    "connection field `{}` references unknown visibility field `{}`",
                    field.id, rule.field
                ));
            }
        }
    }
    Ok(())
}

fn validate_field_options(
    field: &crate::extension::manifest::ResourceConnectionFormField,
) -> Result<(), ExtensionRuntimeError> {
    use crate::extension::manifest::ResourceConnectionFieldType;
    match field.field_type {
        ResourceConnectionFieldType::Select => {
            let values = field
                .options
                .iter()
                .map(|option| option.value.as_str())
                .collect::<HashSet<_>>();
            if values.is_empty() || values.len() != field.options.len() {
                return invalid_connection(format!(
                    "select field `{}` requires unique options",
                    field.id
                ));
            }
            if field
                .default_value
                .as_deref()
                .is_some_and(|value| !values.contains(value))
            {
                return invalid_connection(format!(
                    "select field `{}` has an unknown default value",
                    field.id
                ));
            }
        }
        ResourceConnectionFieldType::Number => {
            if field
                .default_value
                .as_deref()
                .is_some_and(|value| value.parse::<i64>().is_err())
            {
                return invalid_connection(format!(
                    "number field `{}` has an invalid default value",
                    field.id
                ));
            }
        }
        ResourceConnectionFieldType::Checkbox => {
            if field
                .default_value
                .as_deref()
                .is_some_and(|value| value.parse::<bool>().is_err())
            {
                return invalid_connection(format!(
                    "checkbox field `{}` has an invalid default value",
                    field.id
                ));
            }
        }
        _ => {}
    }
    Ok(())
}

fn invalid_connection<T>(reason: impl Into<String>) -> Result<T, ExtensionRuntimeError> {
    Err(ExtensionRuntimeError::InvalidResourceConnection(
        reason.into(),
    ))
}

impl ExtensionRuntimeCatalog {
    pub(super) fn register_manifest(
        &mut self,
        manifest: Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        self.register_wasm_runtimes(&manifest)?;
        self.register_ipc_runtimes(&manifest)?;
        self.register_shell_views(&manifest)?;
        self.register_resource_connections(&manifest)?;
        self.register_html_preview_transforms(&manifest)?;
        self.register_document_renderers(&manifest)?;
        self.register_document_exporters(&manifest)?;
        self.register_commands(&manifest)?;
        self.register_menu_slots(&manifest);
        self.register_toolbar_slots(&manifest);
        self.register_keybindings(&manifest);
        self.register_remote_file_editors(&manifest)?;
        self.register_db_tree_menus(&manifest);
        Ok(())
    }

    fn register_ipc_runtimes(&mut self, manifest: &Manifest) -> Result<(), ExtensionRuntimeError> {
        for runtime in &manifest.runtime.ipc {
            let key = runtime_key(&manifest.id, &runtime.id);
            if self.ipc_runtimes.contains_key(&key) || self.wasm_runtimes.contains_key(&key) {
                return Err(ExtensionRuntimeError::DuplicateRuntime { id: key });
            }
            let working_dir = resolve_ipc_working_dir(
                &manifest.manifest_dir,
                runtime.entry.working_dir.as_deref(),
            );
            let command = resolve_ipc_command(&runtime.entry.command, &working_dir);
            self.ipc_runtimes.insert(
                key.clone(),
                RegisteredIpcRuntimeBinding {
                    extension_id: manifest.id.clone(),
                    runtime_key: key,
                    extension_root: manifest.manifest_dir.clone(),
                    command,
                    required_spawn_permission: required_spawn_permission(
                        &runtime.entry.command,
                        runtime.entry.working_dir.as_deref(),
                    ),
                    args: runtime.entry.args.clone(),
                    working_dir: Some(working_dir),
                    env: runtime.entry.env.clone(),
                    transport_kind: runtime.transport.kind.clone(),
                    connect_timeout_ms: runtime.transport.connect_timeout_ms,
                    auto_restart: runtime.auto_restart,
                    max_restart_attempts: runtime.max_restart_attempts,
                    shutdown_grace_ms: runtime.shutdown_grace_ms,
                    permissions: manifest.permissions.clone(),
                },
            );
        }
        Ok(())
    }

    fn register_shell_views(&mut self, manifest: &Manifest) -> Result<(), ExtensionRuntimeError> {
        validate_shell_views(manifest).map_err(|error| {
            ExtensionRuntimeError::InvalidShellView {
                field: error.field,
                reason: error.reason,
            }
        })?;
        for view in &manifest.contributes.shell_views {
            let view_key = runtime_key(&manifest.id, &view.id);
            if self.shell_views.contains_key(&view_key) {
                return Err(ExtensionRuntimeError::DuplicateShellView { view_key });
            }
            let backends = view
                .backends
                .iter()
                .map(|(alias, runtime_id)| (alias.clone(), runtime_key(&manifest.id, runtime_id)))
                .collect();
            self.shell_views.insert(
                view_key.clone(),
                RegisteredShellViewContribution {
                    extension_id: manifest.id.clone(),
                    extension_version: manifest.version.clone(),
                    id: view.id.clone(),
                    view_key,
                    title: view.title.clone(),
                    description: view.description.clone(),
                    icon_path: view
                        .icon
                        .as_deref()
                        .map(|icon| resolve_extension_path(&manifest.manifest_dir, icon)),
                    extension_root: manifest.manifest_dir.clone(),
                    entry_path: resolve_extension_path(&manifest.manifest_dir, &view.entry),
                    surface: view.surface,
                    singleton: view.singleton,
                    backends,
                    modules: view.modules.iter().copied().collect(),
                    permissions: manifest.permissions.clone(),
                    shell_api_version: manifest.api.shell.clone(),
                    required_gpui_shell_version: manifest.engines.gpui_shell.clone(),
                },
            );
        }
        Ok(())
    }

    fn register_resource_connections(
        &mut self,
        manifest: &Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        let mut ids = HashSet::new();
        for connection in &manifest.contributes.connections {
            validate_resource_connection(manifest, connection, &mut ids)?;
            let id = connection.id.trim().to_string();
            let key = runtime_key(&manifest.id, &id);
            self.resource_connections.insert(
                key,
                RegisteredResourceConnectionContribution {
                    extension_id: manifest.id.clone(),
                    extension_root: manifest.manifest_dir.clone(),
                    id,
                    label: connection.label.clone(),
                    description: connection.description.clone(),
                    icon_path: connection
                        .icon
                        .as_deref()
                        .map(|icon| resolve_extension_path(&manifest.manifest_dir, icon)),
                    runtime_id: runtime_key(&manifest.id, &connection.runtime_id),
                    resource_type: connection.resource_type.clone(),
                    shell_view_id: connection.shell_view_id.clone(),
                    form: connection.form.clone(),
                },
            );
        }
        Ok(())
    }

    fn register_wasm_runtimes(&mut self, manifest: &Manifest) -> Result<(), ExtensionRuntimeError> {
        for runtime in &manifest.runtime.wasm {
            let key = runtime_key(&manifest.id, &runtime.id);
            if self.wasm_runtimes.contains_key(&key) {
                return Err(ExtensionRuntimeError::DuplicateRuntime { id: key });
            }
            #[cfg(feature = "wasm-components")]
            let module_path = resolve_module_path(&manifest.manifest_dir, &runtime.module);
            #[cfg(feature = "wasm-components")]
            let module_path_for_log = module_path.display().to_string();
            #[cfg(not(feature = "wasm-components"))]
            let module_path_for_log = runtime.module.clone();
            #[cfg(feature = "wasm-components")]
            let config = extension_wasm::WasmRuntimeConfig {
                max_memory_mb: runtime.max_memory_mb,
                fuel_per_call: runtime.fuel_per_call,
                timeout_ms: runtime.timeout_ms,
            };
            tracing::debug!(
                target: "extension_loader",
                kind = "wasm",
                extension_id = %manifest.id,
                runtime_id = %runtime.id,
                runtime_key = %key,
                runtime_kind = ?runtime.kind,
                module = %runtime.module,
                module_path = %module_path_for_log,
                "registered wasm runtime"
            );
            self.wasm_runtimes.insert(
                key.clone(),
                WasmRuntimeBinding {
                    #[cfg(feature = "wasm-components")]
                    extension_id: manifest.id.clone(),
                    #[cfg(feature = "wasm-components")]
                    runtime_key: key,
                    kind: runtime.kind,
                    #[cfg(feature = "wasm-components")]
                    module_path,
                    #[cfg(feature = "wasm-components")]
                    config,
                    permissions: manifest.permissions.clone(),
                },
            );
        }
        Ok(())
    }

    fn register_commands(&mut self, manifest: &Manifest) -> Result<(), ExtensionRuntimeError> {
        for command in &manifest.contributes.commands {
            if command.handler.kind != "wasm" {
                continue;
            }
            let runtime_id = runtime_key(&manifest.id, &command.handler.runtime_id);
            if !self.wasm_runtimes.contains_key(&runtime_id) {
                return Err(ExtensionRuntimeError::UnknownRuntime {
                    command_id: command.id.clone(),
                    runtime_id: command.handler.runtime_id.clone(),
                });
            }
            let function = command
                .handler
                .function
                .clone()
                .unwrap_or_else(|| "invoke".to_string());
            self.commands.register(command_descriptor(
                &manifest.id,
                command,
                runtime_id,
                function,
            ))?;
        }
        Ok(())
    }

    fn register_html_preview_transforms(
        &mut self,
        manifest: &Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        for transform in &manifest.contributes.html_preview_transforms {
            let runtime_id = runtime_key(&manifest.id, &transform.runtime_id);
            if !self.wasm_runtimes.contains_key(&runtime_id) {
                return Err(ExtensionRuntimeError::UnknownRuntime {
                    command_id: transform.id.clone(),
                    runtime_id: transform.runtime_id.clone(),
                });
            }
            let assets_root = resolve_asset_root(&manifest.manifest_dir, &transform.assets);
            html_preview::register_extension_asset_root(&manifest.id, assets_root.clone());
            self.html_preview_transforms
                .push(RegisteredHtmlPreviewTransform {
                    extension_id: manifest.id.clone(),
                    id: transform.id.clone(),
                    runtime_id,
                    function: transform.function.clone(),
                    languages: transform.languages.clone(),
                    assets_root,
                });
        }
        Ok(())
    }

    fn register_document_renderers(
        &mut self,
        manifest: &Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        for renderer in &manifest.contributes.document_renderers {
            let runtime_id = runtime_key(&manifest.id, &renderer.runtime_id);
            if !self.wasm_runtimes.contains_key(&runtime_id) {
                return Err(ExtensionRuntimeError::UnknownRuntime {
                    command_id: renderer.id.clone(),
                    runtime_id: renderer.runtime_id.clone(),
                });
            }
            self.document_renderers.push(RegisteredDocumentRenderer {
                extension_id: manifest.id.clone(),
                id: renderer.id.clone(),
                display_name: renderer.display_name.clone(),
                runtime_id,
                function: renderer.function.clone(),
                block_kinds: renderer.block_kinds.clone(),
                output_media_types: renderer.output_media_types.clone(),
                priority: renderer.priority,
            });
        }
        Ok(())
    }

    fn register_document_exporters(
        &mut self,
        manifest: &Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        for exporter in &manifest.contributes.document_exporters {
            let runtime_id = runtime_key(&manifest.id, &exporter.runtime_id);
            if !self.wasm_runtimes.contains_key(&runtime_id) {
                return Err(ExtensionRuntimeError::UnknownRuntime {
                    command_id: exporter.id.clone(),
                    runtime_id: exporter.runtime_id.clone(),
                });
            }
            self.document_exporters.push(RegisteredDocumentExporter {
                extension_id: manifest.id.clone(),
                id: exporter.id.clone(),
                display_name: exporter.display_name.clone(),
                runtime_id,
                function: exporter.function.clone(),
                formats: exporter.formats.clone(),
                output_media_types: exporter.output_media_types.clone(),
                priority: exporter.priority,
            });
        }
        Ok(())
    }

    fn register_menu_slots(&mut self, manifest: &Manifest) {
        for (position, menus) in &manifest.contributes.menus {
            for menu in menus {
                self.menu_slots.add(
                    position.clone(),
                    slot_item_from_menu(
                        &manifest.id,
                        menu.command.id.clone(),
                        menu.label.clone(),
                        menu.group.clone(),
                        menu.when.clone(),
                        Value::Null,
                    ),
                );
            }
        }
    }

    fn register_toolbar_slots(&mut self, manifest: &Manifest) {
        for (position, toolbars) in &manifest.contributes.toolbars {
            for toolbar in toolbars {
                self.toolbar_slots.add(
                    position.clone(),
                    slot_item_from_menu(
                        &manifest.id,
                        toolbar.command.id.clone(),
                        toolbar.label.clone(),
                        toolbar.group.clone(),
                        toolbar.when.clone(),
                        Value::Null,
                    ),
                );
            }
        }
    }

    fn register_keybindings(&mut self, manifest: &Manifest) {
        self.keybindings
            .extend(manifest.contributes.keybindings.iter().map(|binding| {
                RegisteredKeybindingContribution {
                    extension_id: manifest.id.clone(),
                    command: binding.command.clone(),
                    key: binding.key.clone(),
                    mac: binding.mac.clone(),
                    linux: binding.linux.clone(),
                    windows: binding.windows.clone(),
                    when_clause: binding.when.clone(),
                }
            }));
    }

    fn register_remote_file_editors(
        &mut self,
        manifest: &Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        for editor in &manifest.contributes.remote_file_editors {
            validate_remote_file_editor(editor)?;
            self.remote_file_editors
                .push(RegisteredRemoteFileEditorContribution {
                    extension_id: manifest.id.clone(),
                    id: editor.id.clone(),
                    editor_key: runtime_key(&manifest.id, &editor.id),
                    display_name: editor.display_name.clone(),
                    platforms: editor.platforms.clone(),
                    file_masks: editor.file_masks.clone(),
                    priority: editor.priority,
                    command: RegisteredRemoteFileEditorCommand {
                        launch_mode: editor.command.launch_mode,
                        program_candidates: editor.command.program_candidates.clone(),
                        args: editor.command.args.clone(),
                    },
                });
        }
        Ok(())
    }

    fn register_db_tree_menus(&mut self, manifest: &Manifest) {
        let command_titles = command_titles(manifest);
        for (position, menus) in &manifest.contributes.menus {
            if !position.starts_with("db.tree.") {
                continue;
            }
            for menu in menus {
                let command_id = menu.command.id.clone();
                let label = menu
                    .label
                    .clone()
                    .or_else(|| command_titles.get(command_id.as_str()).cloned())
                    .unwrap_or_else(|| command_id.clone());
                self.db_tree_menus.push(RegisteredDbTreeMenuContribution {
                    position: position.clone(),
                    item: DbTreeExtensionMenuItem {
                        extension_id: manifest.id.clone(),
                        command_id,
                        label,
                        group: menu.group.clone(),
                        when_clause: menu.when.clone(),
                        requires_active: menu.requires_active,
                    },
                });
            }
        }
    }
}

fn validate_remote_file_editor(
    editor: &crate::extension::manifest::contributes::RemoteFileEditorContrib,
) -> Result<(), ExtensionRuntimeError> {
    let invalid = |reason: &str| ExtensionRuntimeError::InvalidRemoteFileEditor {
        editor_id: editor.id.clone(),
        reason: reason.to_string(),
    };
    if editor.id.trim().is_empty() {
        return Err(invalid("id must not be empty"));
    }
    if editor.display_name.trim().is_empty() {
        return Err(invalid("displayName must not be empty"));
    }
    if editor.command.program_candidates.is_empty()
        || editor
            .command
            .program_candidates
            .iter()
            .any(|candidate| candidate.trim().is_empty())
    {
        return Err(invalid("command.programCandidates must not be empty"));
    }
    if editor.platforms.iter().any(|platform| {
        !matches!(
            platform.to_ascii_lowercase().as_str(),
            "windows" | "macos" | "linux"
        )
    }) {
        return Err(invalid("platforms contains an unsupported value"));
    }
    Ok(())
}

#[cfg(feature = "wasm-components")]
fn resolve_module_path(manifest_dir: &Path, module: &str) -> PathBuf {
    let path = Path::new(module);
    if path.is_absolute() {
        return path.to_path_buf();
    }
    manifest_dir.join(path).components().collect()
}

fn resolve_extension_path(manifest_dir: &Path, path: &str) -> PathBuf {
    manifest_dir.join(path).components().collect()
}

fn resolve_asset_root(manifest_dir: &Path, assets: &str) -> PathBuf {
    if assets.trim().is_empty() {
        return manifest_dir.to_path_buf();
    }
    manifest_dir.join(assets).components().collect()
}

fn resolve_ipc_working_dir(manifest_dir: &Path, working_dir: Option<&str>) -> PathBuf {
    working_dir
        .filter(|path| !path.trim().is_empty())
        .map(|path| manifest_dir.join(path).components().collect())
        .unwrap_or_else(|| manifest_dir.to_path_buf())
}

fn resolve_ipc_command(command: &str, working_dir: &Path) -> PathBuf {
    let path = Path::new(command);
    if path.is_absolute() || (!command.contains('/') && !command.contains('\\')) {
        path.to_path_buf()
    } else {
        working_dir.join(path).components().collect()
    }
}

pub(super) fn load_installed_composite_manifests(
    root: &Path,
) -> Result<Vec<Manifest>, ExtensionRuntimeError> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let host_version = current_host_version();
    let host_apis = HostApiVersions::current();
    let mut manifests = Vec::new();
    for entry in std::fs::read_dir(root).map_err(ExtensionRuntimeError::ReadCompositeRoot)? {
        let Ok(entry) = entry else {
            continue;
        };
        if !is_candidate_composite_dir(&entry) {
            continue;
        }
        match load_and_check(&entry.path(), &host_version, &host_apis) {
            Ok(manifest) => {
                let wasm_runtimes: Vec<_> = manifest
                    .runtime
                    .wasm
                    .iter()
                    .map(|runtime| runtime_key(&manifest.id, &runtime.id))
                    .collect();
                tracing::debug!(
                    target: "extension_loader",
                    kind = "wasm",
                    extension_id = %manifest.id,
                    name = %manifest.name,
                    version = %manifest.version,
                    path = %manifest.manifest_dir.display(),
                    wasm_runtimes = ?wasm_runtimes,
                    "loaded composite extension manifest"
                );
                manifests.push(manifest);
            }
            Err(ManifestError::NotFound(_)) => {}
            Err(err) => {
                let key = format!("skip:{}:{err:?}", entry.path().display());
                if should_log_wasm_registration_once(&key) {
                    tracing::warn!(
                        target: "extension_loader",
                        kind = "wasm",
                        "skip composite extension {} while building catalog: {err:?}",
                        entry.path().display()
                    );
                }
            }
        }
    }
    Ok(manifests)
}

fn should_log_wasm_registration_once(key: &str) -> bool {
    let seen = WASM_REGISTRATION_LOG_KEYS.get_or_init(|| Mutex::new(HashSet::new()));
    seen.lock()
        .map(|mut seen| seen.insert(key.to_string()))
        .unwrap_or(true)
}

fn is_candidate_composite_dir(entry: &std::fs::DirEntry) -> bool {
    let Ok(file_type) = entry.file_type() else {
        return false;
    };
    file_type.is_dir() && is_active_install_dir_name(&entry.file_name())
}

fn command_titles(manifest: &Manifest) -> BTreeMap<&str, String> {
    manifest
        .contributes
        .commands
        .iter()
        .map(|command| (command.id.as_str(), command.title.clone()))
        .collect()
}
