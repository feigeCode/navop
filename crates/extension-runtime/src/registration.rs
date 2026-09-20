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
    RegisteredResourceWorkbenchContribution, RegisteredShellViewContribution, WasmRuntimeBinding,
    command_descriptor, runtime_key, slot_item_from_menu,
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
        self.register_resource_workbenches(&manifest)?;
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
                    category: view.category.clone(),
                    keywords: view.keywords.clone().unwrap_or_default(),
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

    fn register_resource_workbenches(
        &mut self,
        manifest: &Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        let mut bound_connections = HashSet::new();
        for workbench in &manifest.contributes.resource_workbenches {
            validate_resource_workbench(manifest, workbench, &mut bound_connections)?;
            let key = runtime_key(&manifest.id, &workbench.id);
            if self.resource_workbenches.contains_key(&key) {
                return Err(ExtensionRuntimeError::InvalidResourceWorkbench(format!(
                    "duplicate workbench id `{}`",
                    workbench.id
                )));
            }
            self.resource_workbenches.insert(
                key,
                RegisteredResourceWorkbenchContribution::from_manifest(&manifest.id, workbench),
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

/// 校验嵌入 Shell 视图引用:viewId 必须在同扩展 shellViews 中声明,
/// 且模块声明为 Context + Workbench、不含 raw 模块。
/// 页面级 renderer、区域级 source 与 root `layout.renderer` 共用。
fn validate_shell_renderer(
    manifest: &Manifest,
    view_id: &str,
    invalid: &dyn Fn(&str) -> ExtensionRuntimeError,
) -> Result<(), ExtensionRuntimeError> {
    let Some(view) = manifest
        .contributes
        .shell_views
        .iter()
        .find(|view| view.id == view_id)
    else {
        return Err(invalid("shell renderer references an unknown shell view"));
    };
    let modules = &view.modules;
    if !modules.contains(&crate::extension::manifest::ShellHostModule::Context)
        || !modules.contains(&crate::extension::manifest::ShellHostModule::Workbench)
    {
        return Err(invalid(
            "embedded shell renderer requires context and workbench modules",
        ));
    }
    if modules.iter().any(|module| {
        matches!(
            module,
            crate::extension::manifest::ShellHostModule::Resource
                | crate::extension::manifest::ShellHostModule::Job
                | crate::extension::manifest::ShellHostModule::Event
                | crate::extension::manifest::ShellHostModule::Blob
                | crate::extension::manifest::ShellHostModule::Runtime
                | crate::extension::manifest::ShellHostModule::Dev
        )
    }) {
        return Err(invalid(
            "embedded shell renderer cannot request raw resource/job/event/blob/runtime/dev modules",
        ));
    }
    Ok(())
}

fn validate_resource_workbench(
    manifest: &Manifest,
    workbench: &crate::extension::manifest::ResourceWorkbenchContrib,
    bound_connections: &mut HashSet<String>,
) -> Result<(), ExtensionRuntimeError> {
    use crate::extension::manifest as m;

    let invalid = |reason: &str| {
        ExtensionRuntimeError::InvalidResourceWorkbench(format!("{}: {reason}", workbench.id))
    };
    if workbench.schema_version != 3 {
        return Err(invalid(
            "unsupported schemaVersion; migrate pages to primitives (schemaVersion 3)",
        ));
    }
    if workbench.id.trim().is_empty() || workbench.title.trim().is_empty() {
        return Err(invalid("id and title must not be empty"));
    }
    if workbench.connection_ids.is_empty() || workbench.runtime_id.trim().is_empty() {
        return Err(invalid("connectionIds and runtimeId must not be empty"));
    }
    if !manifest
        .runtime
        .ipc
        .iter()
        .any(|runtime| runtime.id == workbench.runtime_id)
    {
        return Err(invalid("runtimeId does not reference an IPC runtime"));
    }
    let connections = &manifest.contributes.connections;
    for connection_id in &workbench.connection_ids {
        if !bound_connections.insert(connection_id.clone()) {
            return Err(invalid("a connection is bound to more than one workbench"));
        }
        let Some(connection) = connections
            .iter()
            .find(|connection| connection.id == *connection_id)
        else {
            return Err(invalid("connectionIds references an unknown connection"));
        };
        if connection.runtime_id != workbench.runtime_id
            || connection.resource_type != workbench.resource_type
        {
            return Err(invalid("runtimeId/resourceType does not match connection"));
        }
    }
    if !workbench
        .pages
        .iter()
        .any(|page| page.id == workbench.default_page)
    {
        return Err(invalid("defaultPage references an unknown page"));
    }
    let page_exists = |page_id: &str| workbench.pages.iter().any(|page| page.id == page_id);
    let operation_exists = |operation: &str| workbench.operations.contains_key(operation);
    for page in &workbench.pages {
        if let Some(action) = page.load.as_ref() {
            if !operation_exists(&action.operation) {
                return Err(invalid("page load references an unknown operation"));
            }
        }
        if page.stack.is_empty() {
            return Err(invalid("page stack must declare at least one primitive"));
        }
        // 渲染器一页只呈现一个内容原语(table/form/viewer/stream/tasks/terminal
        // 择一)。多原语 stack 会被静默丢弃其中之一,造成"安装合法但界面不忠实
        // 于声明"——在校验期明确拒绝,需要组合时拆成多个页面。
        if page.stack.len() > 1 {
            return Err(invalid(
                "page stack must declare exactly one primitive; split the page instead",
            ));
        }
        for primitive in &page.stack {
            match primitive {
                m::ResourceWorkbenchPrimitive::Table(table) => {
                    if let Some(open) = table.open.as_ref() {
                        if !page_exists(&open.page_id) {
                            return Err(invalid("table open references an unknown page"));
                        }
                        validate_route_bindings(&open.route, "table open", &invalid)?;
                    }
                    for action in &table.actions {
                        if !operation_exists(&action.operation) {
                            return Err(invalid("table action references an unknown operation"));
                        }
                        if action.id.trim().is_empty() || action.label.trim().is_empty() {
                            return Err(invalid("table action id and label must not be empty"));
                        }
                    }
                }
                m::ResourceWorkbenchPrimitive::Form(form) => {
                    if !operation_exists(&form.submit.operation) {
                        return Err(invalid("form submit references an unknown operation"));
                    }
                }
                m::ResourceWorkbenchPrimitive::Terminal(terminal) => {
                    let has_command = terminal
                        .command
                        .as_deref()
                        .is_some_and(|command| !command.trim().is_empty());
                    let has_operation = terminal.operation.is_some();
                    if !has_command && !has_operation {
                        return Err(invalid(
                            "terminal requires either a command or an operation",
                        ));
                    }
                    if has_command && has_operation {
                        return Err(invalid(
                            "terminal command and operation are mutually exclusive",
                        ));
                    }
                    if let Some(operation) = terminal.operation.as_ref() {
                        if !operation_exists(&operation.operation) {
                            return Err(invalid("terminal references an unknown operation"));
                        }
                        // `operation` 形式是**预留声明**,宿主侧明确返回
                        // "runtime terminal operation ... is not supported yet":
                        // 扩展协议目前只有请求-响应与 job 两种形态,没有
                        // provider PTY 流式通道,交互式 exec 无法复用。此前这里
                        // 只校验 operation 存在,于是扩展能装成功、用户点开必定
                        // 失败 —— 典型"声明了但不可用"。在校验期直接拒绝,让失败
                        // 发生在安装而不是使用时。
                        return Err(invalid(
                            "terminal.operation is reserved but not implemented; \
                             declare a local command instead",
                        ));
                    }
                }
                m::ResourceWorkbenchPrimitive::Viewer(_)
                | m::ResourceWorkbenchPrimitive::Stream
                | m::ResourceWorkbenchPrimitive::Tasks => {}
            }
        }
        for link in &page.links {
            if !page_exists(&link.page_id) {
                return Err(invalid("page link references an unknown page"));
            }
            validate_route_bindings(&link.route, "page link", &invalid)?;
        }
        // 页面 tabGroupId 必须命中 layout 中的某个组。
        if let Some(group_id) = page.tab_group_id.as_deref() {
            let in_group = workbench
                .layout
                .as_ref()
                .and_then(|layout| layout.center.as_ref())
                .and_then(|center| match &center.source {
                    m::ResourceWorkbenchCenterSource::Pages { tab_groups } => {
                        Some(tab_groups.iter().any(|group| group.id == group_id))
                    }
                    _ => None,
                })
                .unwrap_or(false);
            if !in_group {
                return Err(invalid("page tabGroupId references an unknown tab group"));
            }
        }
        if matches!(page.renderer.kind, m::ResourceWorkbenchRendererKind::Shell) {
            let Some(view_id) = page.renderer.view_id.as_deref() else {
                return Err(invalid("shell renderer requires viewId"));
            };
            validate_shell_renderer(manifest, view_id, &invalid)?;
        }
    }
    validate_workbench_layout(manifest, workbench, &invalid)?;
    Ok(())
}

/// route 绑定只接受**导航发生那一刻取得到值**的来源。
///
/// `input`(表单输入)与 `paging`(列表分页)只在 provider 参数侧有意义:导航
/// 由行点击 / 链接触发,那时既没有表单输入也没有列表上下文。此前注册期不校验,
/// 扩展声明了它们,`build_route` 只会静默产出空值 —— 与 `connection` 当初
/// 在路由侧被无条件忽略是同一类"声明合法但永远拿不到值"。
fn validate_route_bindings(
    bindings: &std::collections::BTreeMap<
        String,
        crate::extension::manifest::ResourceWorkbenchBinding,
    >,
    location: &str,
    invalid: &dyn Fn(&str) -> ExtensionRuntimeError,
) -> Result<(), ExtensionRuntimeError> {
    use crate::extension::manifest::ResourceWorkbenchBindingSource as S;

    for (name, binding) in bindings {
        if matches!(binding.source, S::Input | S::Paging) {
            return Err(invalid(&format!(
                "{location} route binding `{name}` uses an input/paging source, \
                 which is only meaningful for operation params"
            )));
        }
    }
    Ok(())
}

/// layout 声明校验:root Shell 与区域互斥;tab 组、树根、状态栏
/// 引用的页面/操作必须存在;区域 Shell 源走嵌入校验。
fn validate_workbench_layout(
    manifest: &Manifest,
    workbench: &crate::extension::manifest::ResourceWorkbenchContrib,
    invalid: &dyn Fn(&str) -> ExtensionRuntimeError,
) -> Result<(), ExtensionRuntimeError> {
    use crate::extension::manifest as m;

    let Some(layout) = workbench.layout.as_ref() else {
        return Ok(());
    };
    let has_region = [
        layout.left.is_some(),
        layout.center.is_some(),
        layout.right.is_some(),
        layout.bottom.is_some(),
    ]
    .into_iter()
    .any(|present| present);
    if let Some(renderer) = layout.renderer.as_ref() {
        if has_region {
            return Err(invalid(
                "layout renderer and regions are mutually exclusive",
            ));
        }
        if renderer.kind == m::ResourceWorkbenchRendererKind::Shell {
            let Some(view_id) = renderer.view_id.as_deref() else {
                return Err(invalid("layout renderer requires viewId"));
            };
            validate_shell_renderer(manifest, view_id, invalid)?;
        } else {
            return Err(invalid("layout renderer only supports the shell kind"));
        }
        return Ok(());
    }
    let page_exists = |page_id: &str| workbench.pages.iter().any(|page| page.id == page_id);
    let check_shell_source =
        |source: &m::ResourceWorkbenchShellSource,
         invalid: &dyn Fn(&str) -> ExtensionRuntimeError| {
            if let Some(fallback) = source.fallback.as_deref() {
                if fallback != "none" {
                    return Err(invalid(
                        "region shell source only supports the `none` fallback",
                    ));
                }
            }
            validate_shell_renderer(manifest, &source.view_id, invalid)
        };
    if let Some(left) = layout.left.as_ref() {
        match &left.source {
            m::ResourceWorkbenchNavSource::Tree { roots } => {
                if roots.is_empty() {
                    return Err(invalid("tree nav requires at least one root"));
                }
                for root in roots {
                    if !page_exists(&root.page_id) {
                        return Err(invalid("tree root references an unknown page"));
                    }
                    if let Some(children) = root.children.as_ref() {
                        validate_tree_children(
                            children,
                            &|operation| workbench.operations.contains_key(operation),
                            &page_exists,
                            &invalid,
                            TREE_CHILDREN_MAX_DEPTH,
                        )?;
                    }
                }
            }
            m::ResourceWorkbenchNavSource::List { items } => {
                if items.is_empty() {
                    return Err(invalid("list nav requires at least one entry"));
                }
                for entry in items {
                    if !page_exists(&entry.page_id) {
                        return Err(invalid("list nav references an unknown page"));
                    }
                }
            }
            m::ResourceWorkbenchNavSource::Shell(source) => check_shell_source(source, invalid)?,
            m::ResourceWorkbenchNavSource::None => {}
        }
    }
    if let Some(center) = layout.center.as_ref() {
        if let m::ResourceWorkbenchCenterSource::Pages { tab_groups } = &center.source {
            for group in tab_groups {
                if group.id.trim().is_empty() {
                    return Err(invalid("tab group id must not be empty"));
                }
                for tab in &group.tabs {
                    if !page_exists(&tab.page_id) {
                        return Err(invalid("tab group references an unknown page"));
                    }
                    validate_route_bindings(&tab.route, "tab group tab", invalid)?;
                }
            }
        }
    }
    if let Some(side) = layout.right.as_ref() {
        if let m::ResourceWorkbenchSideSource::Shell(source) = &side.source {
            check_shell_source(source, invalid)?;
        }
    }
    if let Some(bottom) = layout.bottom.as_ref() {
        match &bottom.source {
            m::ResourceWorkbenchBottomSource::Status { operation, items } => {
                let Some(op) = workbench.operations.get(operation) else {
                    return Err(invalid("status bar references an unknown operation"));
                };
                if op.mode != m::ResourceWorkbenchOperationMode::Invoke {
                    return Err(invalid("status bar operation must be an invoke operation"));
                }
                if items.is_empty() {
                    return Err(invalid("status bar requires at least one item"));
                }
                for item in items {
                    if item.format == m::ResourceWorkbenchStatusFormat::Pair
                        && item.other_path.is_none()
                    {
                        return Err(invalid("pair format requires otherPath"));
                    }
                }
            }
            m::ResourceWorkbenchBottomSource::Shell(source) => check_shell_source(source, invalid)?,
            m::ResourceWorkbenchBottomSource::None => {}
        }
    }
    Ok(())
}

/// 树 children 递归深度上限。
///
/// 树的形状是值而不是引用,循环在类型上不可能出现;上限挡的是病态 manifest
/// (例如机器生成的几十层嵌套),让它在安装期被拒绝,而不是渲染时递归爆栈。
const TREE_CHILDREN_MAX_DEPTH: usize = 8;

/// 校验一层树 children 声明(remote 或 static),并递归到下一层。
///
/// 两种形态的字段是**互斥**的:同一个结构体承载了两套字段,如果只校验"该有的
/// 有",写错形态(比如 static 里混了 `operation`)会静默按其中一侧解释,表现成
/// "点开是空的"。因此这里两个方向都查。
///
/// 依赖只收两个闭包(页面是否存在 / 操作是否存在)而不是整个 workbench:
/// 校验逻辑本身与 workbench 的其它字段无关,收窄依赖才能单独跑契约测试。
fn validate_tree_children(
    children: &crate::extension::manifest::ResourceWorkbenchTreeChildren,
    operation_exists: &dyn Fn(&str) -> bool,
    page_exists: &dyn Fn(&str) -> bool,
    invalid: &dyn Fn(&str) -> ExtensionRuntimeError,
    depth: usize,
) -> Result<(), ExtensionRuntimeError> {
    use crate::extension::manifest as m;

    if depth == 0 {
        return Err(invalid(
            "tree children nest deeper than the supported depth",
        ));
    }
    let validate_open =
        |open: &m::ResourceWorkbenchOpen, location: &str| -> Result<(), ExtensionRuntimeError> {
            if !page_exists(&open.page_id) {
                return Err(invalid(&format!("{location} references an unknown page")));
            }
            validate_route_bindings(&open.route, location, invalid)
        };

    if children.is_remote() {
        if !children.items.is_empty() {
            return Err(invalid(
                "tree children declare `items` without `kind: \"static\"`",
            ));
        }
        let operation = children
            .operation
            .as_deref()
            .filter(|op| !op.trim().is_empty());
        let Some(operation) = operation else {
            return Err(invalid("tree children require an operation"));
        };
        if !operation_exists(operation) {
            return Err(invalid("tree children references an unknown operation"));
        }
        for (field, value) in [
            ("itemsPath", children.items_path.as_deref()),
            ("labelPath", children.label_path.as_deref()),
        ] {
            if !value.is_some_and(|value| !value.trim().is_empty()) {
                return Err(invalid(&format!(
                    "tree children require a non-empty {field}"
                )));
            }
        }
        if let Some(open) = children.open.as_ref() {
            validate_open(open, "tree children open")?;
        }
        if let Some(next) = children.children.as_deref() {
            validate_tree_children(next, operation_exists, page_exists, invalid, depth - 1)?;
        }
        return Ok(());
    }

    // `kind: "static"`:零请求展开的功能子节点。
    for (field, present) in [
        ("operation", children.operation.is_some()),
        ("itemsPath", children.items_path.is_some()),
        ("labelPath", children.label_path.is_some()),
        ("keyPaths", !children.key_paths.is_empty()),
        ("open", children.open.is_some()),
        ("children", children.children.is_some()),
    ] {
        if present {
            return Err(invalid(&format!(
                "static tree children must not declare `{field}`; put it on the item instead"
            )));
        }
    }
    if children.items.is_empty() {
        return Err(invalid("static tree children require at least one item"));
    }
    let mut ids = HashSet::new();
    for item in &children.items {
        if item.id.trim().is_empty() || item.title.trim().is_empty() {
            return Err(invalid("static tree item id and title must not be empty"));
        }
        // 节点键 = 父键 + `\u{1}` + 行键。id 里出现控制字符会让键无法再拆回去
        // (父键与行键的边界丢失),所以在这里挡住,而不是在渲染时错位。
        if item.id.chars().any(char::is_control) {
            return Err(invalid(
                "static tree item id must not contain control characters",
            ));
        }
        if !ids.insert(item.id.as_str()) {
            return Err(invalid(
                "static tree item ids must be unique within a level",
            ));
        }
        validate_open(&item.open, "static tree item open")?;
        if let Some(next) = item.children.as_deref() {
            validate_tree_children(next, operation_exists, page_exists, invalid, depth - 1)?;
        }
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

pub(crate) fn load_installed_composite_manifests(
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

#[cfg(test)]
mod tree_children_tests {
    use super::*;
    use serde_json::json;

    fn reason(result: Result<(), ExtensionRuntimeError>) -> String {
        match result {
            Err(ExtensionRuntimeError::InvalidResourceWorkbench(reason)) => reason,
            other => panic!("expected an invalid-resource-workbench error, got {other:?}"),
        }
    }

    /// 只依赖「页面存在吗 / 操作存在吗」两个谓词,不需要构造整个 workbench。
    fn validate_children(
        declaration: serde_json::Value,
        pages: &[&str],
        operations: &[&str],
    ) -> Result<(), ExtensionRuntimeError> {
        let children: crate::extension::manifest::ResourceWorkbenchTreeChildren =
            serde_json::from_value(declaration).expect("declaration must parse");
        let invalid =
            |reason: &str| ExtensionRuntimeError::InvalidResourceWorkbench(reason.to_string());
        validate_tree_children(
            &children,
            &|operation| operations.contains(&operation),
            &|page| pages.contains(&page),
            &invalid,
            TREE_CHILDREN_MAX_DEPTH,
        )
    }

    fn static_item(id: &str, page: &str) -> serde_json::Value {
        json!({"id": id, "title": id, "open": {"pageId": page, "route": {}}})
    }

    /// 静态形态的整个意义就是"不发请求"。校验若要求存在某个 operation,
    /// 扩展就只能声明一个用不到的操作来凑数 —— 能力等于没实现。
    #[test]
    fn static_children_need_no_operation_at_all() {
        assert!(
            validate_children(
                json!({"kind": "static", "items": [static_item("mapping", "index-mapping")]}),
                &["index-mapping"],
                &[],
            )
            .is_ok()
        );
    }

    /// 旧 manifest(不写 `kind`)必须继续按 remote 校验:缺 operation 就是非法,
    /// 而不是"被当成 static 然后零请求展开"。
    #[test]
    fn remote_children_without_an_operation_are_rejected() {
        let message = reason(validate_children(
            json!({"itemsPath": "/items", "labelPath": "/name"}),
            &["page"],
            &["list"],
        ));
        assert!(message.contains("require an operation"), "{message}");
    }

    #[test]
    fn remote_children_must_reference_a_declared_operation() {
        let message = reason(validate_children(
            json!({"operation": "nope", "itemsPath": "/items", "labelPath": "/name"}),
            &["page"],
            &["list"],
        ));
        assert!(message.contains("unknown operation"), "{message}");
    }

    #[test]
    fn remote_children_require_non_empty_paths() {
        for declaration in [
            json!({"operation": "list", "labelPath": "/name"}),
            json!({"operation": "list", "itemsPath": "/items"}),
            json!({"operation": "list", "itemsPath": "  ", "labelPath": "/name"}),
        ] {
            let message = reason(validate_children(declaration, &["page"], &["list"]));
            assert!(message.contains("non-empty"), "{message}");
        }
    }

    /// 形态互斥的两个方向都要查。写错一侧时若静默按另一侧解释,
    /// 表现是"点开是空的",而安装期不会有任何提示。
    #[test]
    fn the_two_child_shapes_are_mutually_exclusive() {
        let remote_with_items = reason(validate_children(
            json!({
                "operation": "list",
                "itemsPath": "/items",
                "labelPath": "/name",
                "items": [static_item("mapping", "index-mapping")],
            }),
            &["index-mapping"],
            &["list"],
        ));
        assert!(
            remote_with_items.contains("without `kind"),
            "{remote_with_items}"
        );

        for field in [
            "operation",
            "itemsPath",
            "labelPath",
            "keyPaths",
            "open",
            "children",
        ] {
            let mut declaration = json!({
                "kind": "static",
                "items": [static_item("mapping", "index-mapping")],
            });
            declaration[field] = match field {
                "keyPaths" => json!(["/name"]),
                "open" => json!({"pageId": "index-mapping", "route": {}}),
                "children" => {
                    json!({"kind": "static", "items": [static_item("x", "index-mapping")]})
                }
                _ => json!("value"),
            };
            let message = reason(validate_children(
                declaration,
                &["index-mapping"],
                &["list"],
            ));
            assert!(
                message.contains(&format!("must not declare `{field}`")),
                "declaring `{field}` on a static collection must fail: {message}"
            );
        }
    }

    #[test]
    fn static_children_require_at_least_one_item() {
        let message = reason(validate_children(
            json!({"kind": "static", "items": []}),
            &["page"],
            &[],
        ));
        assert!(message.contains("at least one item"), "{message}");
    }

    #[test]
    fn static_item_ids_are_unique_per_level() {
        let message = reason(validate_children(
            json!({"kind": "static", "items": [
                static_item("mapping", "index-mapping"),
                static_item("mapping", "index-settings"),
            ]}),
            &["index-mapping", "index-settings"],
            &[],
        ));
        assert!(message.contains("unique within a level"), "{message}");

        // 不同层之间重名是允许的:节点键带父键前缀。
        assert!(validate_children(
            json!({"kind": "static", "items": [{
                "id": "group",
                "title": "Group",
                "open": {"pageId": "index-mapping", "route": {}},
                "children": {"kind": "static", "items": [static_item("group", "index-mapping")]},
            }]}),
            &["index-mapping"],
            &[],
        )
        .is_ok());
    }

    /// 节点键 = 父键 + `\u{1}` + 行键。id 里带分隔符会让键拆不回去。
    #[test]
    fn static_item_ids_must_not_contain_control_characters() {
        let message = reason(validate_children(
            json!({"kind": "static", "items": [
                {"id": "a\u{1}b", "title": "A", "open": {"pageId": "page", "route": {}}},
            ]}),
            &["page"],
            &[],
        ));
        assert!(message.contains("control characters"), "{message}");
    }

    #[test]
    fn static_and_remote_open_targets_must_exist() {
        let message = reason(validate_children(
            json!({"kind": "static", "items": [static_item("mapping", "ghost")]}),
            &["index-mapping"],
            &[],
        ));
        assert!(message.contains("unknown page"), "{message}");

        let message = reason(validate_children(
            json!({"operation": "list", "itemsPath": "/i", "labelPath": "/n",
                   "children": {"kind": "static", "items": [static_item("m", "ghost")]}}),
            &["page"],
            &["list"],
        ));
        assert!(message.contains("unknown page"), "{message}");
    }

    #[test]
    fn nesting_beyond_the_depth_limit_is_rejected() {
        let leaf = json!({"kind": "static", "items": [static_item("leaf", "page")]});
        let mut within_limit = leaf.clone();
        for _ in 0..TREE_CHILDREN_MAX_DEPTH - 1 {
            within_limit = json!({"kind": "static", "items": [{
                "id": "group",
                "title": "Group",
                "open": {"pageId": "page", "route": {}},
                "children": within_limit,
            }]});
        }
        assert!(validate_children(within_limit.clone(), &["page"], &[]).is_ok());

        let mut too_deep = within_limit;
        for _ in 0..2 {
            too_deep = json!({"kind": "static", "items": [{
                "id": "group",
                "title": "Group",
                "open": {"pageId": "page", "route": {}},
                "children": too_deep,
            }]});
        }
        let message = reason(validate_children(too_deep, &["page"], &[]));
        assert!(
            message.contains("deeper than the supported depth"),
            "{message}"
        );
    }
}
