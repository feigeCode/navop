use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use db_view::extension_menu::DbTreeExtensionMenuRegistry;
use one_core::{
    command_registry::{CommandHandler, CommandRegistry},
    contributions::SlotRegistry,
};

use crate::extension::manifest::{Manifest, WasmRuntimeKind};

use super::registration::load_installed_composite_manifests;
use super::types::{
    ExtensionRuntimeError, RegisteredDbTreeMenuContribution, RegisteredDocumentRenderer,
    RegisteredHtmlPreviewTransform, RegisteredKeybindingContribution,
    RegisteredRemoteFileEditorContribution, WasmRuntimeBinding,
};

static WASM_CATALOG_LOG_KEYS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

#[derive(Debug)]
pub struct ExtensionRuntimeCatalog {
    pub(super) commands: CommandRegistry,
    pub(super) wasm_runtimes: BTreeMap<String, WasmRuntimeBinding>,
    pub(super) db_tree_menus: Vec<RegisteredDbTreeMenuContribution>,
    pub(super) toolbar_slots: SlotRegistry,
    pub(super) menu_slots: SlotRegistry,
    pub(super) keybindings: Vec<RegisteredKeybindingContribution>,
    pub(super) html_preview_transforms: Vec<RegisteredHtmlPreviewTransform>,
    pub(super) document_renderers: Vec<RegisteredDocumentRenderer>,
    pub(super) remote_file_editors: Vec<RegisteredRemoteFileEditorContribution>,
}

#[derive(Debug)]
pub struct ExtensionRuntimeCatalogLoadReport {
    pub catalog: ExtensionRuntimeCatalog,
    pub loaded: Vec<CompositeExtensionLoadedEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompositeExtensionLoadedEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub dir: std::path::PathBuf,
    pub wasm_runtimes: Vec<String>,
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
            html_preview_transforms: Vec::new(),
            document_renderers: Vec::new(),
            remote_file_editors: Vec::new(),
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
        Ok(Self::from_installed_composite_root_with_report(root)?.catalog)
    }

    pub fn from_installed_composite_root_with_report(
        root: &Path,
    ) -> Result<ExtensionRuntimeCatalogLoadReport, ExtensionRuntimeError> {
        let manifests = load_installed_composite_manifests(root)?;
        let loaded: Vec<_> = manifests
            .iter()
            .map(CompositeExtensionLoadedEntry::from_manifest)
            .collect();
        let catalog = Self::from_manifests(manifests)?;
        let loaded_ids: Vec<_> = loaded.iter().map(|entry| entry.id.as_str()).collect();
        let summary_key = format!("catalog:{}:{loaded_ids:?}", root.display());
        if should_log_wasm_catalog_once(&summary_key) {
            tracing::info!(
                target: "extension_loader",
                kind = "wasm",
                root = %root.display(),
                loaded = loaded.len(),
                extensions = ?loaded_ids,
                "loaded wasm composite extension catalog"
            );
        }
        Ok(ExtensionRuntimeCatalogLoadReport { catalog, loaded })
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
    ) -> Result<Vec<String>, ExtensionRuntimeError> {
        Ok(self
            .component_binding_for_command(command_id)?
            .permissions
            .clone())
    }

    pub fn html_preview_transforms_for_language(
        &self,
        language: &str,
    ) -> Vec<&RegisteredHtmlPreviewTransform> {
        let language = language.to_ascii_lowercase();
        self.html_preview_transforms
            .iter()
            .filter(|transform| {
                transform
                    .languages
                    .iter()
                    .any(|candidate| candidate.eq_ignore_ascii_case(&language))
            })
            .collect()
    }

    pub fn remote_file_editors(&self) -> &[RegisteredRemoteFileEditorContribution] {
        &self.remote_file_editors
    }

    pub fn document_renderer_for_kind(
        &self,
        block_kind: &str,
    ) -> Option<&RegisteredDocumentRenderer> {
        self.document_renderers
            .iter()
            .filter(|renderer| {
                renderer
                    .block_kinds
                    .iter()
                    .any(|kind| kind.eq_ignore_ascii_case(block_kind))
            })
            .max_by_key(|renderer| renderer.priority)
    }

    #[cfg(feature = "wasm-components")]
    pub async fn render_document(
        &self,
        request: extension_wasm::DocumentRenderRequest,
    ) -> extension_wasm::WasmResult<Option<extension_wasm::DocumentRenderArtifact>> {
        let Some(renderer) = self.document_renderer_for_kind(&request.renderer) else {
            return Ok(None);
        };
        let Some(binding) = self.wasm_runtimes.get(&renderer.runtime_id) else {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                renderer.runtime_id.clone(),
            ));
        };
        if binding.kind != WasmRuntimeKind::Component {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                renderer.runtime_id.clone(),
            ));
        }
        extension_wasm::DocumentRendererRuntime::from_file_with_config(
            renderer.id.clone(),
            &binding.module_path,
            binding.config.clone(),
        )?
        .render(request)
        .await
        .map(Some)
    }

    #[cfg(feature = "wasm-components")]
    pub async fn transform_html_preview(
        &self,
        language: &str,
        html: &str,
    ) -> extension_wasm::WasmResult<Option<html_preview::HtmlPreviewTransformOutput>> {
        let Some(transform) = self
            .html_preview_transforms_for_language(language)
            .into_iter()
            .next()
        else {
            return Ok(None);
        };
        let Some(binding) = self.wasm_runtimes.get(&transform.runtime_id) else {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                transform.runtime_id.clone(),
            ));
        };
        if binding.kind != WasmRuntimeKind::Component {
            return Err(extension_wasm::WasmError::FunctionNotFound(
                transform.runtime_id.clone(),
            ));
        }
        let runtime = extension_wasm::HtmlPreviewTransformRuntime::from_file(
            transform.id.clone(),
            &binding.module_path,
        )?;
        runtime
            .transform_html(language.to_string(), html.to_string())
            .await
            .map(Some)
    }

    #[cfg(not(feature = "wasm-components"))]
    pub async fn transform_html_preview(
        &self,
        _language: &str,
        _html: &str,
    ) -> Result<Option<html_preview::HtmlPreviewTransformOutput>, ExtensionRuntimeError> {
        Ok(None)
    }

    pub(super) fn component_binding_for_command(
        &self,
        command_id: &str,
    ) -> Result<&WasmRuntimeBinding, ExtensionRuntimeError> {
        let Some(command) = self.commands.get(command_id) else {
            return Err(ExtensionRuntimeError::CommandNotFound {
                command_id: command_id.to_string(),
            });
        };
        let CommandHandler::Wasm { runtime_id, .. } = &command.handler else {
            return Err(ExtensionRuntimeError::UnsupportedCommand {
                command_id: command_id.to_string(),
            });
        };
        let Some(binding) = self.wasm_runtimes.get(runtime_id) else {
            return Err(ExtensionRuntimeError::RuntimeBindingNotFound {
                command_id: command_id.to_string(),
                runtime_id: runtime_id.clone(),
            });
        };
        if binding.kind != WasmRuntimeKind::Component {
            return Err(ExtensionRuntimeError::UnsupportedCommand {
                command_id: command_id.to_string(),
            });
        }
        Ok(binding)
    }
}

fn should_log_wasm_catalog_once(key: &str) -> bool {
    let seen = WASM_CATALOG_LOG_KEYS.get_or_init(|| Mutex::new(HashSet::new()));
    seen.lock()
        .map(|mut seen| seen.insert(key.to_string()))
        .unwrap_or(true)
}

impl CompositeExtensionLoadedEntry {
    fn from_manifest(manifest: &Manifest) -> Self {
        Self {
            id: manifest.id.clone(),
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            dir: manifest.manifest_dir.clone(),
            wasm_runtimes: manifest
                .runtime
                .wasm
                .iter()
                .map(|runtime| format!("{}::{}", manifest.id, runtime.id))
                .collect(),
        }
    }
}
