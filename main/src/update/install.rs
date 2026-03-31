use std::path::{Path, PathBuf};
use std::process::Command;

use super::util::UpdateInstallAction;

pub(crate) fn start_install_update(download_path: PathBuf) -> Result<UpdateInstallAction, String> {
    #[cfg(target_os = "windows")]
    {
        spawn_windows_helper(&download_path)?;
        return Ok(UpdateInstallAction::Quit);
    }

    #[cfg(target_os = "macos")]
    {
        apply_update_unix(&download_path)?;
        return Ok(UpdateInstallAction::Quit);
    }

    #[cfg(target_os = "linux")]
    {
        apply_update_unix(&download_path)?;
        return Ok(UpdateInstallAction::Quit);
    }

    #[allow(unreachable_code)]
    Ok(UpdateInstallAction::Noop)
}

pub(super) fn apply_update_helper(download_path: &Path, target_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return apply_update_windows(download_path, target_path);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        return apply_update_unix_with_target(download_path, target_path);
    }

    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(target_os = "windows")]
fn spawn_windows_helper(download_path: &Path) -> Result<(), String> {
    let target_path =
        std::env::current_exe().map_err(|err| format!("获取当前路径失败: {}", err))?;

    Command::new(download_path)
        .arg(super::APPLY_UPDATE_FLAG)
        .arg(download_path)
        .arg(&target_path)
        .spawn()
        .map_err(|err| format!("启动更新进程失败: {}", err))?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn apply_update_windows(download_path: &Path, target_path: &Path) -> Result<(), String> {
    let backup_path = target_path.with_extension("old");
    let mut last_error = None;

    for _ in 0..120 {
        match try_replace_windows(download_path, target_path, &backup_path) {
            Ok(()) => {
                restart_application(target_path)?;
                return Ok(());
            }
            Err(err) if err.kind() == std::io::ErrorKind::PermissionDenied => {
                last_error = Some(err);
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            Err(err) => return Err(format!("替换更新文件失败: {}", err)),
        }
    }

    Err(format!(
        "更新失败: {}",
        last_error
            .map(|err| err.to_string())
            .unwrap_or_else(|| "未知原因".to_string())
    ))
}

#[cfg(target_os = "windows")]
fn try_replace_windows(
    download_path: &Path,
    target_path: &Path,
    backup_path: &Path,
) -> std::io::Result<()> {
    replace_target_with_backup(target_path, backup_path, || {
        replace_via_staging_copy(download_path, target_path)
    })
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn apply_update_unix(download_path: &Path) -> Result<(), String> {
    let target_path =
        std::env::current_exe().map_err(|err| format!("获取当前路径失败: {}", err))?;
    apply_update_unix_with_target(download_path, &target_path)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn apply_update_unix_with_target(download_path: &Path, target_path: &Path) -> Result<(), String> {
    let backup_path = target_path.with_extension("old");
    replace_target_with_backup(target_path, &backup_path, || {
        try_replace_unix(download_path, target_path)
    })
    .map_err(|err| format!("替换更新文件失败: {}", err))?;

    #[cfg(unix)]
    set_executable_permission(target_path)?;

    restart_application(target_path)?;
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn try_replace_unix(download_path: &Path, target_path: &Path) -> std::io::Result<()> {
    match std::fs::rename(download_path, target_path) {
        Ok(()) => Ok(()),
        Err(err) if is_cross_device_link_error(&err) => {
            replace_via_staging_copy(download_path, target_path)
        }
        Err(err) => Err(err),
    }
}

fn replace_target_with_backup(
    target_path: &Path,
    backup_path: &Path,
    replace: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    remove_file_if_exists(backup_path)?;

    let had_target = target_path.exists();
    if had_target {
        std::fs::rename(target_path, backup_path)?;
    }

    match replace() {
        Ok(()) => {
            if had_target {
                let _ = remove_file_if_exists(backup_path);
            }
            Ok(())
        }
        Err(err) => {
            rollback_target_from_backup(target_path, backup_path, had_target).map_err(
                |rollback_err| {
                    std::io::Error::other(format!("{}; 回滚失败: {}", err, rollback_err))
                },
            )?;
            Err(err)
        }
    }
}

fn rollback_target_from_backup(
    target_path: &Path,
    backup_path: &Path,
    had_target: bool,
) -> std::io::Result<()> {
    if !had_target {
        return Ok(());
    }

    remove_file_if_exists(target_path)?;
    if !backup_path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "缺少可回滚的备份文件",
        ));
    }

    std::fs::rename(backup_path, target_path)?;
    Ok(())
}

fn replace_via_staging_copy(download_path: &Path, target_path: &Path) -> std::io::Result<()> {
    let staging_path = target_path.with_extension("new");
    remove_file_if_exists(&staging_path)?;

    if let Err(err) = std::fs::copy(download_path, &staging_path) {
        let _ = remove_file_if_exists(&staging_path);
        return Err(err);
    }

    if let Err(err) = std::fs::rename(&staging_path, target_path) {
        let _ = remove_file_if_exists(&staging_path);
        return Err(err);
    }

    Ok(())
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn restart_application(target_path: &Path) -> Result<(), String> {
    Command::new(target_path)
        .spawn()
        .map_err(|err| format!("重启应用失败: {}", err))?;
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn is_cross_device_link_error(err: &std::io::Error) -> bool {
    err.raw_os_error() == Some(18)
}

#[cfg(unix)]
pub(super) fn set_executable_permission(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)
        .map_err(|err| format!("读取文件权限失败: {}", err))?
        .permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)
        .map_err(|err| format!("设置可执行权限失败: {}", err))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::replace_target_with_backup;

    #[test]
    fn replace_target_with_backup_rolls_back_on_replace_error() {
        let temp_dir = TestDir::new("replace-target-with-backup");
        let target_path = temp_dir.path.join("onetcli");
        let backup_path = temp_dir.path.join("onetcli.old");
        std::fs::write(&target_path, b"old-binary").expect("写入旧版本失败");

        let result = replace_target_with_backup(&target_path, &backup_path, || {
            Err(std::io::Error::other("模拟替换失败"))
        });

        assert!(result.is_err());
        let target_bytes = std::fs::read(&target_path).expect("回滚后旧版本应仍存在");
        assert_eq!(target_bytes, b"old-binary");
        assert!(!backup_path.exists());
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn apply_update_unix_keeps_old_target_when_replace_fails() {
        use super::apply_update_unix_with_target;

        let temp_dir = TestDir::new("apply-update-unix");
        let target_path = temp_dir.path.join("onetcli");
        let missing_download_path = temp_dir.path.join("missing-download");
        std::fs::write(&target_path, b"old-binary").expect("写入旧版本失败");

        let result = apply_update_unix_with_target(&missing_download_path, &target_path);

        assert!(result.is_err());
        let target_bytes = std::fs::read(&target_path).expect("替换失败后旧版本应仍存在");
        assert_eq!(target_bytes, b"old-binary");
    }

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(prefix: &str) -> Self {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("系统时间异常")
                .as_nanos();
            let path =
                std::env::temp_dir().join(format!("{}-{}-{}", prefix, std::process::id(), now));
            std::fs::create_dir_all(&path).expect("创建临时目录失败");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}
