# Workspace Explorer

`workspace_explorer` provides the local workspace experience embedded in a terminal view:

- a lazily expanded file tree;
- Git working-tree change discovery and per-file diff loading;
- a multi-tab local text editor with save, reload, search, soft-wrap, and dirty-close handling;
- a read-only diff viewer for Git changes.

The crate owns filesystem and Git work. Host crates only create the explorer/editor entities,
mount them in their layout, and react to `WorkspaceEditorEvent::VisibilityChanged`.
