use crate::path_policy::{MARKDOWN_SUFFIX, document_display_name};
use crate::storage_support::write_text_atomic_new;
use crate::{DocumentDescriptor, DocumentFormat, NotesStorage};
use anyhow::{Context, Result, bail};
use cditor_app::{EditorDocument, MarkdownExportMode};
use std::fs;
use std::path::Path;
use uuid::Uuid;

impl NotesStorage {
    pub fn convert_rich_text_to_markdown(
        &self,
        relative_path: &Path,
    ) -> Result<DocumentDescriptor> {
        let source = self.descriptor(relative_path)?;
        if source.format != DocumentFormat::RichText {
            bail!("only rich-text documents can be converted to Markdown");
        }
        let document = EditorDocument::from_json(&fs::read_to_string(&source.absolute_path)?)?;
        let exported = document.export_markdown(MarkdownExportMode::Strict)?;
        let file_name = source
            .relative_path
            .file_name()
            .context("rich-text document has no file name")?
            .to_string_lossy();
        let stem = document_display_name(&file_name)
            .map(|(name, _)| name)
            .context("invalid rich-text document name")?;
        let target_name = format!("{stem}{MARKDOWN_SUFFIX}");
        let relative_target = source
            .relative_path
            .parent()
            .unwrap_or(Path::new(""))
            .join(&target_name);
        let absolute_target = source
            .absolute_path
            .parent()
            .context("rich-text document has no parent")?
            .join(target_name);
        if absolute_target.exists() {
            bail!(
                "Markdown document already exists: {}",
                relative_target.display()
            );
        }
        write_text_atomic_new(&absolute_target, &exported.markdown)?;
        let descriptor = DocumentDescriptor {
            document_id: Uuid::new_v4().to_string(),
            format: DocumentFormat::Markdown,
            relative_path: relative_target,
            absolute_path: absolute_target,
        };
        if let Err(error) = self.record_descriptor(&descriptor) {
            let _ = fs::remove_file(&descriptor.absolute_path);
            return Err(error);
        }
        Ok(descriptor)
    }
}
