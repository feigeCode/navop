# Notes local knowledge workspace

Notes manages Markdown, rich-text, and whiteboard material in a local folder, keeping query records, operational procedures, and project knowledge beside active connections. Files remain in the directory you choose; Navop is not a replacement for backup or version control.

## Choose a workspace folder

Select a readable and writable folder on first use, or change the Notes path in Settings. Use clear project directories and remember that changing the configured path does not move old files. Cloud drives, network folders, and Git repositories may work, but their synchronization and locking rules still apply.

Do not store database passwords, SSH private keys, master keys, or tokens in notes. Review the folder's own access controls before sharing it across devices.

## Create Markdown, rich text, and whiteboards

Markdown works well for technical text and portable code samples; rich text favors quick visual formatting; whiteboards organize free-form diagrams and ideas. Choose a durable format for long-lived content because advanced features can vary across renderers.

Opening a `.md` file through the operating system sends it to the Notes Markdown editor. Relative images and links still depend on that file's location. Preserve the complete resource folder for whiteboards and rich documents rather than copying only a primary file.

## Edit and use shortcuts

Common editing, selection, search, and save shortcuts are listed under Settings → Shortcuts → Notes. Observe unsaved-change prompts when closing. Auto-save behavior depends on current settings and format; actively save important work and keep revisions elsewhere.

After pasting rich content, inspect structure and external resources. Use stable relative paths for links, images, and code-related assets.

## Manage resources and external changes

Images and attachments live in the note directory or its resource area. Moving or deleting them can break references, so search for usage before reorganizing. Avoid accumulating very large binaries as ordinary note assets.

If another editor changes a file, compare it with local unsaved content before reloading. Git conflicts, cloud conflict copies, and encoding changes must be resolved at the file level.

## Back up and share safely

Back up the entire Notes directory, including hidden metadata and resources. Markdown is broadly portable, while extension syntax, rich text, and whiteboards may depend on Navop or a document-renderer extension. Preview in the target environment before distribution.

Redact connection names, internal addresses, query results, logs, screenshot metadata, and personal information before sharing.
