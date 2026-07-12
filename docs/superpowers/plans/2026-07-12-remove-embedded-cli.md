# Remove Embedded CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove Navop's in-process business CLI and all CLI-only runtime adapter concepts while preserving tool registries and Public MCP automation.

**Architecture:** The `navop` binary becomes GUI/update-only. `onetcli_runtime` remains the shared tool-registry layer consumed by Public MCP and Agent paths; a future standalone `navop-cli` will connect through Public MCP instead of linking application state.

**Tech Stack:** Rust 2024, Cargo workspace, tool_runtime, Public MCP, structural contract tests.

---

## Execution Status

Completed on 2026-07-12 in the current `dev` workspace without creating a commit. Both structural contracts failed before deletion and passed afterward. The embedded CLI crate, CLI host, GUI command split, CLI bridge, and CLI-only tool runtime variants were removed. Redis behavior tests now call the registry directly. Final targeted verification passed for `tool_runtime`, `onetcli_runtime`, `public_mcp`, and `main` compilation.

### Task 1: Add structural removal contracts

**Files:**
- Modify: `main/src/main.rs`
- Modify: `main/src/public_mcp_runtime.rs`
- Modify: `crates/onetcli_runtime/src/lib.rs`

- [x] Add a `main.rs` test that loads `include_str!("main.rs")`, asserts `handle_cli_command` is absent, and asserts `update::handle_update_command()` remains.
- [x] Add an `onetcli_runtime` test that asserts `lib.rs` does not export `cli_host` and `crates/onetcli_runtime/Cargo.toml` does not contain `onetcli_cli`.
- [x] Run `rtk cargo test -p main main_does_not_route_business_cli` and `rtk cargo test -p onetcli_runtime runtime_has_no_embedded_command_module`.
- [x] Confirm both tests fail because the embedded CLI is still present.

### Task 2: Remove the GUI command-line split and CLI crate

**Files:**
- Modify: `main/src/main.rs`
- Modify: `Cargo.toml`
- Delete: `crates/onetcli_cli/Cargo.toml`
- Delete: `crates/onetcli_cli/src/lib.rs`
- Delete: `crates/onetcli_cli/src/tests.rs`

- [x] Remove the pre-GUI `handle_cli_command()` call from `main()`.
- [x] Remove both platform-specific `handle_cli_command()` functions.
- [x] Remove the now-unused `public_mcp_runtime::cli_tool_registry()` bridge that existed only for the embedded CLI.
- [x] Keep `update::handle_update_command()` unchanged.
- [x] Remove `crates/onetcli_cli` from workspace members and remove the `onetcli_cli` workspace dependency.
- [x] Remove the now-unused direct `clap` workspace dependency after verifying no remaining Cargo manifest uses it.
- [x] Delete the CLI crate files.
- [x] Run the `main_does_not_route_business_cli` test and confirm it passes.

### Task 3: Remove `cli_host` from the runtime

**Files:**
- Modify: `crates/onetcli_runtime/Cargo.toml`
- Modify: `crates/onetcli_runtime/src/lib.rs`
- Delete: `crates/onetcli_runtime/src/cli_host.rs`
- Delete: `crates/onetcli_runtime/src/cli_host/domain.rs`
- Delete: `crates/onetcli_runtime/src/cli_host/tests.rs`

- [x] Remove the `onetcli_cli` dependency from `onetcli_runtime`.
- [x] Remove `pub mod cli_host` from the runtime library.
- [x] Delete the CLI host implementation and its CLI-specific tests.
- [x] Run `rtk cargo test -p onetcli_runtime runtime_has_no_embedded_command_module` and confirm it passes.

### Task 4: Remove CLI-only tool runtime variants

**Files:**
- Modify: `crates/tool_runtime/src/descriptor.rs`
- Modify: `crates/tool_runtime/src/invocation.rs`
- Modify: `crates/tool_runtime/src/resource.rs`
- Modify: `crates/tool_runtime/tests/registry.rs`
- Modify: `crates/onetcli_runtime/src/lib.rs`
- Modify: `crates/onetcli_runtime/src/database_tools.rs`
- Modify: `crates/onetcli_runtime/src/redis_tools.rs`
- Modify: `crates/onetcli_runtime/src/workspaces.rs`
- Modify: `crates/onetcli_runtime/src/connections.rs`
- Modify: `crates/onetcli_runtime/src/connections/tests.rs`
- Modify: `crates/onetcli_runtime/src/sftp_tools.rs`

- [x] Remove `Cli` from `ToolAdapter`, `ToolOrigin`, `ToolCaller`, and `ResourceOrigin`.
- [x] Remove `ToolAdapter::Cli` from every descriptor adapter list.
- [x] Remove the CLI arm from `connections::adapter_name`.
- [x] Change connection tests that used `ToolAdapter::Cli` to `FunctionCalling`; rename the CLI exposure test to describe automation exposure.
- [x] Update the tool registry adapter-filter test to compare MCP with `FunctionCalling` instead of the removed CLI adapter.
- [x] Run `rtk cargo test -p tool_runtime` and the connection tests in `onetcli_runtime`.

### Task 5: Preserve Redis tool behavior without CLI wrappers

**Files:**
- Modify: `crates/onetcli_runtime/tests/redis_tools.rs`

- [x] Remove imports of `onetcli_cli` and `cli_host::run_tool_command`.
- [x] Rename CLI-oriented descriptor test names/messages to automation/tool-registry wording.
- [x] Delete tests whose only contract is CLI `allow_write`; that permission layer is intentionally removed with `cli_host`.
- [x] Convert the read-tool, unknown-alias, and wrong-connection tests to direct `ToolRegistry::call()` calls using `ToolContext::for_adapter(ToolAdapter::FunctionCalling)`.
- [x] Assert direct `ToolError::UnknownTool` or `ToolError::Failed` contracts rather than CLI JSON error strings.
- [x] Run `rtk cargo test -p onetcli_runtime --test redis_tools` and confirm all retained behavior tests pass.

### Task 6: Update migration documentation

**Files:**
- Modify: `docs/migration/onetcli-to-navop-audit.md`
- Modify: `CLAUDE.md`

- [x] Record that the embedded CLI was removed and a future standalone `navop-cli` should use Public MCP discovery.
- [x] Remove any development guidance implying the `navop` GUI binary accepts business CLI subcommands.
- [x] Keep historical compatibility identifiers documented.

### Task 7: Refresh the lockfile and verify the workspace

**Files:**
- Modify: `Cargo.lock`

- [x] Run `rtk cargo metadata --no-deps` to refresh workspace package metadata and `Cargo.lock` after crate removal.
- [x] Run `rtk cargo fmt --all -- --check`.
- [x] Run `rtk cargo test -p tool_runtime`.
- [x] Run `rtk cargo test -p onetcli_runtime`.
- [x] Run `rtk cargo test -p public_mcp`.
- [x] Run `rtk cargo check -p main`.
- [x] Run `rtk rg -n 'onetcli_cli|cli_host|ToolAdapter::Cli|ToolOrigin::Cli|ToolCaller::Cli|ResourceOrigin::Cli' Cargo.toml Cargo.lock main crates` and expect no matches.
- [x] Run `rtk rg -n 'update::handle_update_command|onetcli_runtime::.*tool_registry|PublicMcpRuntime|public_mcp_discovery_path' main crates` and confirm update and Public MCP paths remain.
- [x] Run `rtk git diff --check` and review the final scoped diff.
- [x] Confirm `crates/core/src/cloud_sync/team_key_manager.rs` and `main/src/setting_tab.rs` contain only user changes and no task-generated hunks.
- [x] Do not commit, push, or create a PR unless the user explicitly requests it.
