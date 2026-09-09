# External GPUI Component

Navop no longer embeds `gpui-component`, its macros, or its assets in this
workspace. CI, release, and normal development use the fork pinned in the
workspace manifest:

```text
https://github.com/feigeCode/gpui-component.git
rev e6459613910143b817eecbacc8767976e86cac84
```

The checkout belongs to `https://github.com/feigeCode/gpui-component` and the
Navop compatibility branch is `navop-gpui-ce`. It is based on the
`gpui-ce/gpui-component` adaptation and pins GPUI to:

```text
https://github.com/feigeCode/gpui-ce.git
c13e489375c7cd93838bdabef0213771bc2fe456
```

## Ownership Boundary

- `gpui-component`: generic controls, assets, theme compatibility, icons, dialog
  lifecycle, and small backward-compatible builder aliases.
- `crates/one_ui`: Navop composite UI including `EditTable`, `LargeTextEditor`,
  `ContentState`, `PanelHeader`, `StatusBar`, `IconButton`, resize handles, and
  date/time editors.
- `crates/extension-runtime`: language extension manifests, installed extension
  discovery, SHA-256 verification, and lazy WASM language loading.
- feature crates: page-specific presentation and business interactions.

`one_ui` must not depend on `one-core`. Settings persistence and shortcut
resolution are injected by `main`.

## Updating

Use the synchronization script from the Navop repository:

```bash
# Verify the merge in a temporary worktree without changing either branch.
script/sync-gpui-component.sh \
  --component-dir /path/to/gpui-component \
  --no-fetch \
  --dry-run

# Merge, verify, push the compatibility branch, and update Navop revisions,
# package versions, shell compatibility constants, examples, docs, and lockfile.
script/sync-gpui-component.sh \
  --component-dir /path/to/gpui-component \
  --fetch-source git@github.com:feigeCode/gpui-component.git \
  --push \
  --update-navop
```

The script merges in a temporary worktree first and enables Git rerere for known
conflicts. Unknown conflicts or failed verification stop without changing the
compatibility branch or Navop files. It never guesses a semantic conflict.

To update only Navop after an already-pushed component commit:

```bash
python3 script/update_gpui_component_revision.py \
  --component-dir /path/to/gpui-component \
  --revision <commit>
```

The updater reads actual package versions from `cargo metadata`; do not edit the
five dependency versions or `GPUI_SHELL_VERSION` independently.

For network fetches on this machine, Clash is available at
`http://127.0.0.1:7897`.

## Verification

```bash
cargo check -p main
cargo test -p main --no-run
cargo test -p one-ui
cargo test -p extension-runtime
cargo test -p extension-wasm
cargo test -p extension-plugin-adapter
cargo check -p terminal_view --all-targets
```

The retired declarative UI crate is intentionally absent. Plugin UI uses
`gpui-component-shell`; Navop keeps only the generic shell embedding lifecycle
and provider HostModules.

## SQL Editor Extensions

SQL Editor remains a first-class capability. The component fork exposes generic,
SQL-neutral Editor contracts for:

- monotonic document revision tracking;
- gutter markers, stable lane layout, hitboxes, and click events;
- continuous multi-line range fills and frames;
- non-document inline widgets;
- completion request invalidation and explicit metadata refresh.

`one_ui::ExtendedEditor` owns generic signature-help lifecycle and presentation.
`db_view` owns all SQL parsing, metadata, statement execution, marker state,
INSERT hints, diagnostics, completion, hover, signature providers, and SQL menu
actions. Keep this split when syncing upstream: add generic Editor hooks to the
component layer, reusable presentation to `one_ui`, and SQL behavior to
`db_view`. Do not reintroduce a separate Navop Input engine.
