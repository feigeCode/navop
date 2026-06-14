use std::path::Path;

use anyhow::Result;

use crate::extension::{ExtensionKind, ExtensionProvider, ExtensionSummary};

pub struct LanguageExtensionProvider;

impl ExtensionProvider for LanguageExtensionProvider {
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::Language
    }

    fn list_installed(&self, root: &Path) -> Result<Vec<ExtensionSummary>> {
        let list = gpui_component::highlighter::list_installed(root)?;
        Ok(list
            .into_iter()
            .map(|summary| {
                let description = describe_language(&summary.name, &summary.file_extensions);
                ExtensionSummary::new(
                    ExtensionKind::Language,
                    summary.name,
                    summary.version,
                    summary.path,
                )
                .with_description(description)
                .with_file_extensions(summary.file_extensions)
            })
            .collect())
    }

    fn install_from_dir(&self, dir: &Path) -> Result<ExtensionSummary> {
        let extension = gpui_component::highlighter::InstalledExtension::load_from_dir(dir)?;
        extension.register(gpui_component::highlighter::LanguageRegistry::singleton())?;
        let description = describe_language(
            &extension.manifest.name,
            &extension.manifest.file_extensions,
        );
        Ok(ExtensionSummary::new(
            ExtensionKind::Language,
            extension.manifest.name.clone(),
            extension.manifest.version.clone(),
            dir.to_path_buf(),
        )
        .with_description(description)
        .with_file_extensions(extension.manifest.file_extensions.clone()))
    }

    fn uninstall(&self, dir: &Path) -> Result<String> {
        gpui_component::highlighter::InstalledExtension::uninstall(
            dir,
            gpui_component::highlighter::LanguageRegistry::singleton(),
        )
    }
}

fn describe_language(name: &str, file_extensions: &[String]) -> String {
    if file_extensions.is_empty() {
        format!("Tree-sitter {name} 语法")
    } else {
        let exts = file_extensions
            .iter()
            .map(|extension| format!(".{extension}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Tree-sitter {name} 语法 ({exts})")
    }
}
