# Notes local knowledge workspace

Notes manages Markdown, rich-text, and whiteboard material in a local folder, keeping query records, operational procedures, and project knowledge beside active connections. Files remain in the directory you choose; Navop is not a replacement for backup or version control.

<div class="notes-screenshot-grid">
  <figure><img src="/images/markdown.png" alt="Markdown Notes workspace"><figcaption>Markdown syntax, restricted HTML, and table images</figcaption></figure>
  <figure><img src="/images/richtext.png" alt="Rich-text Notes workspace"><figcaption>Rich-text blocks and embedded content</figcaption></figure>
  <figure><img src="/images/whiteboard.png" alt="Notes whiteboard workspace"><figcaption>Whiteboard canvas and freeform layout</figcaption></figure>
</div>

## Choose a workspace folder

Select a readable and writable folder on first use, or change the Notes path in Settings. Use clear project directories and remember that changing the configured path does not move old files. Cloud drives, network folders, and Git repositories may work, but their synchronization and locking rules still apply.

Do not store database passwords, SSH private keys, master keys, or tokens in notes. Review the folder's own access controls before sharing it across devices.

## Create Markdown, rich text, and whiteboards

Markdown works well for technical text and portable code samples; rich text favors quick visual formatting; whiteboards organize free-form diagrams and ideas. The Markdown document view renders standard syntax, safely restricted HTML, and images inside Markdown tables; it is not an unrestricted browser rendering environment. Choose a durable format for long-lived content because advanced features can vary across renderers.

Opening a `.md` file through the operating system sends it to the Notes Markdown editor. Relative images and media resolve from the directory containing that Markdown file. When a rich-text document with a whiteboard is converted to Markdown, Navop exports a preview image rather than editable whiteboard source data. Keep the original document and complete resource directory if the whiteboard must remain editable.

## Edit and use shortcuts

Common editing, selection, search, and save shortcuts are listed under Settings → Shortcuts → Notes. Observe unsaved-change prompts when closing. Auto-save behavior depends on current settings and format; actively save important work and keep revisions elsewhere.

After pasting rich content, inspect structure and external resources. Use stable relative paths for links, images, and code-related assets.

## Export HTML, PDF, and Word

With the Notes document exporter extension installed, export the current document as self-contained HTML, PDF, or Word DOCX. The Rust WASM exporter does not require local Chrome, Office, or a host conversion library. Choose the output directory deliberately and inspect images, code, Mermaid, and math in the target environment.

## Manage resources and external changes

Images and attachments live in the note directory or its resource area. Moving or deleting them can break references, so search for usage before reorganizing. Avoid accumulating very large binaries as ordinary note assets.

If another editor changes a file, compare it with local unsaved content before reloading. Git conflicts, cloud conflict copies, and encoding changes must be resolved at the file level.

## Back up and share safely

Back up the entire Notes directory, including hidden metadata and resources. Markdown is broadly portable, while extension syntax, rich text, and whiteboards may depend on Navop or a document-renderer extension. Preview in the target environment before distribution.

Redact connection names, internal addresses, query results, logs, screenshot metadata, and personal information before sharing.
