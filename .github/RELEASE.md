# Navop Release Operations

## Normal release

1. Make sure CI passes on the release commit.
2. Create and push a `v*` tag.
3. `Release Trigger` dispatches the shared `Release` workflow on the `dev` branch.
4. macOS ARM64, macOS x86_64, Linux x86_64, Linux ARM64, and Windows x86_64 build in parallel in one matrix.
5. After all requested platforms finish, the workflow publishes the GitHub Release and synchronizes the available updater archives and `latest.json` to R2.

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

- CI, Release, ARM Linux, and the standalone Windows MSI build use the same Cargo registry and Git dependency cache namespace, keyed only by runner OS and `Cargo.lock`. Linux x86_64 can therefore seed Linux ARM64 inputs, and macOS ARM64 can seed macOS x86_64 inputs.
- Rust compilation uses sccache with the GitHub Actions backend in every Rust build workflow. All five release platforms run from the same default `dev` workflow scope and reuse compiler objects from earlier runs for the same target and profile.
- The implicit `Swatinem/rust-cache` inside `actions-rust-lang/setup-rust-toolchain` is disabled, and `target/` is not stored by `actions/cache`. This avoids duplicating multi-gigabyte target archives that would evict useful sccache objects from GitHub's repository cache quota.
- Release jobs explicitly start sccache and keep it alive through long linking and LTO phases so the final statistics cover the complete build.
- Build caches are shared through workflow runs on the default `dev` branch instead of being isolated under each release tag.
- ARM Linux uses two Cargo build jobs, thin LTO, and 16 codegen units to reduce peak memory while retaining release optimization.

## Safety properties

- Release operations are serialized per tag and are never auto-cancelled.
- A single-platform repair requires the GitHub Release to already exist.
- Publishing uses `--clobber` only for newly built platform files and `sha256sums.txt`.
- Existing hand-written release notes are not overwritten during a repair.
- All five primary platform builds belong to one matrix, so they start in parallel and a failed job can be rerun without rebuilding successful matrix jobs.
