use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

const CURRENT_APP_DIR_NAME: &str = "navop";
const LEGACY_APP_DIR_NAME: &str = "one-hub";
const MIGRATION_MARKER: &str = ".one-hub-migration-complete";
const COPY_TEMP_SUFFIX: &str = ".navop-migration-tmp";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MigrationOutcome {
    Migrated,
    Merged,
    AlreadyMigrated,
    LegacyMissing,
}

pub fn config_dir() -> Result<PathBuf> {
    if let Some(paths) = crate::app_paths::initialized_paths() {
        return Ok(paths.config_dir().clone());
    }
    installed_config_dir()
}

pub(crate) fn installed_config_dir() -> Result<PathBuf> {
    let root = config_root()?;
    Ok(preferred_dir_from_root(&root))
}

pub fn data_dir() -> Option<PathBuf> {
    if let Some(paths) = crate::app_paths::initialized_paths() {
        return paths.data_dir().cloned();
    }
    installed_data_dir()
}

pub(crate) fn installed_data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|root| preferred_dir_from_root(&root))
}

/// 单元测试专用的数据目录。
///
/// `cargo test` 绝不能落盘到真实用户数据目录：验证文件、密钥元信息这类
/// 「固定文件名」的状态一旦被测试覆盖，已安装的正式版本就再也解不开自己的
/// 数据（2026-10-26 的 key_verification 污染事故即此原因）。每个测试进程
/// 使用独立临时目录，既隔离真实目录，也避免多次运行互相干扰。
#[cfg(test)]
pub(crate) fn test_data_dir() -> PathBuf {
    static TEST_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    TEST_DATA_DIR
        .get_or_init(|| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|elapsed| elapsed.as_nanos())
                .unwrap_or_default();
            let dir = std::env::temp_dir()
                .join(format!("navop-test-data-{}-{nanos}", std::process::id()));
            let _ = std::fs::create_dir_all(&dir);
            dir
        })
        .clone()
}

pub fn migrate_legacy_directories() -> Result<()> {
    // 便携与 debug 开发目录都不该触碰生产安装目录。
    if crate::app_paths::initialized_paths()
        .is_some_and(|paths| !paths.uses_installed_directories())
    {
        return Ok(());
    }

    let config_root = config_root()?;
    migrate_root(&config_root).context("migrate legacy configuration directory")?;

    if let Some(data_root) = dirs::data_dir()
        && data_root != config_root
    {
        migrate_root(&data_root).context("migrate legacy data directory")?;
    }
    Ok(())
}

fn config_root() -> Result<PathBuf> {
    if cfg!(target_os = "windows") {
        dirs::config_dir().context("Could not find config directory")
    } else {
        dirs::home_dir()
            .context("Could not find home directory")
            .map(|home| home.join(".config"))
    }
}

fn migrate_root(root: &Path) -> Result<MigrationOutcome> {
    migrate_directory(&legacy_dir_from_root(root), &current_dir_from_root(root))
}

fn migrate_directory(legacy: &Path, current: &Path) -> Result<MigrationOutcome> {
    if !legacy.exists() {
        return Ok(MigrationOutcome::LegacyMissing);
    }
    if migration_marker(current).exists() {
        return Ok(MigrationOutcome::AlreadyMigrated);
    }
    if current.exists() {
        copy_missing_entries(legacy, current)?;
        write_migration_marker(current)?;
        return Ok(MigrationOutcome::Merged);
    }

    std::fs::rename(legacy, current).with_context(|| {
        format!(
            "rename {} to {}",
            legacy.to_string_lossy(),
            current.to_string_lossy()
        )
    })?;
    write_migration_marker(current)?;
    Ok(MigrationOutcome::Migrated)
}

fn preferred_dir_from_root(root: &Path) -> PathBuf {
    let current = current_dir_from_root(root);
    let legacy = legacy_dir_from_root(root);
    if legacy.exists() && !migration_marker(&current).exists() {
        legacy
    } else {
        current
    }
}

fn copy_missing_entries(source: &Path, target: &Path) -> Result<()> {
    for entry in std::fs::read_dir(source).with_context(|| format!("read {}", source.display()))? {
        let entry = entry?;
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if target_path.exists() {
            if entry.file_type()?.is_dir() && target_path.is_dir() {
                copy_missing_entries(&source_path, &target_path)?;
            }
            continue;
        }
        copy_entry(&source_path, &target_path, entry.file_type()?.is_dir())?;
    }
    Ok(())
}

fn copy_entry(source: &Path, target: &Path, is_dir: bool) -> Result<()> {
    if is_dir {
        std::fs::create_dir(target).with_context(|| format!("create {}", target.display()))?;
        copy_missing_entries(source, target)
    } else {
        let temporary = copy_temporary_path(target);
        if temporary.exists() {
            std::fs::remove_file(&temporary)
                .with_context(|| format!("remove stale {}", temporary.display()))?;
        }
        std::fs::copy(source, &temporary)
            .with_context(|| format!("copy {} to {}", source.display(), temporary.display()))?;
        std::fs::rename(&temporary, target)
            .with_context(|| format!("rename {} to {}", temporary.display(), target.display()))?;
        Ok(())
    }
}

fn copy_temporary_path(target: &Path) -> PathBuf {
    let file_name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!("{file_name}{COPY_TEMP_SUFFIX}"))
}

fn write_migration_marker(current: &Path) -> Result<()> {
    std::fs::write(migration_marker(current), b"1")
        .with_context(|| format!("write migration marker in {}", current.display()))
}

fn migration_marker(current: &Path) -> PathBuf {
    current.join(MIGRATION_MARKER)
}

fn current_dir_from_root(root: &Path) -> PathBuf {
    root.join(CURRENT_APP_DIR_NAME)
}

fn legacy_dir_from_root(root: &Path) -> PathBuf {
    root.join(LEGACY_APP_DIR_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_directory_uses_navop_name() {
        let root = std::path::Path::new("/tmp/app-root");

        assert_eq!(root.join("navop"), current_dir_from_root(root));
        assert_eq!(root.join("one-hub"), legacy_dir_from_root(root));
    }

    #[test]
    fn directory_resolution_falls_back_to_legacy_after_a_failed_migration() {
        let temp = tempfile::tempdir().expect("tempdir");
        let legacy = temp.path().join("one-hub");
        let current = temp.path().join("navop");
        std::fs::create_dir_all(&legacy).expect("create legacy directory");
        std::fs::create_dir_all(&current).expect("create current directory");

        assert_eq!(legacy, preferred_dir_from_root(temp.path()));
    }

    #[test]
    fn migration_moves_the_legacy_directory_when_current_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let legacy = temp.path().join("one-hub");
        let current = temp.path().join("navop");
        std::fs::create_dir_all(legacy.join("extensions")).expect("create legacy directory");
        std::fs::write(legacy.join("extensions/manifest.json"), "legacy")
            .expect("write legacy file");

        migrate_directory(&legacy, &current).expect("migrate directory");

        assert!(!legacy.exists());
        assert_eq!(
            "legacy",
            std::fs::read_to_string(current.join("extensions/manifest.json"))
                .expect("read migrated file")
        );
    }

    #[test]
    fn migration_merges_into_an_existing_current_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let legacy = temp.path().join("one-hub");
        let current = temp.path().join("navop");
        std::fs::create_dir_all(&legacy).expect("create legacy directory");
        std::fs::create_dir_all(&current).expect("create current directory");
        std::fs::write(legacy.join("settings.json"), "legacy").expect("write legacy file");
        std::fs::write(legacy.join("auth.json"), "auth").expect("write legacy auth");
        std::fs::write(current.join("settings.json"), "current").expect("write current file");

        let outcome = migrate_directory(&legacy, &current).expect("inspect directories");

        assert_eq!(MigrationOutcome::Merged, outcome);
        assert_eq!(
            "current",
            std::fs::read_to_string(current.join("settings.json")).expect("read current file")
        );
        assert_eq!(
            "auth",
            std::fs::read_to_string(current.join("auth.json")).expect("read migrated auth")
        );
        assert!(legacy.exists());
        assert_eq!(current, preferred_dir_from_root(temp.path()));

        std::fs::write(legacy.join("auth.json"), "changed").expect("change legacy auth");
        let outcome = migrate_directory(&legacy, &current).expect("repeat migration");
        assert_eq!(MigrationOutcome::AlreadyMigrated, outcome);
        assert_eq!(
            "auth",
            std::fs::read_to_string(current.join("auth.json")).expect("read current auth")
        );
    }

    #[test]
    fn migration_is_a_noop_when_legacy_directory_is_missing() {
        let temp = tempfile::tempdir().expect("tempdir");
        let legacy = temp.path().join("one-hub");
        let current = temp.path().join("navop");

        let outcome = migrate_directory(&legacy, &current).expect("inspect directories");

        assert_eq!(MigrationOutcome::LegacyMissing, outcome);
        assert!(!current.exists());
    }
}
