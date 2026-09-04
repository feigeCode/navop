# Navop Release Operations

## Normal release

`CHANGELOG.md` is the single source of truth for user-facing release notes. Generate the bilingual entry before creating a release tag:

```bash
python3 script/changelog.py upsert \
  --tag v0.10.1 \
  --date 2026-08-01 \
  --notes-file /private/tmp/navop-v0.10.1-release-notes.md \
  --changelog CHANGELOG.md

python3 script/changelog.py extract \
  --tag v0.10.1 \
  --changelog CHANGELOG.md \
  --output /private/tmp/navop-v0.10.1-release-notes-from-changelog.md

git diff -- CHANGELOG.md
```

Review the extracted Markdown and make sure it matches the source notes, including both the Chinese `更新内容` / `修复与优化` and English `What's New` / `Fixes and Improvements` content sections and the CNB mirror download line. Commit the changelog entry before creating or pushing the tag.

The normal release sequence is:

1. Generate, review, and commit the target version entry in `CHANGELOG.md`.
2. Make sure CI passes on the release commit.
3. Create and push the matching `v*` tag on the `main` branch. Both `script/release-tag.sh` and `script/bump-version.sh` reject a tag without a valid bilingual changelog entry.
4. `Release Trigger` dispatches the shared `Release` workflow on the `main` branch.
5. The workflow checks the application version and tagged changelog entry before starting expensive builds.
6. macOS ARM64, macOS x86_64, Linux x86_64, Linux ARM64, and Windows x86_64 build in parallel in one matrix.
7. After all requested platforms finish, the workflow extracts the tagged entry, uses it as the GitHub Release body, and writes the same Markdown to the R2 `latest.json` `release_notes` field.

The build workflow checks out the requested tag, while the workflow itself runs from `main`. This keeps Cargo input caches and sccache data reusable across tags and repair runs.

## Branch model

- `dev` is the beta development branch. Changes are pushed and validated here before a release.
- `main` is the stable release branch and is protected: changes land via pull request, and CI must pass before merging.
- Cut a release by merging the validated work from `dev` into `main`, then creating the `v*` tag on `main`.

## Changelog format

The newest entry belongs immediately after the `<!-- NAVOP_RELEASES -->` marker:

```markdown
## [v0.10.1] - 2026-08-01

#### 更新内容

- ...

#### 修复与优化

- ...

国内下载：如果 GitHub 下载较慢，可从 [CNB 镜像](https://cnb.cool/navop-dev/navop/-/releases/tag/v0.10.1) 下载桌面端安装包

---

#### What's New

- ...

#### Fixes and Improvements

- ...

**Full Changelog**: https://github.com/feigeCode/navop/compare/v0.10.0...v0.10.1
```

Do not edit generated GitHub or R2 release notes independently for a normal release. Update `CHANGELOG.md`, create a new tag when appropriate, and let the workflows extract the entry. The extraction tool demotes the entry's headings by one level; entries no longer carry the `## 中文` / `## English` language headings, and every new entry must include the CNB mirror download line so the GitHub Release body and R2 `release_notes` show it automatically.

## Repair one platform

Do not move the release tag. Open **Actions → Release → Run workflow**, enter the existing tag, and select only the failed platform:

| Selection | Target |
| --- | --- |
| `macos-arm64` | `aarch64-apple-darwin` |
| `macos-x64` | `x86_64-apple-darwin` |
| `linux-x64` | `x86_64-unknown-linux-gnu` |
| `linux-arm64` | `aarch64-unknown-linux-gnu` |
| `windows-x64` | `x86_64-pc-windows-msvc` |

The repair run rebuilds only the selected platform, overwrites its assets on the existing GitHub Release, regenerates the complete `sha256sums.txt`, synchronizes the GitHub Release body from the changelog stored in that tag, and triggers R2 synchronization. Assets from other platforms are preserved.

Tags created before `CHANGELOG.md` was introduced are treated as legacy repairs: if the GitHub Release already exists, its current body is preserved and reused for R2. A new Release cannot be created from a tag that has neither a tracked changelog entry nor an existing legacy Release body.

For a failed matrix job in the same workflow run, prefer **Re-run failed jobs**. Successful platform jobs and their workflow artifacts remain available to the final publish job.

## Cache model

- CI, Release, and ARM Linux use the same Cargo registry and Git dependency cache namespace, keyed only by runner OS and `Cargo.lock`. Linux x86_64 can therefore seed Linux ARM64 inputs, and macOS ARM64 can seed macOS x86_64 inputs.
- Rust compilation uses sccache with the GitHub Actions backend in every Rust build workflow. All five release platforms run from the same default `main` workflow scope and reuse compiler objects from earlier runs for the same target and profile.
- The implicit `Swatinem/rust-cache` inside `actions-rust-lang/setup-rust-toolchain` is disabled, and `target/` is not stored by `actions/cache`. This avoids duplicating multi-gigabyte target archives that would evict useful sccache objects from GitHub's repository cache quota.
- Release jobs explicitly start sccache and keep it alive through long linking and LTO phases so the final statistics cover the complete build.
- Build caches are shared through workflow runs on the default `main` branch instead of being isolated under each release tag.
- ARM Linux uses two Cargo build jobs, thin LTO, and 16 codegen units to reduce peak memory while retaining release optimization.

## Safety properties

- Release operations are serialized per tag and are never auto-cancelled.
- A single-platform repair requires the GitHub Release to already exist.
- A new release tag must contain a valid bilingual `CHANGELOG.md` entry before builds begin.
- GitHub Release notes and R2 updater `release_notes` are extracted from the same tagged changelog entry.
- Publishing uses `--clobber` only for newly built platform files and `sha256sums.txt`.
- Legacy pre-changelog repairs preserve their existing GitHub Release body.
- All five primary platform builds belong to one matrix, so they start in parallel and a failed job can be rerun without rebuilding successful matrix jobs.
