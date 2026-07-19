# Navop Release Operations

## Normal release

1. Make sure CI passes on the release commit.
2. Create and push a `v*` tag.
3. `Release Trigger` dispatches the shared `Release` workflow on the `dev` branch.
4. The primary macOS, Linux x86_64, and Windows jobs build independently and publish the GitHub Release.
5. `Build ARM Linux Release` automatically dispatches the ARM Linux build after the primary Release succeeds.
6. Each successful Release run synchronizes the available updater archives and `latest.json` to R2.

The build workflow checks out the requested tag, while the workflow itself runs from `dev`. This keeps Cargo input caches and sccache data reusable across tags and repair runs.

## Repair one platform

Do not move the release tag. Open **Actions → Release → Run workflow**, enter the existing tag, and select only the failed platform:

| Selection | Target |
| --- | --- |
| `macos-arm64` | `aarch64-apple-darwin` |
| `macos-x64` | `x86_64-apple-darwin` |
| `linux-x64` | `x86_64-unknown-linux-gnu` |
| `linux-arm64` | `aarch64-unknown-linux-gnu` |
| `windows-x64` | `x86_64-pc-windows-msvc` |

The repair run rebuilds only the selected platform, overwrites its assets on the existing GitHub Release, regenerates the complete `sha256sums.txt`, and triggers R2 synchronization. Existing release notes and assets from other platforms are preserved.

For a failed matrix job in the same workflow run, prefer **Re-run failed jobs**. Successful platform jobs and their workflow artifacts remain available to the final publish job.

## Cache model

- Cargo registry and Git dependency inputs use a stable cache keyed by runner, target, and `Cargo.lock`.
- Rust compilation uses sccache with the GitHub Actions backend.
- Release jobs explicitly start sccache and keep it alive through long linking and LTO phases so the final statistics cover the complete build.
- Build caches are shared through workflow runs on the default `dev` branch instead of being isolated under each release tag.
- ARM Linux uses two Cargo build jobs, thin LTO, and 16 codegen units to reduce peak memory while retaining release optimization.

## Safety properties

- Release operations are serialized per tag and are never auto-cancelled.
- A single-platform repair requires the GitHub Release to already exist.
- Publishing uses `--clobber` only for newly built platform files and `sha256sums.txt`.
- Existing hand-written release notes are not overwritten during a repair.
- ARM Linux is a separate follow-up run, so a slow ARM runner does not force successful desktop platforms to rebuild.
