# Shared Team Connection Form Design

## Goal

Move duplicated team-assignment behavior and translations out of individual connection forms into one reusable workspace crate so future team-form changes have a single implementation point.

## Scope

The new `connection-form` crate owns:

- the GPUI `TeamSelectItem` model;
- localized personal/team/key-status labels;
- construction and initial selection of the team select state;
- reading and validating the selected team before save;
- applying `team_id` and new-connection `owner_id` ownership metadata;
- refresh-safe replacement of team options while preserving the selected team when it still exists.

The crate does not own protocol fields, form layout, workspace selection, sync checkboxes, persistence, notifications, or team-key management. Domain data and key validation remain in `one-core`.

## Architecture

`one-core` remains the domain layer. `connection-form` depends on `one-core`, `gpui`, `gpui-component`, and `rust-i18n`, and exposes a narrow `team` API. Connection UI crates depend on `connection-form` and keep their existing visual structure.

All `TeamSync` strings used by connection forms move to `crates/connection_form/locales/connection_form.yml`. Consumer crates call public label/helper functions instead of resolving `TeamSync.*` in their own locale bundles.

## Public Contract

- `TeamSelectItem::personal()` and `TeamSelectItem::from_team()` produce localized options.
- `team_select_items()` always places the personal option first.
- `create_team_select()` builds GPUI state and restores an optional existing `team_id`.
- `selected_team_id()` reads the normalized selected value.
- `validate_selected_team()` delegates key readiness validation to `one-core` and returns the selected id.
- `apply_team_assignment()` applies validated `team_id` and preserves an existing owner while assigning the current cloud user to newly created connections.
- label functions expose localized team field text to consumer layouts.

## Migration

Migrate DB, Redis, MongoDB, SSH, serial, port forwarding, and remote desktop forms. Remove local `TeamSelectItem`, status-formatting helpers, and duplicated `TeamSync` locale blocks once no local references remain. Main keeps team-management strings that are unrelated to connection form selection.

## Compatibility

Existing form layout, option ordering, edit-mode selection, key validation, sync flags, and persistence behavior remain unchanged. Existing connections retain their owner. New connections still use the active cloud user as owner.

## Testing and Verification

- Unit tests cover option ordering, missing/cached key status labels, selected-id normalization, and edit/new ownership behavior.
- Targeted crate tests/checks verify each migrated form compiles against the shared API.
- A repository search verifies no duplicate `TeamSelectItem` implementations or connection-form `TeamSync` blocks remain.
- Clippy runs with warnings denied for the new crate and affected crates.
