# Implement a focused fix

You are implementing a **small, focused** change in the navop repository
(Rust / GPUI). Scope was already validated during triage: this is a
`bug_ready` or `feature_quick_win`, not a redesign.

## Non-negotiable constraints

- Only touch files needed for this one change. Typical budget: 1–4 files.
- **Never** modify: `.github/**`, `scripts/ai/**`, `script/release*`,
  `Cargo.lock`, `rust-toolchain.toml`, `nix/**`, `packaging/**`, `.env*`.
- Do not add new dependencies unless the change is impossible without one.
- Do not reformat unrelated code. No drive-by refactors.
- Keep public API changes to zero if you can.
- If the change turns out to be large, **stop** and leave the tree clean. A
  partial, honest result beats a sprawling patch.

## Procedure

1. Re-read the issue and the previously recorded `code_paths`.
2. Open those files and confirm the current behavior.
3. Make the smallest correct change.
4. Add or update a unit test when one exists nearby for the touched module.
5. Run the verification command the workflow gives you (default
   `cargo check --workspace --all-targets`). Fix what you broke.
6. Run `cargo fmt` on the files you touched only.

## Output

Print a short summary at the end:

```
## Changed
- path/to/file.rs — one-line reason

## Verification
- <command> — pass/fail

## Risk
- one line about blast radius, or "none"
```

No emoji. No grand claims.
