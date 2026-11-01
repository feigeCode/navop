use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const CURRENT_APP_DIR_NAME: &str = "navop";
const LEGACY_APP_DIR_NAME: &str = "one-hub";
const MIGRATION_MARKER: &str = ".one-hub-migration-complete";
const COPY_TEMP_SUFFIX: &str = ".navop-migration-tmp";

#[derive(Clone, Copy, Debug)]
struct AppDirNames {
    current: &'static str,
    legacy: &'static [&'static str],
    migration_marker: &'static str,
}

const DEFAULT_APP_DIR_NAMES: AppDirNames = AppDirNames {
    current: CURRENT_APP_DIR_NAME,
    legacy: &[LEGACY_APP_DIR_NAME],
    migration_marker: MIGRATION_MARKER,
};

static APP_DIR_NAMES: OnceLock<AppDirNames> = OnceLock::new();

/// 在 `initialize_app_paths` 之前按产品身份覆盖目录名。
///
/// 默认值保持 Navop 全量版的历史目录，确保老用户升级后仍能找到数据。
/// 独立产品（navop-shell / navop-data）必须在任何目录访问前调用本函数，
/// 否则会读写全量版的配置与数据目录。
pub fn configure_app_dir_names(
    current: &'static str,
    legacy: &'static [&'static str],
    migration_marker: &'static str,
) {
    let _ = APP_DIR_NAMES.set(AppDirNames {
        current,
        legacy,
        migration_marker,
    });
}

fn app_dir_names() -> &'static AppDirNames {
    APP_DIR_NAMES.get().unwrap_or(&DEFAULT_APP_DIR_NAMES)
}

/// 当前产品使用的数据目录名。
pub fn current_app_dir_name() -> &'static str {
    app_dir_names().current
}

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

/// 全量产品数据库文件名，与 [`storage`](crate::storage) 里 `get_db_path` 的拼法一致。
pub const DATABASE_FILE_NAME: &str = "one-hub.db";

/// 与当前运行身份无关的**全量产品**（navop）配置目录候选，顺序即优先级。
///
/// 独立产品（`navop-shell` / `navop-data`）启动时把目录名换成了
/// `.navop-data-dev` 之类，[`config_dir`] 只对它自己的数据有效；迁移入口要按全量
/// 产品自己的目录规则去找它的 `one-hub.db`，所以这里始终按全量产品的目录名
/// （现行 `navop` + 迁移前的 `one-hub`）解析：
///
/// 1. 安装态配置根：Windows `%APPDATA%`，其它平台 `~/.config`（与
///    [`config_root`] 一致，即全量产品自己的首选位置）；
/// 2. 开发构建：`~/.navop-dev/config`（`cargo run -p navop` 的目录）；
/// 3. 数据目录根：旧版本把配置放这里（macOS `~/Library/Application Support`、
///    Linux `~/.local/share`）。
///
/// 便携版（`<安装目录>/data/config`）不在这里猜：独立产品自己的便携目录恰好也
/// 是这个形状，猜错就会把独立产品的库当成全量产品的库；便携版用户用导入弹窗里的
/// 「选择文件…」手动指定。
pub fn full_product_config_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Ok(root) = config_root() {
        push_unique_dir(&mut dirs, root.join(CURRENT_APP_DIR_NAME));
        push_unique_dir(&mut dirs, root.join(LEGACY_APP_DIR_NAME));
    }

    if let Some(home) = dirs::home_dir() {
        push_unique_dir(
            &mut dirs,
            home.join(crate::app_paths::DEVELOPMENT_DATA_DIR_NAME)
                .join("config"),
        );
        // Windows 上 `config_root` 是 `%APPDATA%`，但开发/早期版本可能在
        // `~/.config` 下留过库。
        push_unique_dir(&mut dirs, home.join(".config").join(CURRENT_APP_DIR_NAME));
        push_unique_dir(&mut dirs, home.join(".config").join(LEGACY_APP_DIR_NAME));
    }

    if let Some(root) = dirs::data_dir() {
        push_unique_dir(&mut dirs, root.join(CURRENT_APP_DIR_NAME));
        push_unique_dir(&mut dirs, root.join(LEGACY_APP_DIR_NAME));
    }

    dirs
}

/// 全量产品数据库文件的候选路径（顺序即优先级）。
pub fn full_product_database_candidates() -> Vec<PathBuf> {
    full_product_config_dirs()
        .into_iter()
        .map(|dir| dir.join(DATABASE_FILE_NAME))
        .collect()
}

fn push_unique_dir(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
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
    let names = app_dir_names();
    let Some(legacy_name) = names.legacy.first() else {
        return Ok(MigrationOutcome::AlreadyMigrated);
    };
    migrate_directory(&root.join(legacy_name), &current_dir_from_root(root))
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
    current.join(app_dir_names().migration_marker)
}

fn current_dir_from_root(root: &Path) -> PathBuf {
    root.join(app_dir_names().current)
}

fn legacy_dir_from_root(root: &Path) -> PathBuf {
    let legacy = app_dir_names()
        .legacy
        .first()
        .copied()
        .unwrap_or(app_dir_names().current);
    root.join(legacy)
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

    #[test]
    fn full_product_candidates_use_full_product_names_and_are_unique() {
        let candidates = full_product_database_candidates();

        assert!(candidates.len() >= 4, "candidates: {candidates:?}");
        for path in &candidates {
            assert_eq!(
                path.file_name().and_then(|name| name.to_str()),
                Some(DATABASE_FILE_NAME),
                "{path:?} 应当指向全量产品的数据库文件"
            );
        }

        let mut deduped = candidates.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(deduped.len(), candidates.len(), "候选不应重复");

        let home = dirs::home_dir().expect("home");
        // 安装态（非 Windows 是 ~/.config）与开发构建的目录都必须覆盖到。
        assert!(
            candidates.contains(
                &home
                    .join(".config")
                    .join(CURRENT_APP_DIR_NAME)
                    .join(DATABASE_FILE_NAME)
            )
        );
        assert!(
            candidates.contains(
                &home
                    .join(crate::app_paths::DEVELOPMENT_DATA_DIR_NAME)
                    .join("config")
                    .join(DATABASE_FILE_NAME)
            )
        );
        // 历史目录 `one-hub` 也要能在迁移前被找到。
        assert!(
            candidates.contains(
                &home
                    .join(".config")
                    .join(LEGACY_APP_DIR_NAME)
                    .join(DATABASE_FILE_NAME)
            )
        );
    }

    #[test]
    fn full_product_candidates_do_not_guess_the_portable_install_dir() {
        // 独立产品自己的便携数据目录同样是 `<exe>/data/config`，猜它会把独立产品的
        // 库当成全量产品的库；便携版用户走导入弹窗的「选择文件…」。
        let exe = std::env::current_exe().expect("current exe");
        let portable = exe
            .parent()
            .expect("exe dir")
            .join("data")
            .join("config")
            .join(DATABASE_FILE_NAME);

        assert!(!full_product_database_candidates().contains(&portable));
    }
}
