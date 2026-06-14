# Release Distribution

This document records the production distribution contract for app updates and
extension marketplace downloads.

## Main App Releases

The main application release is split into two GitHub Actions workflows:

1. `.github/workflows/release.yml` builds `main`, packages platform assets,
   creates `sha256sums.txt`, writes `release-metadata`, and publishes the
   GitHub Release.
2. `.github/workflows/upload-r2.yml` runs after the `Release` workflow completes
   successfully, downloads the GitHub Release assets, generates
   `updates/latest.json`, and uploads the app assets plus update manifest to R2.

The release workflow does not upload to R2 directly. This keeps GitHub Release
creation as the first successful publication point and makes R2 upload
retryable through `workflow_dispatch` with a release tag.

The R2 workflow uploads:

```text
releases/<tag>/<asset>
releases/latest/<asset>
updates/latest.json
```

`updates/latest.json` contains target-triple download URLs and sha256 values:

```json
{
  "schema_version": 1,
  "version": "0.4.8",
  "release_page_url": "https://<public-base>/releases/v0.4.8/",
  "downloads": {
    "aarch64-apple-darwin": "https://<public-base>/releases/v0.4.8/onetcli-aarch64-apple-darwin.tar.gz",
    "x86_64-apple-darwin": "https://<public-base>/releases/v0.4.8/onetcli-x86_64-apple-darwin.tar.gz",
    "x86_64-unknown-linux-gnu": "https://<public-base>/releases/v0.4.8/onetcli-x86_64-unknown-linux-gnu.tar.gz",
    "x86_64-pc-windows-msvc": "https://<public-base>/releases/v0.4.8/onetcli-x86_64-pc-windows-msvc.zip"
  },
  "sha256s": {
    "aarch64-apple-darwin": "<sha256>",
    "x86_64-apple-darwin": "<sha256>",
    "x86_64-unknown-linux-gnu": "<sha256>",
    "x86_64-pc-windows-msvc": "<sha256>"
  }
}
```

The client reads `ONETCLI_PUBLIC_BASE_URL` from the runtime environment first,
then from the build-time value injected by `build.rs`. When that value is
present, the update URL is derived as `<public-base>/updates/latest.json`.
`ONETCLI_UPDATE_URL` remains an explicit override for custom update endpoints.

There is no hardcoded Cloudflare public base URL in production code. If neither
`ONETCLI_UPDATE_URL` nor `ONETCLI_PUBLIC_BASE_URL` is provided, the client uses
GitHub Releases directly. With the default build, a configured R2/custom update
source is tried first and GitHub Releases are tried next when R2 is unavailable,
returns an invalid manifest, or is stale. The `github-distribution` feature uses
GitHub-only app updates and GitHub-only extension marketplace manifests.

## Extension Marketplace

This repository owns the marketplace consumption mechanism. Extension-specific
pipelines own extension package builds and the marketplace manifest publication.

The default extension manifest source is resolved in this order:

1. Runtime `ONETCLI_EXTENSION_MANIFEST_URL`
2. Build-time `ONETCLI_EXTENSION_MANIFEST_URL`
3. `ONETCLI_PUBLIC_BASE_URL` plus `extensions/manifest.json`
4. GitHub Release asset fallback:
   `https://github.com/feigeCode/onetcli/releases/latest/download/extension-manifest.json`

The extension manifest should prefer Cloudflare/R2 asset URLs and include
GitHub fallback URLs for every downloadable package. The client supports both
global and target-specific asset fields:

```json
{
  "schema_version": 1,
  "release_version": "2026.06",
  "extensions": [
    {
      "id": "duckdb",
      "kind": "database_driver",
      "name": "DuckDB",
      "version": "1.0.0",
      "asset_url": "https://<public-base>/extensions/duckdb/duckdb.tar.gz",
      "fallback_asset_url": "https://github.com/<org>/<repo>/releases/download/v1.0.0/duckdb.tar.gz",
      "sha256": "<sha256>"
    }
  ]
}
```

For platform-specific packages, use target triples first and OS names as
fallback keys:

```json
{
  "id": "duckdb",
  "kind": "database_driver",
  "name": "DuckDB",
  "version": "1.0.0",
  "asset_urls": {
    "aarch64-apple-darwin": "https://<public-base>/extensions/duckdb/duckdb-aarch64-apple-darwin.tar.gz",
    "x86_64-apple-darwin": "https://<public-base>/extensions/duckdb/duckdb-x86_64-apple-darwin.tar.gz",
    "x86_64-unknown-linux-gnu": "https://<public-base>/extensions/duckdb/duckdb-x86_64-unknown-linux-gnu.tar.gz",
    "x86_64-pc-windows-msvc": "https://<public-base>/extensions/duckdb/duckdb-x86_64-pc-windows-msvc.tar.gz"
  },
  "fallback_asset_urls": {
    "aarch64-apple-darwin": "https://github.com/<org>/<repo>/releases/download/v1.0.0/duckdb-aarch64-apple-darwin.tar.gz",
    "x86_64-apple-darwin": "https://github.com/<org>/<repo>/releases/download/v1.0.0/duckdb-x86_64-apple-darwin.tar.gz",
    "x86_64-unknown-linux-gnu": "https://github.com/<org>/<repo>/releases/download/v1.0.0/duckdb-x86_64-unknown-linux-gnu.tar.gz",
    "x86_64-pc-windows-msvc": "https://github.com/<org>/<repo>/releases/download/v1.0.0/duckdb-x86_64-pc-windows-msvc.tar.gz"
  },
  "sha256s": {
    "aarch64-apple-darwin": "<sha256>",
    "x86_64-apple-darwin": "<sha256>",
    "x86_64-unknown-linux-gnu": "<sha256>",
    "x86_64-pc-windows-msvc": "<sha256>"
  }
}
```

`github_asset_url` and `github_asset_urls` are accepted as aliases for the
fallback fields. Non-language extension packages require sha256 validation
metadata before installation.

## Required GitHub Configuration

Repository variables:

- `ONETCLI_PUBLIC_BASE_URL`: public Cloudflare/R2 base URL, for example
  `https://onetcli.test.cn`

Repository secrets for R2 upload:

- `CLOUDFLARE_ACCOUNT_ID`
- `CLOUDFLARE_R2_ACCESS_KEY_ID`
- `CLOUDFLARE_R2_SECRET_ACCESS_KEY`
- `CLOUDFLARE_R2_BUCKET`

The application build can omit `ONETCLI_PUBLIC_BASE_URL`; in that case the
client falls back to GitHub release and marketplace sources.
