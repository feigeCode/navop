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
2. Bump the application version **in all four manifests at once** (`crates/navop_app/Cargo.toml`, `apps/navop/Cargo.toml`, `apps/navop-shell/Cargo.toml`, `apps/navop-data/Cargo.toml`, plus `Cargo.lock`) to the target `X.Y.Z` **inside the same `dev` → `main` release pull request**. `main` is a protected branch (PR + `CI gate`, `enforce_admins=true`), so the version change cannot be pushed to `main` directly.
3. Merge the release pull request once CI passes; `main` now carries both the changelog entry and the bumped version.
4. On `main`, run `script/release-tag.sh vX.Y.Z`. The script refuses to proceed unless all four manifests already equal the tag and the bilingual changelog entry is present; it then creates and pushes the `v*` tag (tag pushes are not branch-protected).
5. `Release Trigger` dispatches the shared `Release` workflow on the `main` branch.
6. The workflow checks the unified application version (all four manifests) and the tagged changelog entry before starting expensive builds.
7. The full `navop` product builds its six platforms (macOS ARM64/x86_64, Linux x86_64/ARM64, Windows x86_64/x86); `navop-shell` and `navop-data` additionally build their four primary platforms (macOS ARM64/x86_64, Linux x86_64, Windows x64) — all in one matrix. The `products` input (`all` / `full` / `standalone`) trims the matrix when repairing a release.
8. After all requested platforms finish, the workflow extracts the tagged entry, uses it as the GitHub Release body, and writes the same Markdown to every R2 manifest's `release_notes` field (up to six manifests per channel run: one per product).
9. Once every uploaded object has been verified, R2 keeps only the tag that was just uploaded: the other `releases/<tag>/` prefixes are pruned, so the bucket carries one version's installers instead of the whole history. Nothing becomes unreachable — every `updates/**/latest.json` only ever points at the tag being uploaded, and older installers stay on their GitHub Release and on the CNB mirror.

The build workflow checks out the requested tag, while the workflow itself runs from `main`. This keeps Cargo input caches and sccache data reusable across tags and repair runs.

## Release-only pull requests

A pull request that only carries the `CHANGELOG.md` entry and the version bump does not need the platform matrix. `.github/workflows/ci.yml` classifies every pull request in the `classify` job through `script/release_pr.py check`:

- The pull request qualifies only when every changed file is `CHANGELOG.md`, `Cargo.lock`, or one of the four version manifests (`crates/navop_app/Cargo.toml`, `apps/navop/Cargo.toml`, `apps/navop-shell/Cargo.toml`, `apps/navop-data/Cargo.toml`), and the `Cargo.toml` / `Cargo.lock` diff contains nothing but `version = "..."` lines. Any other file, or a dependency change in any manifest, keeps the full matrix.
- When it qualifies, the `test` and `windows-rdp-probe` jobs are skipped and the release metadata is validated instead: all four manifests must carry the same version as the `navop_app` package in `Cargo.lock`, and the bilingual entry for that version must exist, be complete (`更新内容` / `修复与优化` plus `What's New` / `Fixes and Improvements`) and carry the CNB mirror line.
- `CI gate` stays the only required status check, and it still fails the pull request when the classification itself fails, so an invalid release entry cannot reach `main`.
- If the diff cannot be computed (for example a fork pull request whose head commit was not fetched), the classifier falls back to the full matrix.

A release pull request keeps the fast path after `main` is merged back into `dev`, because the classifier only looks at the diff against the base branch.

## Branch model

- `dev` is the beta development branch. Changes are pushed and validated here before a release.
- `main` is the stable release branch and is protected: changes land via pull request, and CI must pass before merging.
- Cut a release by merging the validated work from `dev` into `main` — including the four-manifest version bump (`crates/navop_app/Cargo.toml` + `apps/*/Cargo.toml`) and `Cargo.lock` — then creating the `v*` tag on `main` with `script/release-tag.sh`. The version bump must ride the pull request because direct pushes to `main` are rejected (GH006); only the tag is pushed directly.

## Unified product releases（统一 tag 三产品发布）

一个 tag 同时发布三个产品：全量 `navop`、独立 `navop-shell`、独立 `navop-data`。三个 app crate 的 `[package].version` 必须与 `navop_app` 完全一致（`env!("CARGO_PKG_VERSION")` 编译进各自二进制，feed 比较依赖它）。

- **矩阵**：`release.yml` 的 `Resolve platform matrix` 步骤为三个产品各生成条目，字段携带 `product`（cargo 包名）、`binary`、`archive`（`{binary}-{target}` 自更新资产名）、`bundle_name`（macOS .app 目录名）、`portable_marker`。全量 6 平台，独立产品各 4 平台（无 linux-arm64 / win32）。
- **`products` 输入**：`all`（默认）= 全量 + 两独立产品；`full` = 仅全量；`standalone` = 仅独立产品（选 linux-arm64/windows-x86 平台时会显式报错，因为独立产品不覆盖这两个平台）。
- **产物命名**：公开安装包统一 `{product}-{version}-{public_label}`（如 `navop-shell-0.23.0-macos-arm64.dmg`、`navop-data-0.23.0-windows-x64.msi`）；自更新器直连的 updater 资产为 `{binary}-{target}.tar.gz|.zip`。独立产品不做 deb/rpm/AppImage/GPU stack。
- **更新清单（6 份 = 3 产品 × 2 通道）**：全量 `updates/latest.json`（stable）/ `updates/beta/latest.json`（beta）；navop-shell `updates/navop-shell/shell/[beta/]latest.json`；navop-data `updates/navop-data/data/[beta/]latest.json`。键位与 `crates/app_kit/src/update/config.rs` 的 `feed_url()` 推导一致，schema 相同、共用同一份 release_notes。
- **自更新安装**：`install.rs` 按运行时 identity（`crate::identity()`）参数化 bundle/exe/binary 候选名，独立产品可自更新；全量保留 `OnetCli.app` / `onetcli.exe` legacy 兼容。

## Beta 发布通道

beta 通道把测试版与稳定版的更新清单彻底分开：稳定通道继续读写 `updates/latest.json`（老客户端唯一认识的地址），beta 只写 `updates/beta/latest.json`。beta tag 仍然是完整的 GitHub Release，只是会被显式标记为 prerelease。

> 首次使用前的前置条件：`release-trigger.yml` 用 `--ref main` 取 workflow 定义，所以 channel/prerelease 的改动必须已合入 `main`。否则跑的是 main 上的旧版 `release.yml`——不带 `--prerelease`、也不传 `channel`。

### 版本线约定

- beta 的 base 永远是**下一个稳定版**：`0.23.0-beta.1 → 0.23.0`。beta 稳定后就地转正，版本号去掉 `-beta.N`。
- 不要给已发布的稳定版开补丁 beta 线（例如 `0.22.1-beta`）：那会让老客户端把测试版当成同一版本线的修复更新。
- 同一 base 的 beta 依次递增：`0.23.0-beta.1`、`0.23.0-beta.2`……每个 beta 都是独立 tag、独立 Release、独立安装包前缀 `releases/<tag>/`。

### 一条命令发 beta

beta 版本线走 `dev` 分支，前置条件是工作区干净、`dev` 已同步、四个 manifest 版本已对齐。脚本会把 `crates/navop_app/Cargo.toml` 与三个 app crate 的版本统一改成 `X.Y.Z-beta.N`、同步 `Cargo.lock`、写入 `CHANGELOG.md` 双语条目、提交、打 tag，并推送 `dev` 与 tag：

```bash
script/beta-release.sh --base 0.23.0 --notes-file /private/tmp/navop-v0.23.0-beta.1.md
script/beta-release.sh --next --notes-file /private/tmp/navop-v0.23.0-beta.2.md
```

- `--base X.Y.Z`：把版本设为 `X.Y.Z-beta.1`。
- `--next`：要求当前版本是 `X.Y.Z-beta.N`，递增为 `beta.N+1`。
- `--notes-file`：必填，内容规则与正常发布相同（中文 `更新内容` / `修复与优化` + 英文 `What's New` / `Fixes and Improvements` + CNB 镜像行）；`release.yml` 会硬校验 `CHANGELOG.md` 里有 `## [v<tag>] - YYYY-MM-DD` 条目。
- `--dry-run`：只打印将要执行的步骤与最终 tag，不写任何文件、不提交、不打 tag、不推送。

### 发一个 beta 会发生什么

1. `git push cnb <tag>` 触发 `Release Trigger`。
2. `Release` 始终取 `main` 分支上的 workflow 定义运行，`prepare` job 按 tag 推导出 `channel=beta`。
3. 构建完成后创建/更新 GitHub Release，并追加 `--prerelease`。必须如此：否则 GitHub 的 `/releases/latest` 会把 beta 当成最新稳定版推给所有老客户端。
4. 派发 `upload-r2.yml` 时显式带 `-f channel=beta`：安装包照常上传到 `releases/<tag>/`，更新清单写到 `updates/beta/latest.json`。
5. 稳定清单 `updates/latest.json` 完全不动，所有走稳定通道的现有用户不受影响。

### 转正（promotion）

beta 验证稳定后，按正常发布流程把 `dev` 合进 `main`：在 release PR 里把四个 manifest 的版本从 `X.Y.Z-beta.N` 统一改成 `X.Y.Z`，整理 `CHANGELOG.md` 中该版本的条目，合并后在 `main` 上执行 `script/release-tag.sh vX.Y.Z`。这条稳定 tag 会把同一版本写回 `updates/latest.json` 与两个独立产品清单，beta 清单留在原地作为历史。

### 回滚

- beta 发布失败（tag 打错、changelog 缺条目）：`git tag -d vX.Y.Z-beta.N`、`git push cnb :refs/tags/vX.Y.Z-beta.N`，必要时 `git reset --hard HEAD~1`。beta 不影响稳定清单，通常无须额外回滚。
- 万一稳定清单被污染（例如误用 beta tag 跑了 stable 通道）：用一个已知正确的稳定 tag 重跑 `upload-r2.yml`（`-f tag=vX.Y.Z -f channel=stable`），脚本会幂等重建 `updates/latest.json` 与 `releases/<tag>/` 下的安装包。
- `node script/test-release-packaging.mjs` 里有一组回归断言（含假 aws 仿真），保证 beta tag 永远只写 `updates/beta/latest.json`。

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

The repair run rebuilds only the selected platform, overwrites its assets on the existing GitHub Release, regenerates the complete `sha256sums.txt`, synchronizes the GitHub Release body from the changelog stored in that tag, and triggers R2 synchronization. Assets from other platforms are preserved. The `products` input works the same way for product selection: `all` rebuilds the full product plus both standalone products on the selected platform, `full`/`standalone` trim the matrix to one side. Selecting `standalone` together with `linux-arm64` or `windows-x86` fails fast — standalone products do not cover those platforms.

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
- R2 keeps the newest stable release plus the beta installer that `updates/beta/latest.json` still points at, so beta-channel downloads keep working after a stable release. The prune step runs after the upload has been verified object by object and never carries `always()`, so a failed upload leaves the existing objects alone; it refuses to delete anything when its own tag is missing from R2, and a non-stable tag (for example `v0.20.0-rc.1`) skips pruning so it cannot take the last stable installers with it. Objects under `updates/` are never deleted.
- Legacy pre-changelog repairs preserve their existing GitHub Release body.
- All five primary platform builds belong to one matrix, so they start in parallel and a failed job can be rerun without rebuilding successful matrix jobs.
