# Extension Repository Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move extension package production, DuckDB driver builds, and extension marketplace manifest publishing into a separate `feigeCode/onetcli-extensions` repository while keeping `onetcli` responsible for the main app, update client, extension runtime, and marketplace consumption.

**Architecture:** `onetcli` remains the host application and owns the runtime contracts: update checking, extension installation, marketplace UI, R2-first downloads, and GitHub fallback behavior. `onetcli-extensions` is a first-party extension monorepo: it owns concrete IPC/Wasm/language extension source code, per-extension package builds, manifest entry generation, GitHub Release publication, and R2 upload for changed extension assets. The first split keeps `extension-protocol`, `extension-driver`, and related SDK crates in `onetcli`; the new repository depends on them by Git tag to avoid prematurely creating a third SDK repository.

**Tech Stack:** Rust workspace, Cargo, GitHub Actions, Cloudflare R2 via AWS CLI, Node.js for JSON manifest generation, Bash packaging scripts, GitHub Releases.

---

## Current Boundary

Keep these responsibilities in `onetcli`:

- `crates/extension-runtime`: runtime registry, extension installation, marketplace manifest fetch, asset download, permission review.
- `crates/extension_view`: extension marketplace UI.
- `crates/extension-protocol`, `crates/extension-driver`, `crates/extension-host`, `crates/extension-component`, `crates/extension-wasm`: host and SDK/protocol crates used by the app and extension authors.
- `.github/workflows/release.yml`: main app build and GitHub Release.
- `.github/workflows/upload-r2.yml`: main app asset upload and `updates/latest.json`.

Move these responsibilities to `onetcli-extensions`:

- `crates/duckdb_driver`: DuckDB IPC driver implementation, tests, `driver.json`, locales.
- Future first-party IPC drivers, Wasm composite extensions, and language extensions.
- Extension package build scripts.
- Extension package Release workflow.
- Extension R2 upload workflow.
- `extension-manifest.json` generation and publication.

---

## Multi-Extension Build Strategy

`onetcli-extensions` can keep all official extensions in one repository without
building every extension on every change. The repository must make every
extension an independent release unit:

```text
onetcli-extensions/
  extensions/
    ipc/
      duckdb/
        extension.build.json
        driver.json
        locales/
        src/
      postgres/
        extension.build.json
    wasm/
      sql-formatter/
        extension.build.json
        extension.json
        wasm/
    language/
      rust/
        extension.build.json
  manifest/
    entries/
      duckdb.json
      postgres.json
      sql-formatter.json
  scripts/
    changed-extensions.mjs
    generate-marketplace-manifest.mjs
    package-extension.mjs
```

Each extension gets an `extension.build.json` file. IPC drivers normally build
one package per target triple; pure Wasm and language extensions usually build
one universal package.

Example IPC driver metadata:

```json
{
  "id": "duckdb",
  "kind": "database_driver",
  "package": "duckdb_driver",
  "binary": "duckdb_driver",
  "path": "extensions/ipc/duckdb",
  "targets": [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc"
  ],
  "releaseTagPrefix": "duckdb-v",
  "r2Prefix": "extensions/duckdb"
}
```

Example Wasm extension metadata:

```json
{
  "id": "sql-formatter",
  "kind": "composite",
  "path": "extensions/wasm/sql-formatter",
  "targets": ["universal"],
  "releaseTagPrefix": "sql-formatter-v",
  "r2Prefix": "extensions/sql-formatter"
}
```

CI and Release selection rules:

- Pull requests build only extensions whose source directories changed.
- Changes under shared scripts, shared crates, or workflows build all extensions at first; this can later be refined with dependency mapping.
- Release tags are extension-scoped, for example `duckdb-v1.0.0` or `sql-formatter-v0.3.0`, so a DuckDB release never builds unrelated extensions.
- R2 upload uploads only the current extension's package files plus a regenerated global `extensions/manifest.json`.
- `extension-manifest.json` is always generated as a full manifest, but it is built by merging `manifest/entries/*.json`; unchanged extensions reuse their existing entry files and do not rebuild.
- Manifest `asset_url`, `asset_urls`, `fallback_asset_url`, and `fallback_asset_urls` values may be relative paths. When a value does not start with `http://` or `https://`, the `onetcli` client resolves it against the directory containing the manifest URL. Because the R2 manifest is published at `/extensions/manifest.json`, primary R2 package paths should look like `duckdb/1.0.0/duckdb-driver-x86_64-unknown-linux-gnu.tar.gz`, not a hard-coded public domain.

Submodules are optional later. If official extensions move to separate
repositories, `onetcli-extensions` can become a registry repository whose
`extensions/<id>` directories are submodules. The same changed-path detection
still applies: when a submodule pointer changes, only that extension is built.
Do not make the `onetcli` main app release depend on extension submodules.

---

## Task 1: Create The `onetcli-extensions` Workspace

**Files:**
- Create in new repository: `Cargo.toml`
- Create in new repository: `.gitignore`
- Create in new repository: `extensions/ipc/duckdb/extension.build.json`
- Copy from `onetcli`: `crates/duckdb_driver/**` into `extensions/ipc/duckdb/**`

- [ ] **Step 1: Create the new repository locally**

Run:

```bash
mkdir -p ../onetcli-extensions
cd ../onetcli-extensions
git init
mkdir -p extensions/ipc
cp -R ../onetcli/crates/duckdb_driver extensions/ipc/duckdb
```

Expected: `extensions/ipc/duckdb/Cargo.toml`, `extensions/ipc/duckdb/driver.json`, and `extensions/ipc/duckdb/src/main.rs` exist in the new repository.

- [ ] **Step 2: Create workspace manifest**

Create `Cargo.toml`:

```toml
[workspace]
members = [
    "extensions/ipc/duckdb",
]
resolver = "2"

[workspace.package]
publish = false
edition = "2024"

[workspace.dependencies]
anyhow = "1.0.102"
base64 = "0.22"
chrono = { version = "0.4.23", features = ["serde"] }
duckdb = { version = "1.10501.0", features = ["bundled"] }
extension-driver = { git = "https://github.com/feigeCode/onetcli", tag = "v0.4.8", package = "extension-driver" }
extension-host = { git = "https://github.com/feigeCode/onetcli", tag = "v0.4.8", package = "extension-host" }
extension-protocol = { git = "https://github.com/feigeCode/onetcli", tag = "v0.4.8", package = "extension-protocol" }
hex = "0.4"
interprocess = { version = "2.4.0", features = ["tokio"] }
serde = { version = "1.0.219", features = ["derive"] }
serde_json = "1"
tempfile = "3"
tokio = { version = "1", features = ["io-util", "rt-multi-thread", "macros", "sync"] }
tracing = "0.1.41"
tracing-subscriber = "0.3"
uuid = { version = "1.23.0", features = ["v4", "serde"] }
```

- [ ] **Step 3: Keep `duckdb_driver` dependencies on workspace aliases**

Verify `extensions/ipc/duckdb/Cargo.toml` still uses `.workspace = true` dependencies. It should compile against the new workspace manifest above without path dependencies to the main app repository.

- [ ] **Step 4: Add DuckDB extension build metadata**

Create `extensions/ipc/duckdb/extension.build.json`:

```json
{
  "id": "duckdb",
  "kind": "database_driver",
  "package": "duckdb_driver",
  "binary": "duckdb_driver",
  "path": "extensions/ipc/duckdb",
  "targets": [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc"
  ],
  "releaseTagPrefix": "duckdb-v",
  "r2Prefix": "extensions/duckdb"
}
```

- [ ] **Step 5: Create `.gitignore`**

Create `.gitignore`:

```gitignore
/target/
/.idea/
.DS_Store
*.log
```

- [ ] **Step 6: Run initial build**

Run:

```bash
cargo test -p duckdb_driver -- --nocapture
```

Expected: DuckDB driver tests compile and pass. If this fails because an SDK type changed in `onetcli`, update the Git tag to the first main-app tag containing the committed R2/extension-runtime changes.

---

## Task 2: Add Extension Package Scripts

**Files:**
- Create in new repository: `scripts/package-driver.sh`
- Create in new repository: `scripts/verify-package.sh`
- Create in new repository: `scripts/changed-extensions.mjs`
- Create in new repository: `scripts/generate-marketplace-manifest.mjs`

- [ ] **Step 1: Create packaging script**

Create `scripts/package-driver.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 4 ]; then
  echo "Usage: $0 <extension-id> <target-triple> <artifact-dir> <version>" >&2
  exit 2
fi

EXTENSION_ID="$1"
TARGET="$2"
ARTIFACT_DIR="$3"
VERSION="$4"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"

SOURCE_DIR="${REPO_DIR}/extensions/ipc/${EXTENSION_ID}"
BUILD_METADATA="${SOURCE_DIR}/extension.build.json"
if [ ! -f "$BUILD_METADATA" ]; then
  echo "Missing extension build metadata: ${BUILD_METADATA}" >&2
  exit 1
fi

BIN_STEM="$(node -e 'const fs = require("fs"); const data = JSON.parse(fs.readFileSync(process.argv[1], "utf8")); process.stdout.write(data.binary || `${data.id}_driver`);' "$BUILD_METADATA")"
BIN_NAME="$BIN_STEM"
if [[ "$TARGET" == *windows* ]]; then
  BIN_NAME="${BIN_STEM}.exe"
fi

SOURCE_BIN="${REPO_DIR}/target/${TARGET}/release/${BIN_NAME}"
PACKAGE_ROOT="${REPO_DIR}/target/extension-packages/${TARGET}"
DRIVER_DIR="${PACKAGE_ROOT}/${EXTENSION_ID}"
ARCHIVE_NAME="${EXTENSION_ID}-driver-${TARGET}.tar.gz"

if [ ! -f "$SOURCE_BIN" ]; then
  echo "Missing driver binary: ${SOURCE_BIN}" >&2
  echo "Run: cargo build --release -p ${BIN_STEM} --target ${TARGET}" >&2
  exit 1
fi

rm -rf "$DRIVER_DIR"
mkdir -p "$DRIVER_DIR" "$ARTIFACT_DIR"
cp "$SOURCE_BIN" "${DRIVER_DIR}/${BIN_NAME}"
cp -R "${SOURCE_DIR}/locales" "${DRIVER_DIR}/locales"

DRIVER_JSON_SOURCE="${SOURCE_DIR}/driver.json"
DRIVER_JSON_TARGET="${DRIVER_DIR}/driver.json"
DRIVER_JSON_SOURCE="$DRIVER_JSON_SOURCE" \
DRIVER_JSON_TARGET="$DRIVER_JSON_TARGET" \
VERSION="$VERSION" \
BIN_NAME="$BIN_NAME" \
node <<'NODE'
const fs = require("fs");
const source = process.env.DRIVER_JSON_SOURCE;
const target = process.env.DRIVER_JSON_TARGET;
const version = process.env.VERSION;
const binName = process.env.BIN_NAME;
const manifest = JSON.parse(fs.readFileSync(source, "utf8"));
manifest.version = version;
manifest.entry = manifest.entry || {};
manifest.entry.command = `./${binName}`;
fs.writeFileSync(target, `${JSON.stringify(manifest, null, 2)}\n`);
NODE

if [[ "$TARGET" != *windows* ]]; then
  chmod +x "${DRIVER_DIR}/${BIN_NAME}"
fi

tar czf "${ARTIFACT_DIR}/${ARCHIVE_NAME}" -C "$PACKAGE_ROOT" "$EXTENSION_ID"
echo "${ARTIFACT_DIR}/${ARCHIVE_NAME}"
```

- [ ] **Step 2: Make the packaging script executable**

Run:

```bash
chmod +x scripts/package-driver.sh
```

Expected: the script is executable and patches `driver.json.entry.command` to `./duckdb_driver` or `./duckdb_driver.exe`.

- [ ] **Step 3: Create package verifier**

Create `scripts/verify-package.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "Usage: $0 <extension-package.tar.gz>" >&2
  exit 2
fi

ARCHIVE="$1"
TMP_DIR="$(mktemp -d)"
cleanup() {
  rm -rf "$TMP_DIR"
}
trap cleanup EXIT

tar xzf "$ARCHIVE" -C "$TMP_DIR"

DRIVER_DIR="${TMP_DIR}/duckdb"
DRIVER_JSON="${DRIVER_DIR}/driver.json"
if [ ! -f "$DRIVER_JSON" ]; then
  echo "Missing driver.json" >&2
  exit 1
fi

COMMAND="$(node -e 'const fs = require("fs"); const p = process.argv[1]; const data = JSON.parse(fs.readFileSync(p, "utf8")); process.stdout.write(data.entry && data.entry.command || "");' "$DRIVER_JSON")"
if [ -z "$COMMAND" ]; then
  echo "driver.json entry.command is empty" >&2
  exit 1
fi

BIN_PATH="${DRIVER_DIR}/${COMMAND#./}"
if [ ! -f "$BIN_PATH" ]; then
  echo "driver binary referenced by entry.command does not exist: ${COMMAND}" >&2
  exit 1
fi

if [ ! -d "${DRIVER_DIR}/locales" ]; then
  echo "Missing locales directory" >&2
  exit 1
fi

echo "Package verification ok: ${ARCHIVE}"
```

Run:

```bash
chmod +x scripts/verify-package.sh
```

- [ ] **Step 4: Create changed-extension detector**

Create `scripts/changed-extensions.mjs`:

```javascript
import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";

const [baseSha, headSha] = process.argv.slice(2);
if (!baseSha || !headSha) {
  throw new Error("Usage: node scripts/changed-extensions.mjs <base-sha> <head-sha>");
}

const extensions = loadExtensions();
const changedFiles = execFileSync(
  "git",
  ["diff", "--name-only", baseSha, headSha],
  { encoding: "utf8" },
)
  .trim()
  .split(/\n/)
  .filter(Boolean);

const sharedChange = changedFiles.some((file) =>
  file.startsWith("scripts/") ||
  file.startsWith("crates/") ||
  file.startsWith(".github/workflows/"),
);

const changedIds = new Set();
if (sharedChange) {
  for (const extension of extensions) changedIds.add(extension.id);
} else {
  for (const file of changedFiles) {
    for (const extension of extensions) {
      if (file === extension.path || file.startsWith(`${extension.path}/`)) {
        changedIds.add(extension.id);
      }
    }
  }
}

const include = [];
for (const extension of extensions) {
  if (!changedIds.has(extension.id)) continue;
  for (const target of extension.targets) {
    include.push({
      extension: extension.id,
      package: extension.package || "",
      kind: extension.kind,
      target,
      os: runnerForTarget(target),
    });
  }
}

process.stdout.write(`${JSON.stringify({ include })}\n`);

function loadExtensions() {
  const roots = ["extensions/ipc", "extensions/wasm", "extensions/language"];
  const result = [];
  for (const root of roots) {
    if (!fs.existsSync(root)) continue;
    for (const name of fs.readdirSync(root)) {
      const file = path.join(root, name, "extension.build.json");
      if (!fs.existsSync(file)) continue;
      const data = JSON.parse(fs.readFileSync(file, "utf8"));
      if (!data.id || !data.path || !Array.isArray(data.targets)) {
        throw new Error(`invalid extension build metadata: ${file}`);
      }
      result.push(data);
    }
  }
  return result;
}

function runnerForTarget(target) {
  if (target === "universal") return "ubuntu-latest";
  if (target.includes("apple-darwin")) {
    return target.startsWith("x86_64") ? "macos-15-intel" : "macos-latest";
  }
  if (target.includes("windows")) return "windows-latest";
  return "ubuntu-latest";
}
```

- [ ] **Step 5: Create marketplace manifest generator**

Create `scripts/generate-marketplace-manifest.mjs`:

```javascript
import fs from "node:fs";
import path from "node:path";

const artifactDir = process.env.ARTIFACT_DIR || "artifacts";
const version = requiredEnv("EXTENSION_VERSION");
const releaseTag = requiredEnv("RELEASE_TAG");
const extensionId = requiredEnv("EXTENSION_ID");
const repository = requiredEnv("GITHUB_REPOSITORY");

const targets = [
  "aarch64-apple-darwin",
  "x86_64-apple-darwin",
  "x86_64-unknown-linux-gnu",
  "x86_64-pc-windows-msvc",
];

const checksums = readChecksums(path.join(artifactDir, "sha256sums.txt"));
const assetUrls = {};
const fallbackAssetUrls = {};
const sha256s = {};

for (const target of targets) {
  const fileName = `duckdb-driver-${target}.tar.gz`;
  assetUrls[target] = `${extensionId}/${version}/${fileName}`;
  fallbackAssetUrls[target] = `https://github.com/${repository}/releases/download/${releaseTag}/${fileName}`;
  sha256s[target] = checksumFor(checksums, fileName);
}

const manifest = {
  schema_version: 1,
  release_version: releaseTag,
  extensions: [],
};

const currentEntry = {
  id: extensionId,
  kind: "database_driver",
  name: "DuckDB",
  version,
  description: "DuckDB embedded analytical database IPC driver",
  asset_urls: assetUrls,
  fallback_asset_urls: fallbackAssetUrls,
  sha256s,
};

fs.mkdirSync(artifactDir, { recursive: true });
fs.mkdirSync("manifest/entries", { recursive: true });
fs.writeFileSync(
  `manifest/entries/${extensionId}.json`,
  `${JSON.stringify(currentEntry, null, 2)}\n`,
);

manifest.extensions = fs
  .readdirSync("manifest/entries")
  .filter((fileName) => fileName.endsWith(".json"))
  .sort()
  .map((fileName) =>
    JSON.parse(fs.readFileSync(path.join("manifest/entries", fileName), "utf8")),
  );

fs.writeFileSync(
  path.join(artifactDir, "extension-manifest.json"),
  `${JSON.stringify(manifest, null, 2)}\n`,
);

function requiredEnv(name) {
  const value = process.env[name];
  if (!value || !value.trim()) {
    throw new Error(`${name} is required`);
  }
  return value.trim();
}

function readChecksums(filePath) {
  const lines = fs.readFileSync(filePath, "utf8").trim().split(/\n/).filter(Boolean);
  return new Map(lines.map((line) => {
    const [sha256, fileName] = line.trim().split(/\s+/, 2);
    return [fileName, sha256];
  }));
}

function checksumFor(checksums, fileName) {
  const sha256 = checksums.get(fileName);
  if (!sha256 || !/^[0-9a-f]{64}$/.test(sha256)) {
    throw new Error(`missing checksum for ${fileName}`);
  }
  return sha256;
}
```

- [ ] **Step 6: Verify scripts locally**

Run on a host target that is available locally:

```bash
cargo build --release -p duckdb_driver
mkdir -p artifacts
bash scripts/package-driver.sh duckdb "$(rustc -vV | sed -n 's/^host: //p')" artifacts 1.0.0
bash scripts/verify-package.sh "artifacts/duckdb-driver-$(rustc -vV | sed -n 's/^host: //p').tar.gz"
(
  cd artifacts
  sha256sum duckdb-driver-*.tar.gz > sha256sums.txt
)
ARTIFACT_DIR=artifacts \
EXTENSION_VERSION=1.0.0 \
EXTENSION_ID=duckdb \
RELEASE_TAG=duckdb-v1.0.0 \
GITHUB_REPOSITORY=feigeCode/onetcli-extensions \
node scripts/generate-marketplace-manifest.mjs
node -e 'const fs = require("fs"); const m = JSON.parse(fs.readFileSync("artifacts/extension-manifest.json", "utf8")); const e = m.extensions[0]; if (e.id !== "duckdb" || !e.asset_urls["x86_64-unknown-linux-gnu"].startsWith("duckdb/1.0.0/")) process.exit(1);'
```

Expected: local package verifies and `artifacts/extension-manifest.json` contains a DuckDB database driver entry whose primary asset URLs are relative to the manifest directory.

---

## Task 3: Add CI To `onetcli-extensions`

**Files:**
- Create in new repository: `.github/workflows/ci.yml`

- [ ] **Step 1: Create CI workflow**

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  pull_request:
  push:
    branches:
      - main

permissions:
  contents: read

env:
  CARGO_TERM_COLOR: always

jobs:
  detect:
    name: Detect changed extensions
    runs-on: ubuntu-latest
    outputs:
      matrix: ${{ steps.detect.outputs.matrix }}
      has_changes: ${{ steps.detect.outputs.has_changes }}
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - id: detect
        run: |
          BASE_SHA="${{ github.event.pull_request.base.sha || github.event.before }}"
          HEAD_SHA="${{ github.sha }}"
          node scripts/changed-extensions.mjs "$BASE_SHA" "$HEAD_SHA" > matrix.json
          cat matrix.json
          echo "matrix=$(cat matrix.json)" >> "$GITHUB_OUTPUT"
          echo "has_changes=$(node -e 'const fs = require("fs"); const m = JSON.parse(fs.readFileSync("matrix.json", "utf8")); process.stdout.write(m.include.length > 0 ? "true" : "false");')" >> "$GITHUB_OUTPUT"

  test:
    name: Test changed Rust packages
    needs: detect
    if: ${{ needs.detect.outputs.has_changes == 'true' }}
    strategy:
      fail-fast: false
      matrix: ${{ fromJson(needs.detect.outputs.matrix) }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
      - name: Test Rust package
        if: ${{ matrix.package != '' }}
        run: cargo test -p ${{ matrix.package }} -- --nocapture

  package:
    name: Package (${{ matrix.extension }} / ${{ matrix.target }})
    needs: detect
    if: ${{ needs.detect.outputs.has_changes == 'true' }}
    strategy:
      fail-fast: false
      matrix: ${{ fromJson(needs.detect.outputs.matrix) }}
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
        if: ${{ matrix.target != 'universal' }}
        with:
          target: ${{ matrix.target }}
      - name: Build
        if: ${{ matrix.package != '' && matrix.target != 'universal' }}
        run: cargo build --release -p ${{ matrix.package }} --target ${{ matrix.target }}
      - name: Package
        shell: bash
        run: |
          mkdir -p artifacts
          if [ "${{ matrix.kind }}" = "database_driver" ]; then
            bash scripts/package-driver.sh "${{ matrix.extension }}" "${{ matrix.target }}" artifacts 1.0.0-ci
            bash scripts/verify-package.sh "artifacts/${{ matrix.extension }}-driver-${{ matrix.target }}.tar.gz"
          else
            echo "::error::Unsupported extension kind in CI package workflow: ${{ matrix.kind }}"
            exit 1
          fi
```

- [ ] **Step 2: Validate workflow YAML locally**

Run from the new repository:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/ci.yml"); puts "ci yaml ok"'
```

Expected: `ci yaml ok`.

---

## Task 4: Add Extension Release Workflow

**Files:**
- Create in new repository: `.github/workflows/release.yml`

- [ ] **Step 1: Create Release workflow**

Create `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags:
      - "*-v*"
  workflow_dispatch:
    inputs:
      extension:
        description: Extension id, for example duckdb
        required: true
        type: string
      version:
        description: Extension version, for example 1.0.0
        required: true
        type: string

permissions:
  contents: write

env:
  CARGO_TERM_COLOR: always

jobs:
  prepare:
    name: Resolve release extension
    runs-on: ubuntu-latest
    outputs:
      extension_id: ${{ steps.prepare.outputs.extension_id }}
      version: ${{ steps.prepare.outputs.version }}
      release_tag: ${{ steps.prepare.outputs.release_tag }}
      matrix: ${{ steps.prepare.outputs.matrix }}
    steps:
      - uses: actions/checkout@v4
      - id: prepare
        shell: bash
        run: |
          set -euo pipefail
          if [ "${{ github.event_name }}" = "workflow_dispatch" ]; then
            EXTENSION_ID="${{ inputs.extension }}"
            VERSION="${{ inputs.version }}"
            RELEASE_TAG="${EXTENSION_ID}-v${VERSION}"
          else
            RELEASE_TAG="$GITHUB_REF_NAME"
            EXTENSION_ID="${RELEASE_TAG%%-v*}"
            VERSION="${RELEASE_TAG#${EXTENSION_ID}-v}"
          fi
          MATRIX="$(EXTENSION_ID="$EXTENSION_ID" node <<'NODE'
          const fs = require("fs");
          const path = require("path");
          const id = process.env.EXTENSION_ID;
          const roots = ["extensions/ipc", "extensions/wasm", "extensions/language"];
          let metadata = null;
          for (const root of roots) {
            const file = path.join(root, id, "extension.build.json");
            if (fs.existsSync(file)) {
              metadata = JSON.parse(fs.readFileSync(file, "utf8"));
              break;
            }
          }
          if (!metadata) throw new Error(`unknown extension id: ${id}`);
          const include = metadata.targets.map((target) => ({
            extension: metadata.id,
            package: metadata.package || "",
            kind: metadata.kind,
            target,
            os: runnerForTarget(target),
          }));
          process.stdout.write(JSON.stringify({ include }));
          function runnerForTarget(target) {
            if (target === "universal") return "ubuntu-latest";
            if (target.includes("apple-darwin")) {
              return target.startsWith("x86_64") ? "macos-15-intel" : "macos-latest";
            }
            if (target.includes("windows")) return "windows-latest";
            return "ubuntu-latest";
          }
          NODE
          )"
          echo "extension_id=$EXTENSION_ID" >> "$GITHUB_OUTPUT"
          echo "version=$VERSION" >> "$GITHUB_OUTPUT"
          echo "release_tag=$RELEASE_TAG" >> "$GITHUB_OUTPUT"
          echo "matrix=$MATRIX" >> "$GITHUB_OUTPUT"

  build:
    name: Build (${{ matrix.extension }} / ${{ matrix.target }})
    needs: prepare
    strategy:
      fail-fast: false
      matrix: ${{ fromJson(needs.prepare.outputs.matrix) }}
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rust-lang/setup-rust-toolchain@v1
        if: ${{ matrix.target != 'universal' }}
        with:
          target: ${{ matrix.target }}
      - name: Build
        if: ${{ matrix.package != '' && matrix.target != 'universal' }}
        run: cargo build --release -p ${{ matrix.package }} --target ${{ matrix.target }}
      - name: Package
        shell: bash
        run: |
          mkdir -p artifacts
          if [ "${{ matrix.kind }}" = "database_driver" ]; then
            bash scripts/package-driver.sh "${{ matrix.extension }}" "${{ matrix.target }}" artifacts "${{ needs.prepare.outputs.version }}"
            bash scripts/verify-package.sh "artifacts/${{ matrix.extension }}-driver-${{ matrix.target }}.tar.gz"
          else
            echo "::error::Unsupported extension kind in release workflow: ${{ matrix.kind }}"
            exit 1
          fi
      - name: Upload package artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.extension }}-${{ matrix.target }}
          path: artifacts/${{ matrix.extension }}-driver-${{ matrix.target }}.tar.gz

  release:
    name: Create Release
    needs:
      - prepare
      - build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Download artifacts
        uses: actions/download-artifact@v4
        with:
          path: artifacts
          merge-multiple: true
      - name: Generate checksums
        run: |
          cd artifacts
          sha256sum *.tar.gz > sha256sums.txt
          cat sha256sums.txt
      - name: Generate marketplace manifest
        env:
          ARTIFACT_DIR: artifacts
          EXTENSION_ID: ${{ needs.prepare.outputs.extension_id }}
          EXTENSION_VERSION: ${{ needs.prepare.outputs.version }}
          RELEASE_TAG: ${{ needs.prepare.outputs.release_tag }}
        run: node scripts/generate-marketplace-manifest.mjs
      - name: Write release metadata
        run: |
          node <<'NODE'
          const fs = require("fs");
          fs.writeFileSync(
            "artifacts/release-metadata.json",
            `${JSON.stringify({
              release_tag: "${{ needs.prepare.outputs.release_tag }}",
              extension_id: "${{ needs.prepare.outputs.extension_id }}",
              extension_version: "${{ needs.prepare.outputs.version }}",
            }, null, 2)}\n`,
          );
          NODE
      - name: Upload release metadata
        uses: actions/upload-artifact@v4
        with:
          name: release-metadata
          path: artifacts/release-metadata.json
      - name: Create GitHub Release
        uses: softprops/action-gh-release@v2
        with:
          tag_name: ${{ needs.prepare.outputs.release_tag }}
          name: ${{ needs.prepare.outputs.extension_id }} ${{ needs.prepare.outputs.version }}
          generate_release_notes: true
          files: |
            artifacts/*.tar.gz
            artifacts/sha256sums.txt
            artifacts/extension-manifest.json
```

- [ ] **Step 2: Validate release workflow YAML locally**

Run:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/release.yml"); puts "release yaml ok"'
```

Expected: `release yaml ok`.

---

## Task 5: Add Extension R2 Upload Workflow

**Files:**
- Create in new repository: `.github/workflows/upload-r2.yml`

- [ ] **Step 1: Create R2 upload workflow**

Create `.github/workflows/upload-r2.yml`:

```yaml
name: Upload R2

on:
  workflow_run:
    workflows:
      - Release
    types:
      - completed
  workflow_dispatch:
    inputs:
      tag:
        description: Extension release tag, for example duckdb-v1.0.0
        required: true
        type: string

permissions:
  actions: read
  contents: read

jobs:
  upload:
    name: Upload extension assets to R2
    if: ${{ github.event_name == 'workflow_dispatch' || github.event.workflow_run.conclusion == 'success' }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Resolve release tag
        id: release
        env:
          GH_TOKEN: ${{ github.token }}
          INPUT_TAG: ${{ github.event_name == 'workflow_dispatch' && inputs.tag || '' }}
          WORKFLOW_RUN_ID: ${{ github.event_name == 'workflow_run' && github.event.workflow_run.id || '' }}
        run: |
          set -euo pipefail
          if [ -n "$INPUT_TAG" ]; then
            release_tag="$INPUT_TAG"
            extension_id="${release_tag%%-v*}"
            extension_version="${release_tag#${extension_id}-v}"
          else
            mkdir -p run-metadata
            gh run download "$WORKFLOW_RUN_ID" \
              --repo "$GITHUB_REPOSITORY" \
              --name release-metadata \
              --dir run-metadata
            release_tag="$(node -e 'const fs = require("fs"); const d = JSON.parse(fs.readFileSync("run-metadata/release-metadata.json", "utf8")); process.stdout.write(d.release_tag);')"
            extension_id="$(node -e 'const fs = require("fs"); const d = JSON.parse(fs.readFileSync("run-metadata/release-metadata.json", "utf8")); process.stdout.write(d.extension_id);')"
            extension_version="$(node -e 'const fs = require("fs"); const d = JSON.parse(fs.readFileSync("run-metadata/release-metadata.json", "utf8")); process.stdout.write(d.extension_version);')"
          fi
          gh release view "$release_tag" --repo "$GITHUB_REPOSITORY" >/dev/null
          echo "tag=$release_tag" >> "$GITHUB_OUTPUT"
          echo "extension_id=$extension_id" >> "$GITHUB_OUTPUT"
          echo "version=$extension_version" >> "$GITHUB_OUTPUT"

      - name: Resolve extension metadata
        id: extension
        env:
          EXTENSION_ID: ${{ steps.release.outputs.extension_id }}
        run: |
          set -euo pipefail
          metadata="$(node <<'NODE'
          const fs = require("fs");
          const path = require("path");
          const id = process.env.EXTENSION_ID;
          const roots = ["extensions/ipc", "extensions/wasm", "extensions/language"];
          let data = null;
          for (const root of roots) {
            const file = path.join(root, id, "extension.build.json");
            if (fs.existsSync(file)) {
              data = JSON.parse(fs.readFileSync(file, "utf8"));
              break;
            }
          }
          if (!data) throw new Error(`unknown extension id: ${id}`);
          process.stdout.write(JSON.stringify({
            kind: data.kind,
            targets: data.targets,
            r2Prefix: data.r2Prefix,
          }));
          NODE
          )"
          echo "kind=$(node -e 'const data = JSON.parse(process.argv[1]); process.stdout.write(data.kind);' "$metadata")" >> "$GITHUB_OUTPUT"
          echo "targets=$(node -e 'const data = JSON.parse(process.argv[1]); process.stdout.write(JSON.stringify(data.targets));' "$metadata")" >> "$GITHUB_OUTPUT"
          echo "r2_prefix=$(node -e 'const data = JSON.parse(process.argv[1]); process.stdout.write(data.r2Prefix);' "$metadata")" >> "$GITHUB_OUTPUT"

      - name: Verify R2 secrets are set
        env:
          CLOUDFLARE_ACCOUNT_ID: ${{ secrets.CLOUDFLARE_ACCOUNT_ID }}
          CLOUDFLARE_R2_ACCESS_KEY_ID: ${{ secrets.CLOUDFLARE_R2_ACCESS_KEY_ID }}
          CLOUDFLARE_R2_SECRET_ACCESS_KEY: ${{ secrets.CLOUDFLARE_R2_SECRET_ACCESS_KEY }}
          CLOUDFLARE_R2_BUCKET: ${{ secrets.CLOUDFLARE_R2_BUCKET }}
        run: |
          for name in CLOUDFLARE_ACCOUNT_ID CLOUDFLARE_R2_ACCESS_KEY_ID CLOUDFLARE_R2_SECRET_ACCESS_KEY CLOUDFLARE_R2_BUCKET; do
            if [ -z "${!name}" ]; then
              echo "::error::${name} secret is empty or not set"
              exit 1
            fi
          done

      - name: Download GitHub Release assets
        env:
          GH_TOKEN: ${{ github.token }}
          RELEASE_TAG: ${{ steps.release.outputs.tag }}
        run: |
          mkdir -p artifacts
          gh release download "$RELEASE_TAG" \
            --repo "$GITHUB_REPOSITORY" \
            --pattern "${{ steps.release.outputs.extension_id }}-driver-*.tar.gz" \
            --pattern "sha256sums.txt" \
            --pattern "extension-manifest.json" \
            --dir artifacts
          ls -la artifacts

      - name: Verify downloaded assets
        env:
          EXTENSION_ID: ${{ steps.release.outputs.extension_id }}
          TARGETS_JSON: ${{ steps.extension.outputs.targets }}
        run: |
          set -euo pipefail
          node -e 'const targets = JSON.parse(process.env.TARGETS_JSON); for (const target of targets) console.log(`${process.env.EXTENSION_ID}-driver-${target}.tar.gz`);' > expected-assets.txt
          while read -r file; do
            test -s "artifacts/${file}"
            awk -v file="$file" '$2 == file && $1 ~ /^[0-9a-f]{64}$/ { found = 1 } END { exit found ? 0 : 1 }' artifacts/sha256sums.txt
          done < expected-assets.txt
          test -s artifacts/extension-manifest.json
          node -e 'const fs = require("fs"); const m = JSON.parse(fs.readFileSync("artifacts/extension-manifest.json", "utf8")); if (!m.extensions || !m.extensions.length) process.exit(1);'

      - name: Configure Cloudflare R2 credentials
        uses: aws-actions/configure-aws-credentials@v4
        with:
          aws-access-key-id: ${{ secrets.CLOUDFLARE_R2_ACCESS_KEY_ID }}
          aws-secret-access-key: ${{ secrets.CLOUDFLARE_R2_SECRET_ACCESS_KEY }}
          aws-region: auto

      - name: Upload extension assets and manifest to R2
        env:
          CLOUDFLARE_ACCOUNT_ID: ${{ secrets.CLOUDFLARE_ACCOUNT_ID }}
          CLOUDFLARE_R2_BUCKET: ${{ secrets.CLOUDFLARE_R2_BUCKET }}
          EXTENSION_ID: ${{ steps.release.outputs.extension_id }}
          EXTENSION_VERSION: ${{ steps.release.outputs.version }}
          R2_PREFIX: ${{ steps.extension.outputs.r2_prefix }}
          TARGETS_JSON: ${{ steps.extension.outputs.targets }}
        run: |
          set -euo pipefail
          endpoint="https://${CLOUDFLARE_ACCOUNT_ID}.r2.cloudflarestorage.com"
          upload_object() {
            local source="$1"
            local key="$2"
            local content_type="$3"
            local cache_control="$4"
            aws s3 cp "$source" "s3://${CLOUDFLARE_R2_BUCKET}/${key}" \
              --endpoint-url "$endpoint" \
              --content-type "$content_type" \
              --cache-control "$cache_control"
          }
          node -e 'const targets = JSON.parse(process.env.TARGETS_JSON); for (const target of targets) console.log(`${process.env.EXTENSION_ID}-driver-${target}.tar.gz`);' > upload-assets.txt
          while read -r file; do
            upload_object "artifacts/${file}" "${R2_PREFIX}/${EXTENSION_VERSION}/${file}" "application/gzip" "public, max-age=31536000, immutable"
            upload_object "artifacts/${file}" "${R2_PREFIX}/latest/${file}" "application/gzip" "public, max-age=300"
          done < upload-assets.txt
          upload_object "artifacts/extension-manifest.json" "extensions/manifest.json" "application/json; charset=utf-8" "no-cache"
```

- [ ] **Step 2: Validate R2 workflow YAML locally**

Run:

```bash
ruby -e 'require "yaml"; YAML.load_file(".github/workflows/upload-r2.yml"); puts "upload-r2 yaml ok"'
```

Expected: `upload-r2 yaml ok`.

---

## Task 6: Publish And Verify The First Extension Release

**Files:**
- No source edits required after workflows are merged.

- [ ] **Step 1: Configure repository variables and secrets**

Set repository secrets:

```text
CLOUDFLARE_ACCOUNT_ID
CLOUDFLARE_R2_ACCESS_KEY_ID
CLOUDFLARE_R2_SECRET_ACCESS_KEY
CLOUDFLARE_R2_BUCKET
```

- [ ] **Step 2: Tag the first extension release**

Run from `onetcli-extensions`:

```bash
git tag duckdb-v1.0.0
git push origin duckdb-v1.0.0
```

Expected: GitHub Actions runs `Release`, then `Upload R2`.

- [ ] **Step 3: Verify public R2 manifest**

Run:

```bash
curl -fsSL https://onetcli.test.cn/extensions/manifest.json -o /tmp/onetcli-extension-manifest.json
node -e 'const fs = require("fs"); const m = JSON.parse(fs.readFileSync("/tmp/onetcli-extension-manifest.json", "utf8")); if (m.extensions[0].id !== "duckdb") process.exit(1); console.log("manifest ok");'
```

Expected: `manifest ok`.

- [ ] **Step 4: Verify one extension package from R2**

Run:

```bash
asset_url="$(node -e 'const fs = require("fs"); const m = JSON.parse(fs.readFileSync("/tmp/onetcli-extension-manifest.json", "utf8")); const asset = m.extensions[0].asset_urls["x86_64-unknown-linux-gnu"]; const manifest = "https://onetcli.test.cn/extensions/manifest.json"; const prefix = manifest.slice(0, manifest.lastIndexOf("/") + 1); process.stdout.write(/^https?:\/\//.test(asset) ? asset : prefix + asset.replace(/^\/+/, ""));')"
curl -fsSL "$asset_url" -o /tmp/duckdb-driver-x86_64-unknown-linux-gnu.tar.gz
tar tzf /tmp/duckdb-driver-x86_64-unknown-linux-gnu.tar.gz | sort
```

Expected output includes:

```text
duckdb/driver.json
duckdb/duckdb_driver
duckdb/locales/en.yml
duckdb/locales/zh-CN.yml
duckdb/locales/zh-HK.yml
```

---

## Task 7: Point `onetcli` GitHub Marketplace Fallback To The Extension Repository

**Files:**
- Modify in `onetcli`: `.github/workflows/release.yml`
- Modify in `onetcli`: `crates/extension-runtime/src/extension_downloader/transfer.rs`
- Test in `onetcli`: `crates/extension-runtime/src/extension_downloader_network_tests.rs`

- [ ] **Step 1: Add injectable GitHub fallback manifest URL**

In `crates/extension-runtime/build.rs`, pass through both extension manifest environment variables:

```rust
fn main() {
    for key in [
        "ONETCLI_EXTENSION_MANIFEST_URL",
        "ONETCLI_EXTENSION_GITHUB_MANIFEST_URL",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        if let Ok(value) = std::env::var(key)
            && !value.is_empty()
        {
            println!("cargo:rustc-env={key}={value}");
        }
    }
}
```

In `crates/extension-runtime/src/extension_downloader/transfer.rs`, keep the built-in GitHub fallback as the last resort and resolve the effective fallback in this order:

```rust
pub fn github_extension_manifest_url_from_parts(
    runtime: Option<&str>,
    build_time: Option<&str>,
) -> String {
    runtime
        .and_then(non_empty_trimmed)
        .or_else(|| build_time.and_then(non_empty_trimmed))
        .unwrap_or_else(|| GITHUB_EXTENSION_MANIFEST_URL.to_string())
}
```

- [ ] **Step 2: Inject the extension repository fallback in the main app release build**

Set this repository variable in `feigeCode/onetcli`:

```text
ONETCLI_EXTENSION_GITHUB_MANIFEST_URL=https://github.com/feigeCode/onetcli-extensions/releases/latest/download/extension-manifest.json
```

In `onetcli/.github/workflows/release.yml`, pass the repository variable through to app bundle builds:

```yaml
ONETCLI_EXTENSION_GITHUB_MANIFEST_URL: ${{ vars.ONETCLI_EXTENSION_GITHUB_MANIFEST_URL }}
```

Do not hard-code this URL in source. Do not require this variable for local developer builds. If it is absent, the client keeps using the built-in GitHub fallback URL.

- [ ] **Step 3: Add relative asset URL coverage**

In `crates/extension-runtime/src/extension_downloader_network_tests.rs`, add a test that fetches manifest URL `https://onetcli.test.cn/extensions/manifest.json` with relative primary and fallback asset URLs:

```json
{
  "asset_url": "packages/missing.tar.gz",
  "fallback_asset_url": "packages/fake_pg.tar.gz"
}
```

Expected download requests:

```text
https://onetcli.test.cn/extensions/packages/missing.tar.gz
https://onetcli.test.cn/extensions/packages/fake_pg.tar.gz
```

- [ ] **Step 4: Run extension marketplace tests**

Run:

```bash
cargo test -p extension-runtime extension_downloader_network_tests -- --nocapture
cargo test -p extension-runtime --features github-marketplace extension_downloader_network_tests -- --nocapture
```

Expected: default build tries configured/R2 manifest then GitHub fallback; injected `ONETCLI_EXTENSION_GITHUB_MANIFEST_URL` can redirect GitHub fallback to `onetcli-extensions`; relative asset URLs resolve against the manifest URL prefix; `github-marketplace` feature uses a GitHub-only manifest.

---

## Task 8: Remove DuckDB Driver Production From `onetcli`

**Files:**
- Modify in `onetcli`: `Cargo.toml`
- Delete in `onetcli`: `crates/duckdb_driver/**`
- Delete in `onetcli`: `script/package-ipc-drivers.sh`
- Modify in `onetcli`: `crates/db/src/ipc/registry/discovery.rs`
- Modify in `onetcli`: `docs/design/ipc-drivers.md`
- Modify in `onetcli`: `docs/design/release-distribution.md`

- [ ] **Step 1: Remove the driver crate from the workspace**

In root `Cargo.toml`, remove this member:

```toml
"crates/duckdb_driver",
```

If the `duckdb` workspace dependency is no longer used outside `crates/duckdb_driver`, remove:

```toml
duckdb = { version = "1.10501.0", features = ["bundled"] }
```

Keep the dependency if `crates/db` still uses `duckdb` directly.

- [ ] **Step 2: Delete the old driver source and package helper**

Run:

```bash
git rm -r crates/duckdb_driver
git rm script/package-ipc-drivers.sh
```

- [ ] **Step 3: Remove debug-only workspace fallback**

In `crates/db/src/ipc/registry/discovery.rs`, remove the fallback that pushes:

```rust
workspace_dir.join("crates").join("duckdb_driver")
```

The remaining supported discovery sources should be `ONETCLI_IPC_DRIVER_DIR`, the user config driver directory, and application bundled/portable driver directories.

- [ ] **Step 4: Update docs**

In `docs/design/ipc-drivers.md`, replace packaging instructions with a pointer to `feigeCode/onetcli-extensions` and keep only the runtime discovery contract.

In `docs/design/release-distribution.md`, record that extension GitHub fallback is now:

```text
https://github.com/feigeCode/onetcli-extensions/releases/latest/download/extension-manifest.json
```

- [ ] **Step 5: Run host-side checks**

Run:

```bash
cargo check -p main -p extension-runtime -p extension_view -p db -p db_view
cargo test -p extension-runtime extension_downloader_network_tests -- --nocapture
cargo test -p main update::tests -- --nocapture
git diff --check
```

Expected: `onetcli` still builds the host app and extension consumption path without owning the DuckDB driver source.

---

## Task 9: End-To-End Acceptance

**Files:**
- No source edits required.

- [ ] **Step 1: Verify R2-first extension marketplace**

Run the main app with:

```bash
ONETCLI_PUBLIC_BASE_URL=https://onetcli.test.cn cargo run -p main
```

Open the extension marketplace. Expected: DuckDB appears from `https://onetcli.test.cn/extensions/manifest.json`.

- [ ] **Step 2: Verify GitHub fallback manifest**

Run the main app with:

```bash
ONETCLI_EXTENSION_MANIFEST_URL=https://invalid.onetcli.local/extensions/manifest.json cargo run -p main
```

Open the extension marketplace. Expected: marketplace still loads from `https://github.com/feigeCode/onetcli-extensions/releases/latest/download/extension-manifest.json`.

- [ ] **Step 3: Verify R2 asset fallback to GitHub asset**

Create a temporary manifest whose DuckDB `asset_urls` points to an invalid R2 URL and whose `fallback_asset_urls` points to a real GitHub Release asset. Run:

```bash
ONETCLI_EXTENSION_MANIFEST_URL=http://127.0.0.1:8000/extension-manifest.json cargo run -p main
```

Expected: DuckDB install first tries the invalid primary asset, then downloads and installs from the GitHub fallback asset.

- [ ] **Step 4: Verify DuckDB connection after marketplace install**

Remove any local DuckDB driver installation from the app config extension directory. Start the app, create a DuckDB connection, accept the driver install prompt, and run:

```sql
select 1;
```

Expected: the driver is installed under the user extension directory, `IpcDriverRegistry::load_default()` discovers it, and the query returns one row.

---

## Task 10: Add Future IPC Drivers Or Wasm Extensions Without Full Builds

**Files:**
- Create in new repository for each IPC driver: `extensions/ipc/<driver-id>/extension.build.json`
- Create in new repository for each Wasm extension: `extensions/wasm/<extension-id>/extension.build.json`
- Modify in new repository when adding the first Wasm extension: `scripts/package-wasm-extension.sh`
- Modify in new repository when adding the first Wasm extension: `.github/workflows/ci.yml`
- Modify in new repository when adding the first Wasm extension: `.github/workflows/release.yml`
- Modify in new repository when adding the first Wasm extension: `.github/workflows/upload-r2.yml`

- [ ] **Step 1: Add another IPC driver with no workflow changes**

For a new PostgreSQL driver, create:

```text
extensions/ipc/postgres/
  Cargo.toml
  driver.json
  extension.build.json
  locales/
  src/
```

Create `extensions/ipc/postgres/extension.build.json`:

```json
{
  "id": "postgres",
  "kind": "database_driver",
  "package": "postgres_driver",
  "binary": "postgres_driver",
  "path": "extensions/ipc/postgres",
  "targets": [
    "aarch64-apple-darwin",
    "x86_64-apple-darwin",
    "x86_64-unknown-linux-gnu",
    "x86_64-pc-windows-msvc"
  ],
  "releaseTagPrefix": "postgres-v",
  "r2Prefix": "extensions/postgres"
}
```

Add the package to root `Cargo.toml`:

```toml
[workspace]
members = [
    "extensions/ipc/duckdb",
    "extensions/ipc/postgres",
]
```

Expected behavior:

- A PR touching only `extensions/ipc/postgres/**` builds only PostgreSQL driver targets.
- A tag `postgres-v1.0.0` builds only PostgreSQL driver targets.
- R2 upload writes only `extensions/postgres/1.0.0/*`, `extensions/postgres/latest/*`, and the merged `extensions/manifest.json`.

- [ ] **Step 2: Add Wasm package script before the first Wasm extension**

Create `scripts/package-wasm-extension.sh`:

```bash
#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "Usage: $0 <extension-id> <artifact-dir> <version>" >&2
  exit 2
fi

EXTENSION_ID="$1"
ARTIFACT_DIR="$2"
VERSION="$3"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
SOURCE_DIR="${REPO_DIR}/extensions/wasm/${EXTENSION_ID}"
PACKAGE_ROOT="${REPO_DIR}/target/extension-packages/universal"
EXTENSION_DIR="${PACKAGE_ROOT}/${EXTENSION_ID}"
ARCHIVE_NAME="${EXTENSION_ID}-${VERSION}.tar.gz"

if [ ! -f "${SOURCE_DIR}/extension.json" ]; then
  echo "Missing extension.json in ${SOURCE_DIR}" >&2
  exit 1
fi

rm -rf "$EXTENSION_DIR"
mkdir -p "$EXTENSION_DIR" "$ARTIFACT_DIR"
cp -R "${SOURCE_DIR}/." "$EXTENSION_DIR/"
node - "$EXTENSION_DIR/extension.json" "$VERSION" <<'NODE'
const fs = require("fs");
const [file, version] = process.argv.slice(2);
const manifest = JSON.parse(fs.readFileSync(file, "utf8"));
manifest.version = version;
fs.writeFileSync(file, `${JSON.stringify(manifest, null, 2)}\n`);
NODE
tar czf "${ARTIFACT_DIR}/${ARCHIVE_NAME}" -C "$PACKAGE_ROOT" "$EXTENSION_ID"
echo "${ARTIFACT_DIR}/${ARCHIVE_NAME}"
```

Run:

```bash
chmod +x scripts/package-wasm-extension.sh
```

- [ ] **Step 3: Add a Wasm extension metadata file**

For a SQL formatter extension, create `extensions/wasm/sql-formatter/extension.build.json`:

```json
{
  "id": "sql-formatter",
  "kind": "composite",
  "path": "extensions/wasm/sql-formatter",
  "targets": ["universal"],
  "releaseTagPrefix": "sql-formatter-v",
  "r2Prefix": "extensions/sql-formatter"
}
```

Expected behavior:

- A PR touching only `extensions/wasm/sql-formatter/**` builds only the universal SQL formatter package.
- A tag `sql-formatter-v1.0.0` builds only the universal SQL formatter package.
- R2 upload writes only `extensions/sql-formatter/1.0.0/*`, `extensions/sql-formatter/latest/*`, and the merged `extensions/manifest.json`.

- [ ] **Step 4: Extend CI and Release package branches for Wasm**

In `.github/workflows/ci.yml` and `.github/workflows/release.yml`, replace the unsupported-kind branch with:

```bash
elif [ "${{ matrix.kind }}" = "composite" ]; then
  bash scripts/package-wasm-extension.sh "${{ matrix.extension }}" artifacts "$EXTENSION_VERSION"
  bash scripts/verify-package.sh "artifacts/${{ matrix.extension }}-${EXTENSION_VERSION}.tar.gz"
else
  echo "::error::Unsupported extension kind: ${{ matrix.kind }}"
  exit 1
fi
```

In CI, set `EXTENSION_VERSION=1.0.0-ci` before that branch. In Release, use the resolved release version.

- [ ] **Step 5: Extend R2 upload file naming for universal Wasm packages**

In `.github/workflows/upload-r2.yml`, replace the asset list generator with metadata-aware logic:

```bash
node <<'NODE' > upload-assets.txt
const fs = require("fs");
const kind = process.env.EXTENSION_KIND;
const id = process.env.EXTENSION_ID;
const version = process.env.EXTENSION_VERSION;
const targets = JSON.parse(process.env.TARGETS_JSON);
for (const target of targets) {
  if (kind === "database_driver") {
    console.log(`${id}-driver-${target}.tar.gz`);
  } else if (target === "universal") {
    console.log(`${id}-${version}.tar.gz`);
  } else {
    console.log(`${id}-${target}.tar.gz`);
  }
}
NODE
```

Set the upload job environment:

```yaml
EXTENSION_KIND: ${{ steps.extension.outputs.kind }}
```

Expected: IPC driver packages and universal Wasm packages are uploaded by the same R2 workflow without uploading unrelated extension assets.

- [ ] **Step 6: Optional submodule registry mode**

If extensions later move into separate repositories, convert `extensions/<kind>/<id>` directories into submodules in `onetcli-extensions`. Keep `extension.build.json` at the submodule root. No main-app changes are required.

PR and Release rules stay the same:

```text
submodule pointer changed at extensions/ipc/duckdb -> build duckdb only
submodule pointer changed at extensions/wasm/sql-formatter -> build sql-formatter only
scripts or workflow changed -> build all extensions
```

Do not add extension submodules to the `onetcli` main app repository.

---

## Risk Controls

- Keep `extension-protocol` and `extension-driver` in `onetcli` for the first split. A separate SDK repository can be introduced after extension release automation is stable.
- Do not delete `crates/duckdb_driver` from `onetcli` until `onetcli-extensions` has published one verified GitHub Release and one verified R2 manifest.
- Treat `extensions/manifest.json` as `no-cache` in R2. Treat versioned extension packages as immutable.
- Keep `ONETCLI_IPC_DRIVER_DIR` support in `onetcli` so local development can point the host app at packages built by `onetcli-extensions`.
- Update the fallback manifest URL to `onetcli-extensions`; otherwise the split still depends on main-app releases for extension marketplace fallback.
- Keep PR CI path-based and Release tag-based. Do not introduce any workflow path that builds all extensions for a normal single-extension change.
