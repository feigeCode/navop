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
    if backup_path.exists() {
        let _ = std::fs::remove_file(backup_path);
    }

    if target_path.exists() {
        std::fs::rename(target_path, backup_path)?;
    }

    std::fs::copy(download_path, target_path)?;
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn apply_update_unix(download_path: &Path) -> Result<(), String> {
    let target_path =
        std::env::current_exe().map_err(|err| format!("获取当前路径失败: {}", err))?;
    apply_update_unix_with_target(download_path, &target_path)
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn apply_update_unix_with_target(download_path: &Path, target_path: &Path) -> Result<(), String> {
    if target_path.exists() {
        std::fs::remove_file(target_path).map_err(|err| format!("移除旧版本失败: {}", err))?;
    }

    match std::fs::rename(download_path, target_path) {
        Ok(()) => {}
        Err(err) if is_cross_device_link_error(&err) => {
            std::fs::copy(download_path, target_path)
                .map_err(|copy_err| format!("复制更新文件失败: {}", copy_err))?;
        }
        Err(err) => return Err(format!("替换更新文件失败: {}", err)),
    }
    #[cfg(unix)]
    set_executable_permission(target_path)?;

    restart_application(target_path)?;
    Ok(())
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
