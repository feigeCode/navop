# Navop Safe Rebrand Cleanup Design

## Goal

Record the repository-wide OnetCli-to-Navop migration audit and fix a first batch of clearly incorrect Navop branding without changing persisted identities, external protocols, extension contracts, or upgrade compatibility.

## Scope

This batch will:

- save the complete migration audit to `docs/migration/onetcli-to-navop-audit.md`;
- make `navop --help` identify the command as `navop` instead of `onetcli`;
- update user-visible extension compatibility errors to call the product Navop while retaining the manifest field name `engines.onetcli`;
- update `CLAUDE.md` so it describes Navop as the current product while preserving accurate references to historical internal identifiers such as `OnetCliApp` and `onetcli_app.rs`;
- remove the obsolete `.gitignore` entry for `resources/macos/OnetCli.icns`.

This batch will not modify the four files that already contain user changes: `crates/core/src/license/models.rs`, `main/src/home/home_tabs.rs`, `main/src/home_tab.rs`, and `main/src/setting_tab.rs`.

## Compatibility Boundary

The following identifiers remain unchanged because they are persisted identities, public contracts, ecosystem identifiers, or upgrade paths:

- macOS bundle identifier `com.onetcli.app`;
- `one-hub` configuration, database, authentication, license, key, and extension paths;
- `.onetcli-sync` and personal-sync `app: onetcli`;
- `ProviderType::OnetCli` and serialized provider id `onet_cli`;
- `engines.onetcli` as a manifest field;
- `com.onetcli.*` extension ids and the `onetcli-extensions` repository URLs;
- `onetcli-public-mcp`, MCP server migration state, and `onetcli.*` tool ids;
- `kss://onetcli`, `sht://onetcli`, `/*onetcli-ipc-wire*/`, extension socket variables, and shell-integration markers;
- update support for `OnetCli.app`, `onetcli.exe`, and Linux `onetcli`;
- Debian/RPM replacement metadata for the historical `onetcli` package;
- the `onetcli-upstream` Git remote.

## Implementation Design

### Audit document

The audit document will organize findings into already migrated, change now, migrate with compatibility, retain, and internal-cleanup-later categories. Each actionable finding will include file references, risk, and the recommended migration strategy.

### CLI branding

The installed executable is already `navop`, so Clap's explicit command name will change from `onetcli` to `navop`. The parser and Rust crate/type names remain unchanged. A test will first demonstrate that help output still exposes the legacy command name, then protect the new `navop` usage text.

### Extension error branding

Only human-readable error text changes. The serialized manifest structure and compatibility lookup continue to use `manifest.engines.onetcli`. Tests will verify that relevant compatibility errors mention Navop while still identifying `engines.onetcli` when the field itself is the problem.

### Development documentation and ignore rule

`CLAUDE.md` will distinguish the current Navop product identity from internal legacy code names. The obsolete `OnetCli.icns` ignore rule will be removed because the repository now uses the tracked `resources/macos/Navop.icns` asset.

## Testing and Verification

- Run the CLI test that checks the help brand and command name through a red-green TDD cycle.
- Run extension manifest/versioning tests after updating expected error branding.
- Run formatting checks for changed Rust files.
- Run targeted tests for `onetcli_cli` and `extension-runtime`.
- Search the changed user-facing surfaces to confirm the intended old names are removed while the compatibility identifiers listed above remain intact.
- Review the final diff to ensure the four pre-existing user-modified files were not changed by this work.

## Acceptance Criteria

- The migration audit exists as a repository document and clearly separates safe changes from compatibility-sensitive identifiers.
- `navop --help` presents `navop` as the command name.
- Extension compatibility errors refer to Navop as the product and retain the literal field name `engines.onetcli` where technically necessary.
- `CLAUDE.md` no longer introduces the repository as OnetCli.
- The obsolete `resources/macos/OnetCli.icns` ignore entry is absent.
- No compatibility-sensitive identifier listed in this design is renamed or removed.
- Existing user changes remain untouched.
