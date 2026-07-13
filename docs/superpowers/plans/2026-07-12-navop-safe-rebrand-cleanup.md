# Navop Safe Rebrand Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Save the OnetCli-to-Navop migration audit and correct a first batch of user-visible legacy branding without changing compatibility-sensitive identifiers.

**Architecture:** Keep product display branding separate from persisted and protocol identities. Change only the CLI help label, human-readable extension errors, development documentation, and an obsolete ignore rule; protect Rust behavior changes with focused tests.

**Tech Stack:** Rust 2024, Clap, thiserror, Cargo tests, Markdown.

---

## Execution Status

Completed on 2026-07-12 in the current `dev` workspace without creating a commit. The CLI red test failed on the static Clap command metadata (`onetcli`) before the implementation change and passed after switching it to `navop`. Both extension error-branding tests failed before the message changes and passed afterward. Final targeted verification passed for `onetcli_cli` and the `extension-runtime` manifest tests.

### Task 1: Save the migration audit

**Files:**
- Create: `docs/migration/onetcli-to-navop-audit.md`

- [x] **Step 1: Write the audit document**

Document the already migrated surfaces, immediate fixes, compatibility-sensitive identifiers, internal cleanup candidates, and recommended migration order. Explicitly record that Bundle ID, `one-hub`, `.onetcli-sync`, provider ids, extension ids, protocol prefixes, tool ids, helper ids, update aliases, package replacement metadata, and the upstream remote are outside this batch.

- [x] **Step 2: Check the document for incomplete sections**

Run:

```bash
rtk rg -n 'TBD|TODO|待定|稍后补充' docs/migration/onetcli-to-navop-audit.md
```

Expected: no matches.

### Task 2: Make CLI help identify Navop

**Files:**
- Modify: `crates/onetcli_cli/src/tests.rs:10-19`
- Modify: `crates/onetcli_cli/src/lib.rs:241-244`

- [x] **Step 1: Write the failing help-brand test**

Replace the legacy-name assertion with:

```rust
#[test]
fn help_uses_navop_brand_and_command_name() {
    use clap::CommandFactory;

    let help = CliArgs::try_parse_from(["navop", "--help"])
        .unwrap_err()
        .to_string();

    assert_eq!("navop", CliArgs::command().get_name());
    assert!(help.contains("Navop desktop app and automation commands"));
    assert!(help.contains("Usage: navop"));
    assert!(!help.contains("Usage: onetcli"));
}
```

- [x] **Step 2: Run the test and verify RED**

Run:

```bash
rtk cargo test -p onetcli_cli help_uses_navop_brand_and_command_name
```

Expected: FAIL because the static Clap command metadata is still named `onetcli`. The argv-derived Usage text alone is insufficient to detect this regression.

- [x] **Step 3: Apply the minimal CLI change**

Change the Clap declaration to:

```rust
#[derive(Debug, Parser)]
#[command(name = "navop")]
#[command(about = "Navop desktop app and automation commands")]
struct CliArgs {
```

- [x] **Step 4: Run the focused test and verify GREEN**

Run:

```bash
rtk cargo test -p onetcli_cli help_uses_navop_brand_and_command_name
```

Expected: PASS.

- [x] **Step 5: Run the complete CLI crate tests**

Run:

```bash
rtk cargo test -p onetcli_cli
```

Expected: all tests pass.

### Task 3: Brand extension compatibility errors as Navop

**Files:**
- Modify: `crates/extension-runtime/src/extension/manifest/versioning.rs:88-104`
- Test: `crates/extension-runtime/src/extension/manifest/versioning.rs`

- [x] **Step 1: Add failing error-display tests**

Append this test module:

```rust
#[cfg(test)]
mod tests {
    use super::CompatibilityError;

    #[test]
    fn compatibility_errors_use_navop_product_name() {
        let schema_error = CompatibilityError::SchemaVersionTooNew { found: 2, max: 1 };
        let mismatch_error = CompatibilityError::HostVersionMismatch {
            required: ">=1.0.0".to_string(),
            current: "0.8.6".to_string(),
        };

        for message in [schema_error.to_string(), mismatch_error.to_string()] {
            assert!(message.contains("Navop"));
            assert!(!message.contains("升级 onetcli"));
            assert!(!message.contains("当前 onetcli"));
        }
    }

    #[test]
    fn missing_engine_error_keeps_field_name_and_uses_navop_brand() {
        let message = CompatibilityError::EnginesOnetcliMissing.to_string();

        assert!(message.contains("engines.onetcli"));
        assert!(message.contains("Navop"));
    }
}
```

- [x] **Step 2: Run the tests and verify RED**

Run:

```bash
rtk cargo test -p extension-runtime compatibility_errors_use_navop_product_name
rtk cargo test -p extension-runtime missing_engine_error_keeps_field_name_and_uses_navop_brand
```

Expected: both tests fail because current messages use the old product name.

- [x] **Step 3: Apply the minimal message changes**

Use these messages while retaining the protocol field name:

```rust
#[error("manifest schema 版本 {found} 高于宿主支持的最高版本 {max},请升级 Navop")]
SchemaVersionTooNew { found: u32, max: u32 },

#[error("engines.onetcli 字段为空,需要声明依赖的 Navop 版本范围")]
EnginesOnetcliMissing,

#[error("engines.onetcli {required:?} 不是合法 SemVer range: {reason}")]
EnginesOnetcliInvalid { required: String, reason: String },

#[error("扩展要求 Navop {required:?},当前 Navop 版本 {current},请升级或寻找兼容版本")]
HostVersionMismatch { required: String, current: String },
```

- [x] **Step 4: Run focused tests and verify GREEN**

Run:

```bash
rtk cargo test -p extension-runtime compatibility_errors_use_navop_product_name
rtk cargo test -p extension-runtime missing_engine_error_keeps_field_name_and_uses_navop_brand
```

Expected: both tests pass.

- [x] **Step 5: Run extension manifest tests**

Run:

```bash
rtk cargo test -p extension-runtime extension::manifest
```

Expected: all selected tests pass.

### Task 4: Update development documentation and ignore rules

**Files:**
- Modify: `CLAUDE.md`
- Modify: `.gitignore:55-57`

- [x] **Step 1: Update the product overview**

Describe Navop as the current product. Change the built-in provider display reference to Navop, but retain exact internal names such as `OnetCliApp`, `onetcli_app.rs`, and `onetcli_app::init(cx)` because those identifiers still exist.

- [x] **Step 2: Clarify legacy environment configuration**

Document `NAVOP_UPDATE_URL` as the current legacy environment variable accepted by the implementation, and state that the identifier is retained for compatibility rather than presenting it as the product name.

- [x] **Step 3: Remove the obsolete icon ignore rule**

Remove only:

```gitignore
resources/macos/OnetCli.icns
```

Do not add an ignore rule for the tracked `resources/macos/Navop.icns` asset.

- [x] **Step 4: Verify documentation branding boundaries**

Run:

```bash
rtk rg -n 'One Net Client|\*\*onetcli\*\*|Built-in AI chat \(OnetCli' CLAUDE.md
rtk rg -n 'resources/macos/OnetCli\.icns' .gitignore
```

Expected: no matches. Exact internal code identifiers may still appear in `CLAUDE.md`.

### Task 5: Final review and verification

**Files:**
- Review all files changed by Tasks 1-4.

- [x] **Step 1: Format Rust code**

Run:

```bash
rtk cargo fmt --all -- --check
```

Expected: exit code 0.

- [x] **Step 2: Run targeted test suites**

Run:

```bash
rtk cargo test -p onetcli_cli
rtk cargo test -p extension-runtime extension::manifest
```

Expected: all selected tests pass.

- [x] **Step 3: Check patch formatting**

Run:

```bash
rtk git diff --check
```

Expected: exit code 0.

- [x] **Step 4: Verify compatibility identifiers remain**

Run:

```bash
rtk rg -n 'com\.onetcli\.app|SYNC_PACKAGE_DIR: &str = "\.onetcli-sync"|APP_ID: &str = "onetcli"|ProviderType::OnetCli|pub onetcli: String|onetcli\.app_info|LEGACY_APP_BUNDLE_NAME' resources main crates
```

Expected: matches remain, demonstrating this batch did not remove compatibility-sensitive identifiers.

- [x] **Step 5: Verify pre-existing user files were not modified by this batch**

Review:

```bash
rtk git diff -- crates/core/src/license/models.rs main/src/home/home_tabs.rs main/src/home_tab.rs main/src/setting_tab.rs
```

Expected: only the user's pre-existing changes are present; this implementation adds no hunks to those files.

- [x] **Step 6: Review the final scoped diff**

Run:

```bash
rtk git diff -- .gitignore CLAUDE.md crates/onetcli_cli/src/lib.rs crates/onetcli_cli/src/tests.rs crates/extension-runtime/src/extension/manifest/versioning.rs docs/migration/onetcli-to-navop-audit.md docs/superpowers/specs/2026-07-12-navop-safe-rebrand-cleanup-design.md docs/superpowers/plans/2026-07-12-navop-safe-rebrand-cleanup.md
```

Expected: only the approved safe-rebrand batch is present. Do not commit unless the user explicitly requests it.
