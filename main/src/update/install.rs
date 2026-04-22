use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::extract::extract_archive;
use super::util::UpdateInstallAction;

pub(crate) fn start_install_update(download_path: PathBuf) -> Result<UpdateInstallAction, String> {
    let staging_dir = create_staging_dir()?;
    extract_archive(&download_path, &staging_dir)?;

    #[cfg(target_os = "windows")]
    {
        spawn_windows_helper(&staging_dir)?;
        return Ok(UpdateInstallAction::Quit);
    }

    #[cfg(target_os = "macos")]
    {
        install_macos(&staging_dir)?;
        return Ok(UpdateInstallAction::Quit);
    }

    #[cfg(target_os = "linux")]
    {
        install_linux(&staging_dir)?;
        return Ok(UpdateInstallAction::Quit);
    }

    #[allow(unreachable_code)]
    Ok(UpdateInstallAction::Noop)
}

pub(super) fn apply_update_helper(source_path: &Path, target_path: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        return apply_update_windows(source_path, target_path);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        return apply_update_unix_with_target(source_path, target_path);
    }

    #[allow(unreachable_code)]
    Ok(())
}

pub(super) fn cleanup_stale_update_backups() {
    #[cfg(target_os = "macos")]
    {
        if let Ok(app_path) = current_app_bundle_path() {
            let _ = remove_dir_all_if_exists(&app_path.with_extension("app.old"));
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        if let Ok(target_path) = std::env::current_exe() {
            let _ = remove_file_if_exists(&target_path.with_extension("old"));
        }
    }
}

fn create_staging_dir() -> Result<PathBuf, String> {
    let root = std::env::temp_dir().join("onetcli-update");
    fs::create_dir_all(&root).map_err(|err| format!("创建更新临时目录失败: {err}"))?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("读取系统时间失败: {err}"))?
        .as_millis();
    let staging_dir = root.join(format!("staged-{}-{now}", std::process::id()));
    remove_dir_all_if_exists(&staging_dir)
        .map_err(|err| format!("清理旧 staging 目录失败: {err}"))?;
    fs::create_dir_all(&staging_dir).map_err(|err| format!("创建 staging 目录失败: {err}"))?;
    Ok(staging_dir)
}

#[cfg(target_os = "windows")]
fn spawn_windows_helper(staging_dir: &Path) -> Result<(), String> {
    let source_path = find_windows_executable(staging_dir)?;
    let target_path =
        std::env::current_exe().map_err(|err| format!("获取当前路径失败: {}", err))?;

    Command::new(&source_path)
        .arg(super::APPLY_UPDATE_FLAG)
        .arg(&source_path)
        .arg(&target_path)
        .spawn()
        .map_err(|err| format!("启动更新进程失败: {}", err))?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn find_windows_executable(staging_dir: &Path) -> Result<PathBuf, String> {
    let direct = staging_dir.join("onetcli.exe");
    if direct.is_file() {
        return Ok(direct);
    }

    find_file_named(staging_dir, "onetcli.exe").ok_or_else(|| "未找到 onetcli.exe".to_string())
}

#[cfg(target_os = "windows")]
fn apply_update_windows(source_path: &Path, target_path: &Path) -> Result<(), String> {
    let backup_path = target_path.with_extension("old");
    let mut last_error = None;

    for _ in 0..120 {
        match replace_target_with_backup(target_path, &backup_path, || {
            replace_via_staging_copy(source_path, target_path)
        }) {
            Ok(()) => {
                let _ = remove_file_if_exists(&backup_path);
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

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn apply_update_unix_with_target(source_path: &Path, target_path: &Path) -> Result<(), String> {
    let backup_path = target_path.with_extension("old");
    replace_target_with_backup(target_path, &backup_path, || {
        try_replace_unix(source_path, target_path)
    })
    .map_err(|err| format!("替换更新文件失败: {}", err))?;

    #[cfg(unix)]
    set_executable_permission(target_path)?;

    restart_application(target_path)?;
    Ok(())
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn try_replace_unix(source_path: &Path, target_path: &Path) -> std::io::Result<()> {
    match fs::rename(source_path, target_path) {
        Ok(()) => Ok(()),
        Err(err) if is_cross_device_link_error(&err) => {
            replace_via_staging_copy(source_path, target_path)
        }
        Err(err) => Err(err),
    }
}

#[cfg(target_os = "macos")]
fn install_macos(staging_dir: &Path) -> Result<(), String> {
    let new_app = find_first_app_bundle(staging_dir)?;
    let current_app = current_app_bundle_path()?;
    let backup_app = current_app.with_extension("app.old");

    remove_dir_all_if_exists(&backup_app).map_err(|err| format!("清理旧备份失败: {err}"))?;
    move_dir(&current_app, &backup_app).map_err(|err| format!("备份当前应用失败: {}", err))?;

    match move_dir(&new_app, &current_app) {
        Ok(()) => {
            clear_quarantine_xattr(&current_app);
            let _ = remove_dir_all_if_exists(&backup_app);
            restart_macos_application(&current_app)?;
            Ok(())
        }
        Err(err) => {
            let _ = move_dir(&backup_app, &current_app);
            Err(format!("安装 macOS 更新失败: {}", err))
        }
    }
}

#[cfg(target_os = "macos")]
fn restart_macos_application(app_path: &Path) -> Result<(), String> {
    Command::new("/usr/bin/open")
        .arg("-n")
        .arg(app_path)
        .spawn()
        .map_err(|err| format!("重启应用失败: {}", err))?;
    Ok(())
}

#[cfg(target_os = "macos")]
fn current_app_bundle_path() -> Result<PathBuf, String> {
    let exe_path = std::env::current_exe().map_err(|err| format!("获取当前路径失败: {err}"))?;
    current_app_bundle_path_from_exe(&exe_path)
}

#[cfg(target_os = "macos")]
fn current_app_bundle_path_from_exe(exe_path: &Path) -> Result<PathBuf, String> {
    let macos_dir = exe_path
        .parent()
        .ok_or_else(|| "当前可执行文件缺少父目录".to_string())?;
    if macos_dir.file_name().and_then(|name| name.to_str()) != Some("MacOS") {
        return Err("当前可执行文件不在 .app/Contents/MacOS 中".to_string());
    }

    let contents_dir = macos_dir
        .parent()
        .ok_or_else(|| "当前可执行文件缺少 Contents 目录".to_string())?;
    if contents_dir.file_name().and_then(|name| name.to_str()) != Some("Contents") {
        return Err("当前可执行文件不在 .app/Contents/MacOS 中".to_string());
    }

    let app_dir = contents_dir
        .parent()
        .ok_or_else(|| "当前可执行文件缺少 .app 目录".to_string())?;
    if app_dir.extension().and_then(|ext| ext.to_str()) != Some("app") {
        return Err("当前可执行文件不在 .app bundle 中".to_string());
    }

    Ok(app_dir.to_path_buf())
}

#[cfg(target_os = "macos")]
fn find_first_app_bundle(staging_dir: &Path) -> Result<PathBuf, String> {
    let mut stack = vec![staging_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|err| format!("读取 staging 目录失败 {}: {}", dir.display(), err))?;

        for entry in entries {
            let entry = entry.map_err(|err| format!("读取 staging 条目失败: {}", err))?;
            let path = entry.path();
            if path.is_dir() {
                if path.extension().and_then(|ext| ext.to_str()) == Some("app") {
                    return Ok(path);
                }
                stack.push(path);
            }
        }
    }

    Err("未找到 OnetCli.app".to_string())
}

#[cfg(target_os = "macos")]
fn clear_quarantine_xattr(app_path: &Path) {
    let _ = Command::new("xattr")
        .arg("-dr")
        .arg("com.apple.quarantine")
        .arg(app_path)
        .spawn();
}

#[cfg(target_os = "macos")]
fn move_dir(source: &Path, destination: &Path) -> std::io::Result<()> {
    match fs::rename(source, destination) {
        Ok(()) => Ok(()),
        Err(err) if is_cross_device_link_error(&err) => {
            copy_dir_recursive(source, destination)?;
            fs::remove_dir_all(source)?;
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(target_os = "macos")]
fn copy_dir_recursive(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path)?;
        } else {
            return Err(std::io::Error::other(format!(
                "不支持复制的 bundle 条目: {}",
                source_path.display()
            )));
        }
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn install_linux(staging_dir: &Path) -> Result<(), String> {
    let new_binary = locate_linux_binary(staging_dir)?;
    let target_path = std::env::current_exe().map_err(|err| format!("获取当前路径失败: {err}"))?;
    let backup_path = target_path.with_extension("old");

    ensure_writable(&target_path)?;
    replace_target_with_backup(&target_path, &backup_path, || {
        replace_via_staging_copy(&new_binary, &target_path)
    })
    .map_err(|err| format!("替换更新文件失败: {}", err))?;
    set_executable_permission(&target_path)?;
    restart_application(&target_path)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn locate_linux_binary(staging_dir: &Path) -> Result<PathBuf, String> {
    let packaged = staging_dir.join("usr/bin/onetcli");
    if packaged.is_file() {
        return Ok(packaged);
    }

    let direct = staging_dir.join("onetcli");
    if direct.is_file() {
        return Ok(direct);
    }

    Err("未找到 Linux 更新二进制 onetcli".to_string())
}

#[cfg(target_os = "linux")]
fn ensure_writable(target_path: &Path) -> Result<(), String> {
    fs::OpenOptions::new()
        .write(true)
        .open(target_path)
        .map(|_| ())
        .map_err(|err| format!("当前安装位置不可写: {}", err))
}

fn replace_target_with_backup(
    target_path: &Path,
    backup_path: &Path,
    replace: impl FnOnce() -> std::io::Result<()>,
) -> std::io::Result<()> {
    remove_file_if_exists(backup_path)?;

    let had_target = target_path.exists();
    if had_target {
        fs::rename(target_path, backup_path)?;
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

    fs::rename(backup_path, target_path)?;
    Ok(())
}

fn replace_via_staging_copy(source_path: &Path, target_path: &Path) -> std::io::Result<()> {
    let staging_path = target_path.with_extension("new");
    remove_file_if_exists(&staging_path)?;

    if let Err(err) = fs::copy(source_path, &staging_path) {
        let _ = remove_file_if_exists(&staging_path);
        return Err(err);
    }

    if let Err(err) = fs::rename(&staging_path, target_path) {
        let _ = remove_file_if_exists(&staging_path);
        return Err(err);
    }

    Ok(())
}

#[cfg(target_os = "windows")]
fn find_file_named(root: &Path, file_name: &str) -> Option<PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).ok()?;
        for entry in entries {
            let entry = entry.ok()?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
                return Some(path);
            }
        }
    }
    None
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn remove_dir_all_if_exists(path: &Path) -> std::io::Result<()> {
    match fs::remove_dir_all(path) {
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

    let mut permissions = fs::metadata(path)
        .map_err(|err| format!("读取文件权限失败: {}", err))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).map_err(|err| format!("设置可执行权限失败: {}", err))?;

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

    #[cfg(target_os = "linux")]
    #[test]
    fn locate_linux_binary_prefers_usr_bin_onetcli() {
        use super::locate_linux_binary;

        let temp_dir = TestDir::new("locate-linux-binary-priority");
        let usr_bin = temp_dir.path.join("usr/bin");
        std::fs::create_dir_all(&usr_bin).expect("创建 usr/bin 失败");
        let preferred = usr_bin.join("onetcli");
        let fallback = temp_dir.path.join("onetcli");
        std::fs::write(&preferred, b"preferred").expect("写入 usr/bin/onetcli 失败");
        std::fs::write(&fallback, b"fallback").expect("写入根目录 onetcli 失败");

        let located = locate_linux_binary(&temp_dir.path).expect("应定位到 Linux 二进制");

        assert_eq!(located, preferred);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn locate_linux_binary_falls_back_to_root_onetcli() {
        use super::locate_linux_binary;

        let temp_dir = TestDir::new("locate-linux-binary-fallback");
        let fallback = temp_dir.path.join("onetcli");
        std::fs::write(&fallback, b"fallback").expect("写入根目录 onetcli 失败");

        let located = locate_linux_binary(&temp_dir.path).expect("应回退到根目录 onetcli");

        assert_eq!(located, fallback);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn current_app_bundle_path_from_exe_returns_app_bundle() {
        use super::current_app_bundle_path_from_exe;

        let exe = PathBuf::from("/Applications/OnetCli.app/Contents/MacOS/onetcli");

        let app = current_app_bundle_path_from_exe(&exe).expect("应能定位 .app bundle");

        assert_eq!(app, PathBuf::from("/Applications/OnetCli.app"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn current_app_bundle_path_from_exe_rejects_non_bundle_path() {
        use super::current_app_bundle_path_from_exe;

        let exe = PathBuf::from("/tmp/onetcli");

        let err = current_app_bundle_path_from_exe(&exe).expect_err("非 .app 路径应失败");

        assert!(
            err.contains(".app"),
            "错误信息应说明 bundle 校验失败: {err}"
        );
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
