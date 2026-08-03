# CI Requirements

CI runs on every push to `main` / `master` and every PR (Ubuntu latest, stable Rust toolchain).

## Steps & Commands

| Step | Command | Notes |
|:---|:---|:---|
| **Format** | `cargo fmt --all -- --check` | Code must be fmt-clean. Run `cargo fmt` locally before pushing. |
| **Clippy** | `cargo clippy --all-targets -- -D warnings` | All clippy warnings are treated as errors. Covers lib, examples, tests, and benches. |
| **Test** | `cargo test --verbose` | Includes unit tests **and doctests**. |

## Common Pitfalls (from experience)

### cargo fmt
- Run `cargo fmt` before every commit.
- CI uses `--check` mode — any formatting diff fails the build.

### cargo clippy --all-targets
The `--all-targets` flag means examples and tests are also linted, not just the library.
Common lints that bite:

| Lint | Example | Fix |
|:---|:---|:---|
| `collapsible_if` | Nested `if` / `if let` that can be merged | Use `&&` let-chain: `if cond && let Some(x) = y { … }` |
| `single_component_path_imports` | `use llm_providers;` (crate name only) | Delete it — use full path `llm_providers::fn()` directly |
| `empty_line_after_doc_comments` | `///` doc comment with blank line and no item | Remove or merge the stray comment |
| `empty_string_in_println` | `println!("")` | Use `println!()` |
| `unused_variables` | `let x = …` never used | Prefix with `_`: `let _x = …` |
| `dead_code` | Struct field never read | Prefix with `_`: `_field: Type` |

### cargo test --doc (doctests)
- All code examples in `///` doc comments are compiled and run.
- If you add a new field to a public struct, **update every doctest** that constructs that struct.
- Use `..Default::default()` or list the new field explicitly.

## Local Pre-Push Checklist

```bash
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --verbose
```

All three must pass with zero errors before pushing.
