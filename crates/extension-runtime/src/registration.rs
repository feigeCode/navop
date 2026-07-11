use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use db_view::extension_menu::DbTreeExtensionMenuItem;
use serde_json::Value;

use crate::extension::manifest::{
    HostApiVersions, Manifest, ManifestError, current_host_version, load_and_check,
};

use super::catalog::ExtensionRuntimeCatalog;
use super::types::{
    ExtensionRuntimeError, RegisteredDbTreeMenuContribution, RegisteredHtmlPreviewTransform,
    RegisteredKeybindingContribution, RegisteredRemoteFileEditorCommand,
    RegisteredRemoteFileEditorContribution, WasmRuntimeBinding, command_descriptor, runtime_key,
    slot_item_from_menu,
};

static WASM_REGISTRATION_LOG_KEYS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

impl ExtensionRuntimeCatalog {
    pub(super) fn register_manifest(
        &mut self,
        manifest: Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        self.register_wasm_runtimes(&manifest)?;
        self.register_html_preview_transforms(&manifest)?;
        self.register_commands(&manifest)?;
        self.register_menu_slots(&manifest);
        self.register_toolbar_slots(&manifest);
        self.register_keybindings(&manifest);
        self.register_remote_file_editors(&manifest)?;
        self.register_db_tree_menus(&manifest);
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
            let base_config = extension_wasm::WasmRuntimeConfig::default();
            #[cfg(feature = "wasm-components")]
            let config = extension_wasm::WasmRuntimeConfig {
                max_memory_mb: runtime.max_memory_mb,
                fuel_per_call: runtime.fuel_per_call,
                ..base_config
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

fn resolve_asset_root(manifest_dir: &Path, assets: &str) -> PathBuf {
    if assets.trim().is_empty() {
        return manifest_dir.to_path_buf();
    }
    manifest_dir.join(assets).components().collect()
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
    file_type.is_dir() && !entry.file_name().to_string_lossy().starts_with('_')
}

fn command_titles(manifest: &Manifest) -> BTreeMap<&str, String> {
    manifest
        .contributes
        .commands
        .iter()
        .map(|command| (command.id.as_str(), command.title.clone()))
        .collect()
}
