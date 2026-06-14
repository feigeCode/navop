use std::collections::BTreeMap;
use std::path::Path;

use db_view::extension_menu::DbTreeExtensionMenuRegistry;
use one_core::{
    command_registry::{CommandHandler, CommandRegistry},
    contributions::SlotRegistry,
};

use crate::extension::manifest::{Manifest, WasmRuntimeKind};

use super::registration::load_installed_composite_manifests;
use super::types::{
    ExtensionRuntimeError, RegisteredDbTreeMenuContribution, RegisteredKeybindingContribution,
    WasmRuntimeBinding,
};

#[derive(Debug)]
pub struct ExtensionRuntimeCatalog {
    pub(super) commands: CommandRegistry,
    pub(super) wasm_runtimes: BTreeMap<String, WasmRuntimeBinding>,
    pub(super) db_tree_menus: Vec<RegisteredDbTreeMenuContribution>,
    pub(super) toolbar_slots: SlotRegistry,
    pub(super) menu_slots: SlotRegistry,
    pub(super) keybindings: Vec<RegisteredKeybindingContribution>,
}

impl ExtensionRuntimeCatalog {
    pub fn empty() -> Self {
        Self {
            commands: CommandRegistry::new(),
            wasm_runtimes: BTreeMap::new(),
            db_tree_menus: Vec::new(),
            toolbar_slots: SlotRegistry::default(),
            menu_slots: SlotRegistry::default(),
            keybindings: Vec::new(),
        }
    }

    pub fn from_manifests(manifests: Vec<Manifest>) -> Result<Self, ExtensionRuntimeError> {
        let mut catalog = Self::empty();
        for manifest in manifests {
            catalog.register_manifest(manifest)?;
        }
        Ok(catalog)
    }

    pub fn from_installed_composite_root(root: &Path) -> Result<Self, ExtensionRuntimeError> {
        let manifests = load_installed_composite_manifests(root)?;
        Self::from_manifests(manifests)
    }

    pub fn db_tree_menu_registry(&self) -> DbTreeExtensionMenuRegistry {
        let mut registry = DbTreeExtensionMenuRegistry::default();
        for menu in &self.db_tree_menus {
            registry.add(menu.position.clone(), menu.item.clone());
        }
        registry
    }

    pub fn component_permissions_for_command(
        &self,
        command_id: &str,
    ) -> extension_wasm::WasmResult<Vec<String>> {
        Ok(self
            .component_binding_for_command(command_id)?
            .permissions
            .clone())
    }

    pub(super) fn component_binding_for_command(
        &self,
        command_id: &str,
    ) -> extension_wasm::WasmResult<&WasmRuntimeBinding> {
        let Some(command) = self.commands.get(command_id) else {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                command_id.to_string(),
            ));
        };
        let CommandHandler::Wasm { runtime_id, .. } = &command.handler else {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                command_id.to_string(),
            ));
        };
        let Some(binding) = self.wasm_runtimes.get(runtime_id) else {
            return Err(extension_wasm::WasmError::ModuleNotFound(
                runtime_id.clone(),
            ));
        };
        if binding.kind != WasmRuntimeKind::Component {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                command_id.to_string(),
            ));
        }
        Ok(binding)
    }
}
