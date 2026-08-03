# AGENTS.md

This file defines repository-wide instructions for agents working in `llm-connector`.

## Project Boundary

`llm-connector` is a Rust protocol adapter library for LLM providers.

Its responsibility is to normalize request and response shapes across providers while preserving provider-native behavior where that is required for correctness.

In scope:

- Unified public request and response types such as `ChatRequest`, `ChatResponse`, streaming types, embeddings, and Responses API types.
- Provider adapters, protocol-family reuse, capability checks, request mapping, response normalization, SSE parsing, auth/header policies, and endpoint construction.
- Provider-native extensions when they belong at the protocol boundary, such as Anthropic-native tool payloads or provider-specific reasoning fields.
- Test coverage for mapping, parsing, streaming, and provider compatibility behavior.
- User-facing package documentation in `README.md` and `www/`, plus internal design and verification docs in `docs/`.

Out of scope:

- Running a gateway, router, agent runtime, MCP host, or application server.
- Executing tools, enforcing business workflows, storing state, or orchestrating multi-step product logic.
- Hiding provider routing decisions behind hardcoded endpoints or implicit deployment assumptions.
- UI features, dashboards, databases, and operational control-plane behavior.
- One-off provider hacks that bypass shared abstractions when the same behavior belongs in common protocol-family code.

When adding functionality, keep the abstraction boundary tight: `llm-connector` should translate protocol differences, not become a general AI platform.

## Documentation Rules

To maintain a clean and navigable documentation structure:

- Keep internal technical and verification documents consolidated in `docs/`.
  - Existing internal docs in this repository include files such as `docs/PARAMETER_MAPPING.md`, `docs/PROTOCOL_DESIGN_V2.md`, `docs/ci_requirements.md`, and `docs/model_testing_results.md`.
- Keep user-facing package and site documentation in `README.md` and `www/`.
- Do not create scattered generic `*_REPORT.md`, `*_SUMMARY.md`, or ad hoc root-level documentation files for work that belongs in the existing docs.
- Do not create `docs/guides/` or `docs/archive/` unless the repository structure is intentionally changed first.

## Release Rules

- All release notes must go into `CHANGELOG.md`.
- Do not create `RELEASE_NOTES_*.md` files in the repository root.
  - Exception: temporary files used only for a release command are allowed if they are deleted immediately afterward.
- Git tags must match the version in `Cargo.toml`.

## Coding Standards

- Use English only for comments, documentation, commit messages, and checked-in text.
- The code should compile with zero warnings.
- Remove unused imports and dead code immediately.
- Do not leave commented-out code behind.
- Prefer shared protocol-family logic first, and keep provider-specific code limited to true protocol differences.

## Scripts

- Keep `scripts/` minimal and maintenance-focused.
- Do not check in one-off debugging scripts such as `fix_*` or `test_*` helpers that are only useful for a single investigation.
