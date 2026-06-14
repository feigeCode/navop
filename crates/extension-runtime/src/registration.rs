use std::collections::BTreeMap;
use std::path::Path;

use db_view::extension_menu::DbTreeExtensionMenuItem;
use serde_json::Value;

use crate::extension::manifest::{
    HostApiVersions, Manifest, ManifestError, current_host_version, load_and_check,
};

use super::catalog::ExtensionRuntimeCatalog;
use super::types::{
    ExtensionRuntimeError, RegisteredDbTreeMenuContribution, RegisteredKeybindingContribution,
    WasmRuntimeBinding, command_descriptor, runtime_key, slot_item_from_menu,
};

impl ExtensionRuntimeCatalog {
    pub(super) fn register_manifest(
        &mut self,
        manifest: Manifest,
    ) -> Result<(), ExtensionRuntimeError> {
        self.register_wasm_runtimes(&manifest)?;
        self.register_commands(&manifest)?;
        self.register_menu_slots(&manifest);
        self.register_toolbar_slots(&manifest);
        self.register_keybindings(&manifest);
        self.register_db_tree_menus(&manifest);
        Ok(())
    }

    fn register_wasm_runtimes(&mut self, manifest: &Manifest) -> Result<(), ExtensionRuntimeError> {
        for runtime in &manifest.runtime.wasm {
            let key = runtime_key(&manifest.id, &runtime.id);
            if self.wasm_runtimes.contains_key(&key) {
                return Err(ExtensionRuntimeError::DuplicateRuntime { id: key });
            }
            let base_config = extension_wasm::WasmRuntimeConfig::default();
            let module_path =
                base_config.resolve_module_path(&manifest.manifest_dir, &runtime.module);
            let config = extension_wasm::WasmRuntimeConfig {
                max_memory_mb: runtime.max_memory_mb,
                fuel_per_call: runtime.fuel_per_call,
                ..base_config
            };
            self.wasm_runtimes.insert(
                key.clone(),
                WasmRuntimeBinding {
                    extension_id: manifest.id.clone(),
                    runtime_key: key,
                    kind: runtime.kind,
                    module_path,
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
            Ok(manifest) => manifests.push(manifest),
            Err(ManifestError::NotFound(_)) => {}
            Err(err) => {
                tracing::warn!(
                    "skip composite extension {} while building catalog: {err:?}",
                    entry.path().display()
                );
            }
        }
    }
    Ok(manifests)
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
