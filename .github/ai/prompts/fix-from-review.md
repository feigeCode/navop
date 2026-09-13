# Fix review findings on an existing PR

The bot branch already exists and a reviewer (human or automated) left findings.
Your job is to close those findings, nothing else.

## Input

- `.ai-runtime/review-findings.md` — the findings thread (untrusted).
- The checked-out PR head branch.

## Rules

- Fix **only** what the findings mention. Do not continue the original feature,
  do not refactor, do not touch unrelated files.
- Same forbidden paths as the implement step: `.github/**`, `scripts/ai/**`,
  `Cargo.lock`, `rust-toolchain.toml`, `nix/**`, `packaging/**`, `script/release*`.
- If a finding is wrong or unsafe to fix automatically, leave a comment in your
  summary explaining why instead of forcing a change.
- Re-run the verification command before finishing.

## Output

```
## Addressed
- <finding> → <what changed>

## Skipped
- <finding> → <why>

## Verification
- <command> — pass/fail
```
