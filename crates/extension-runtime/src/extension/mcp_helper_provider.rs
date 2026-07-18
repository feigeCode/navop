use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use serde::Deserialize;

use crate::extension::{ExtensionKind, ExtensionProvider, ExtensionSummary};

const MANIFEST_FILE: &str = "mcp_helper.json";

pub struct McpHelperExtensionProvider;

impl ExtensionProvider for McpHelperExtensionProvider {
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::McpHelper
    }

    fn list_installed(&self, root: &Path) -> Result<Vec<ExtensionSummary>> {
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut summaries = Vec::new();
        for entry in std::fs::read_dir(root).with_context(|| format!("read {}", root.display()))? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                summaries.push(to_summary(&load_manifest(&entry.path())?));
            }
        }
        Ok(summaries)
    }

    fn install_from_dir(&self, dir: &Path) -> Result<ExtensionSummary> {
        let manifest = load_manifest(dir)?;
        Ok(to_summary(&manifest))
    }

    fn uninstall(&self, dir: &Path) -> Result<String> {
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .map(String::from)
            .ok_or_else(|| anyhow!("无效的 MCP helper 目录名: {}", dir.display()))?;
        std::fs::remove_dir_all(dir)
            .with_context(|| format!("删除 MCP helper 目录 {}", dir.display()))?;
        Ok(name)
    }
}

#[derive(Debug, Deserialize)]
struct McpHelperManifest {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: String,
    entry: McpHelperEntry,
    distribution: Option<McpHelperDistribution>,
    #[serde(skip)]
    manifest_dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct McpHelperEntry {
    command: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum McpHelperDistribution {
    Npm { package: String, version: String },
}

fn load_manifest(dir: &Path) -> Result<McpHelperManifest> {
    let manifest_path = dir.join(MANIFEST_FILE);
    if !manifest_path.exists() {
        anyhow::bail!("未在 {} 找到 {MANIFEST_FILE}", dir.display());
    }
    let bytes = std::fs::read(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    let mut manifest: McpHelperManifest = serde_json::from_slice(&bytes)
        .with_context(|| format!("解析 {MANIFEST_FILE} 失败: {}", manifest_path.display()))?;
    validate_manifest(&manifest, dir)?;
    manifest.manifest_dir = dir.to_path_buf();
    Ok(manifest)
}

fn validate_manifest(manifest: &McpHelperManifest, dir: &Path) -> Result<()> {
    if manifest.id.trim().is_empty() {
        anyhow::bail!("{MANIFEST_FILE} 缺少 id");
    }
    if manifest.name.trim().is_empty() {
        anyhow::bail!("{MANIFEST_FILE} 缺少 name");
    }
    if manifest.entry.command.trim().is_empty() {
        anyhow::bail!("{MANIFEST_FILE} 缺少 entry.command");
    }
    match &manifest.distribution {
        Some(McpHelperDistribution::Npm { package, version }) => {
            validate_npm_distribution(manifest, package, version)?;
        }
        None => validate_entry_command(dir, manifest.entry.command.trim())?,
    }
    Ok(())
}

fn validate_npm_distribution(
    manifest: &McpHelperManifest,
    package: &str,
    version: &str,
) -> Result<()> {
    if !is_valid_scoped_package(package) {
        anyhow::bail!("{MANIFEST_FILE} npm package is invalid: {package}");
    }
    if !is_exact_semver(version) {
        anyhow::bail!("{MANIFEST_FILE} npm version must be exact semver: {version}");
    }
    let expected = ["-y", &format!("{package}@{version}"), "mcp"];
    if manifest.entry.command != "npx"
        || manifest.entry.args.iter().map(String::as_str).ne(expected)
    {
        anyhow::bail!("{MANIFEST_FILE} npm entry must use exact-version npx arguments");
    }
    Ok(())
}

fn is_valid_scoped_package(package: &str) -> bool {
    let Some((scope, name)) = package
        .strip_prefix('@')
        .and_then(|value| value.split_once('/'))
    else {
        return false;
    };
    !scope.is_empty()
        && !name.is_empty()
        && scope.chars().all(package_char)
        && name.chars().all(package_char)
}

fn package_char(character: char) -> bool {
    character.is_ascii_lowercase()
        || character.is_ascii_digit()
        || matches!(character, '-' | '_' | '.')
}

fn is_exact_semver(version: &str) -> bool {
    let core = version.split_once('-').map_or(version, |(core, _)| core);
    let mut parts = core.split('.');
    let valid = (0..3).all(|_| parts.next().is_some_and(valid_number));
    valid && parts.next().is_none() && !version.contains(['*', '^', '~', ' '])
}

fn valid_number(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn validate_entry_command(dir: &Path, command: &str) -> Result<()> {
    let command_path = Path::new(command);
    if command_path.is_absolute() || command_path.components().any(reject_component) {
        anyhow::bail!("{MANIFEST_FILE} entry.command must stay inside package: {command}");
    }
    let resolved = dir.join(command_path);
    if !resolved.is_file() {
        anyhow::bail!(
            "{MANIFEST_FILE} entry.command does not exist or is not a file: {}",
            resolved.display()
        );
    }
    validate_command_is_executable(&resolved)?;
    Ok(())
}

fn reject_component(component: Component<'_>) -> bool {
    matches!(component, Component::ParentDir | Component::Prefix(_))
}

#[cfg(unix)]
fn validate_command_is_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mode = std::fs::metadata(path)
        .with_context(|| format!("metadata {}", path.display()))?
        .permissions()
        .mode();
    if mode & 0o111 == 0 {
        anyhow::bail!(
            "{MANIFEST_FILE} entry.command is not executable: {}",
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
fn validate_command_is_executable(_path: &Path) -> Result<()> {
    Ok(())
}

fn to_summary(manifest: &McpHelperManifest) -> ExtensionSummary {
    let description = if manifest.description.trim().is_empty() {
        format!("{} helper", manifest.name)
    } else {
        manifest.description.clone()
    };
    let version = if manifest.version.trim().is_empty() {
        "0.0.0".to_string()
    } else {
        manifest.version.clone()
    };
    ExtensionSummary::new(
        ExtensionKind::McpHelper,
        manifest.id.clone(),
        version,
        manifest.manifest_dir.clone(),
    )
    .with_description(description)
}
