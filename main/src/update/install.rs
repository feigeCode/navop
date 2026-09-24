use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::extract::extract_archive;
use super::util::UpdateInstallAction;

#[cfg(target_os = "macos")]
const CURRENT_APP_BUNDLE_NAME: &str = "Navop.app";
#[cfg(target_os = "macos")]
const LEGACY_APP_BUNDLE_NAME: &str = "OnetCli.app";

/// 等旧版本进程释放可执行文件的上限。
///
/// 更新替换可以**在旧进程仍然存活时**完成（Windows 允许重命名正在运行的 exe），
/// 所以替换成功并不代表旧实例已经退出。见 `wait_for_previous_instance_exit`。
///
/// 取 10 秒：正常退出是亚秒级的，到这个量级基本可以判定旧实例卡住了；再等下去只会拉长
/// "更新完成后桌面上什么都没有"的空窗期。超时不再盲重启，见 `restart_after_replacement`。
#[cfg(target_os = "windows")]
const PREVIOUS_INSTANCE_EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(target_os = "windows")]
const PREVIOUS_INSTANCE_EXIT_POLL_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(200);

/// 文件仍被占用时 Windows 返回的错误码。
///
/// 必须按**原始错误码**判断，不能按 `ErrorKind`：std 把 5 映射成 `PermissionDenied`，
/// 却把 32 映射成 `Uncategorized`（实测 `fs::remove_file` 在两种占用下分别返回
/// `raw_os_error()==Some(5)` / `Some(32)`）。这与单实例模块按原始码分类的理由相同。
#[cfg(target_os = "windows")]
const ERROR_ACCESS_DENIED: i32 = 5;
/// 句柄未共享 `FILE_SHARE_DELETE` 时的占用错误码。
#[cfg(target_os = "windows")]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// 备份是否仍被某个进程占用（因此还不能拉起新版本）。
///
/// - `ERROR_ACCESS_DENIED(5)`：被映射为运行中映像的 exe，实测 `DeleteFileW` 返回 5，
///   进程退出后转为 0 —— 这正是"旧版本实例是否还活着"的探针。
/// - `ERROR_SHARING_VIOLATION(32)`：句柄存在但未共享删除权限（安全软件扫描等）。
///   同属"现在别重启"，等其释放即可。
#[cfg(target_os = "windows")]
fn file_is_still_in_use(error: &std::io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(ERROR_ACCESS_DENIED) | Some(ERROR_SHARING_VIOLATION)
    )
}

/// `wait_for_previous_instance_exit` 的结论。
#[cfg(target_os = "windows")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PreviousInstanceExit {
    /// 备份已可删除：旧实例确实退出了，拉起新版本是安全的。
    Released,
    /// 超时或探测失败：**无法确认**旧实例已退出，此时绝不能拉起新版本。
    Unconfirmed,
}

pub(crate) fn start_install_update(download_path: PathBuf) -> Result<UpdateInstallAction, String> {
    if one_core::app_paths::is_portable() {
        return Err("便携模式不支持应用内安装更新，请下载新的便携版并保留 data 目录".to_string());
    }
    if !download_path.is_file() {
        return Err(format!("更新归档不存在: {}", download_path.display()));
    }

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
    let root = std::env::temp_dir().join("navop-update");
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
    for name in ["navop.exe", "onetcli.exe"] {
        let direct = staging_dir.join(name);
        if direct.is_file() {
            return Ok(direct);
        }
        if let Some(path) = find_file_named(staging_dir, name) {
            return Ok(path);
        }
    }

    Err("未找到 navop.exe 或兼容的 onetcli.exe".to_string())
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
                // 必须先确认旧实例已退出，再拉起新版本：替换成功并不代表旧进程
                // 已经消失，而新版本一旦在旧实例仍持有单实例管道时启动，就会走
                // "转发给已有实例"分支后立刻退出，表现为"更新完成后应用不再出现"。
                restart_after_replacement(&backup_path, target_path)?;
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

/// 替换成功后的收尾：确认旧实例已退出才拉起新版本。
///
/// `Unconfirmed` 说明旧实例可能还占着单实例管道，这时拉起新版本，新版本要么把启动请求
/// 转发给它然后自己退出（用户什么也看不到），要么直接弹"启动请求没能交给它"。两种结果
/// 都比"让用户手动启动一次"更糟，所以超时不再盲重启，只弹框说清楚。
#[cfg(target_os = "windows")]
fn restart_after_replacement(backup_path: &Path, target_path: &Path) -> Result<(), String> {
    if wait_for_previous_instance_exit(backup_path, PREVIOUS_INSTANCE_EXIT_TIMEOUT)
        == PreviousInstanceExit::Unconfirmed
    {
        report_previous_instance_still_running();
        return Err(format!(
            "旧版本实例没有在 {} 秒内退出，新版本文件已就位，请结束 Navop 进程后手动启动",
            PREVIOUS_INSTANCE_EXIT_TIMEOUT.as_secs()
        ));
    }

    restart_application(target_path)
}

/// 等不到旧实例退出时的用户可见提示。
///
/// 更新 helper 没有自己的窗口，release 构建又是 `windows_subsystem = "windows"`（没有
/// 控制台），`eprintln!` 用户看不到：这里必须弹系统对话框，否则这次更新的结果就只剩
/// "桌面上什么都没出现"。
#[cfg(target_os = "windows")]
fn report_previous_instance_still_running() {
    use windows::Win32::UI::WindowsAndMessaging::{
        MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MessageBoxW,
    };
    use windows::core::HSTRING;

    let text = HSTRING::from(
        "更新已经完成，但旧版本的 Navop 进程还没有退出。\n\n请在任务管理器中结束 Navop 进程，然后重新启动，即可使用新版本。",
    );
    let caption = HSTRING::from(crate::NAVOP_WINDOW_TITLE);
    // SAFETY: 两个字符串在调用期间保持存活；更新 helper 没有属主窗口，句柄传 None。
    unsafe {
        MessageBoxW(
            None,
            &text,
            &caption,
            MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND,
        );
    }
}

/// 等到被替换掉的旧版本进程真正退出，再让调用方拉起新版本。
///
/// 更新替换过程可以在旧进程**仍然存活**时走完：Windows 允许重命名正在运行的
/// exe（实测 `MoveFileW` 返回 0），于是 `rename` + 拷贝新文件都能成功，而旧进程
/// 直到 `restart_application` 之后才真正消失。旧进程此刻仍占着 Windows 单实例
/// 管道，新版本一旦在此时启动就会走进"转发给已有实例"分支、拿到确认后立刻退出
/// —— 用户看到的是"更新完成后应用不再出现"。
///
/// 运行中的 exe 无法被删除（实测 `DeleteFileW` 返回 `ERROR_ACCESS_DENIED(5)`），
/// 因此"备份文件变成可删除"就是旧进程已退出的可靠探针。超时或探测失败都返回
/// `Unconfirmed`：**不能**把"等不到"当成"可以重启"，否则又回到上面那条链路。
#[cfg(target_os = "windows")]
fn wait_for_previous_instance_exit(
    backup_path: &Path,
    timeout: std::time::Duration,
) -> PreviousInstanceExit {
    if !backup_path.exists() {
        // 没有旧文件可删（首次安装，或备份已被清理）：没有任何东西需要等待。
        return PreviousInstanceExit::Released;
    }

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match remove_file_if_exists(backup_path) {
            // 备份转为可删除 = 旧进程已退出，管道名已经空闲。
            Ok(()) => return PreviousInstanceExit::Released,
            Err(err) if !file_is_still_in_use(&err) => {
                eprintln!("清理更新备份失败，无法确认旧实例已退出: {err}");
                return PreviousInstanceExit::Unconfirmed;
            }
            Err(_) if std::time::Instant::now() >= deadline => {
                eprintln!(
                    "旧版本进程在 {:?} 内未释放 {}",
                    timeout,
                    backup_path.display()
                );
                return PreviousInstanceExit::Unconfirmed;
            }
            Err(_) => std::thread::sleep(PREVIOUS_INSTANCE_EXIT_POLL_INTERVAL),
        }
    }
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
                if is_supported_app_bundle(&path) {
                    return Ok(path);
                }
                stack.push(path);
            }
        }
    }

    Err("未找到 Navop.app 或 OnetCli.app".to_string())
}

#[cfg(target_os = "macos")]
fn is_supported_app_bundle(path: &Path) -> bool {
    let name = path.file_name().and_then(|name| name.to_str());
    name == Some(CURRENT_APP_BUNDLE_NAME) || name == Some(LEGACY_APP_BUNDLE_NAME)
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
    for relative in ["usr/bin/navop", "navop", "usr/bin/onetcli", "onetcli"] {
        let candidate = staging_dir.join(relative);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err("未找到 Linux 更新二进制 navop 或兼容的 onetcli".to_string())
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

    use super::{replace_target_with_backup, start_install_update};

    #[test]
    fn start_install_update_rejects_missing_archive_before_staging() {
        let temp_dir = TestDir::new("missing-update-archive");
        let missing_archive = temp_dir.path.join("navop-update.tar.gz");

        let err = match start_install_update(missing_archive) {
            Err(err) => err,
            Ok(_) => panic!("缺失归档应直接失败"),
        };

        assert!(
            err.contains("更新归档不存在"),
            "错误应明确说明文件缺失: {err}"
        );
    }

    #[test]
    fn replace_target_with_backup_rolls_back_on_replace_error() {
        let temp_dir = TestDir::new("replace-target-with-backup");
        let target_path = temp_dir.path.join("navop");
        let backup_path = temp_dir.path.join("navop.old");
        std::fs::write(&target_path, b"old-binary").expect("写入旧版本失败");

        let result = replace_target_with_backup(&target_path, &backup_path, || {
            Err(std::io::Error::other("模拟替换失败"))
        });

        assert!(result.is_err());
        let target_bytes = std::fs::read(&target_path).expect("回滚后旧版本应仍存在");
        assert_eq!(target_bytes, b"old-binary");
        assert!(!backup_path.exists());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn restart_waits_until_the_previous_instance_releases_the_executable() {
        use super::PreviousInstanceExit;
        use super::wait_for_previous_instance_exit;
        use std::fs;
        use std::os::windows::fs::OpenOptionsExt;
        use std::time::Duration;

        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;

        let temp_dir = TestDir::new("wait-previous-instance");
        let backup_path = temp_dir.path.join("navop.old");
        fs::write(&backup_path, b"old-binary").expect("写入备份失败");

        // 模拟"旧 exe 仍在运行"：被映射为映像的 exe 会拒绝删除（实测 `DeleteFileW`
        // 返回 ERROR_ACCESS_DENIED=5）。注意 std 的 `File::open` 默认共享
        // `FILE_SHARE_DELETE`，那样是锁不住的（实测此时删除会成功），必须显式去掉它
        // ——实测该句柄下删除返回 ERROR_SHARING_VIOLATION=32，与"仍被占用"同属一类。
        let occupied = fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&backup_path)
            .expect("打开备份失败");

        assert_eq!(
            wait_for_previous_instance_exit(&backup_path, Duration::from_millis(500)),
            PreviousInstanceExit::Unconfirmed,
            "等不到旧实例退出时必须报 Unconfirmed，调用方据此拒绝重启"
        );
        assert!(
            backup_path.exists(),
            "旧实例尚未退出时不能放行重启，否则新版本会转发给旧实例后立刻退出"
        );

        drop(occupied);
        assert_eq!(
            wait_for_previous_instance_exit(&backup_path, Duration::from_millis(500)),
            PreviousInstanceExit::Released,
            "旧实例退出后必须放行重启"
        );
        assert!(
            !backup_path.exists(),
            "旧实例退出后必须立刻放行重启，并把备份清理掉"
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn restart_does_not_wait_when_there_is_no_previous_executable() {
        use super::PREVIOUS_INSTANCE_EXIT_TIMEOUT;
        use super::PreviousInstanceExit;
        use super::wait_for_previous_instance_exit;
        use std::time::{Duration, Instant};

        let temp_dir = TestDir::new("no-previous-instance");
        let backup_path = temp_dir.path.join("navop.old");

        let started = Instant::now();
        assert_eq!(
            wait_for_previous_instance_exit(&backup_path, PREVIOUS_INSTANCE_EXIT_TIMEOUT),
            PreviousInstanceExit::Released,
            "没有旧可执行文件时必须直接放行重启"
        );

        assert!(
            started.elapsed() < Duration::from_secs(1),
            "没有旧可执行文件时不应等待，实际耗时 {:?}",
            started.elapsed()
        );
    }

    /// 超时（`Unconfirmed`）绝不允许走到 `restart_application`。
    ///
    /// 用源文本钉住分支形态：这条分支只有在旧实例卡住时才会走到，而 Windows 侧代码在
    /// macOS 上不参与编译，所以只能这样把它锁住——"超时仍然继续重启"正是这次要改掉的旧行为。
    #[test]
    fn restart_is_refused_when_the_previous_instance_has_not_exited() {
        let source = include_str!("install.rs").replace("\r\n", "\n");
        let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
        let function = compact
            .find("fnrestart_after_replacement")
            .expect("restart_after_replacement");
        let body = &compact[function..];
        let unconfirmed = body
            .find("==PreviousInstanceExit::Unconfirmed")
            .expect("超时必须判定为 Unconfirmed");
        let report = body
            .find("report_previous_instance_still_running()")
            .expect("超时必须给用户可见提示");
        let restart = body
            .find("restart_application(target_path)")
            .expect("确认旧实例退出后才允许拉起新版本");

        assert!(
            unconfirmed < report && report < restart,
            "超时必须在重启之前提示；不得在未确认旧实例退出时直接拉起新版本"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn apply_update_unix_keeps_old_target_when_replace_fails() {
        use super::apply_update_unix_with_target;

        let temp_dir = TestDir::new("apply-update-unix");
        let target_path = temp_dir.path.join("navop");
        let missing_download_path = temp_dir.path.join("missing-download");
        std::fs::write(&target_path, b"old-binary").expect("写入旧版本失败");

        let result = apply_update_unix_with_target(&missing_download_path, &target_path);

        assert!(result.is_err());
        let target_bytes = std::fs::read(&target_path).expect("替换失败后旧版本应仍存在");
        assert_eq!(target_bytes, b"old-binary");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn locate_linux_binary_prefers_usr_bin_navop() {
        use super::locate_linux_binary;

        let temp_dir = TestDir::new("locate-linux-binary-priority");
        let usr_bin = temp_dir.path.join("usr/bin");
        std::fs::create_dir_all(&usr_bin).expect("创建 usr/bin 失败");
        let preferred = usr_bin.join("navop");
        let fallback = temp_dir.path.join("navop");
        std::fs::write(&preferred, b"preferred").expect("写入 usr/bin/navop 失败");
        std::fs::write(&fallback, b"fallback").expect("写入根目录 navop 失败");

        let located = locate_linux_binary(&temp_dir.path).expect("应定位到 Linux 二进制");

        assert_eq!(located, preferred);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn locate_linux_binary_accepts_legacy_root_onetcli() {
        use super::locate_linux_binary;

        let temp_dir = TestDir::new("locate-linux-binary-fallback");
        let fallback = temp_dir.path.join("onetcli");
        std::fs::write(&fallback, b"fallback").expect("写入根目录 onetcli 失败");

        let located = locate_linux_binary(&temp_dir.path).expect("应兼容根目录 onetcli");

        assert_eq!(located, fallback);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn current_app_bundle_path_from_exe_returns_app_bundle() {
        use super::current_app_bundle_path_from_exe;

        let exe = PathBuf::from("/Applications/Navop.app/Contents/MacOS/navop");

        let app = current_app_bundle_path_from_exe(&exe).expect("应能定位 .app bundle");

        assert_eq!(app, PathBuf::from("/Applications/Navop.app"));
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

    #[cfg(target_os = "macos")]
    #[test]
    fn find_app_bundle_accepts_current_and_legacy_names() {
        use super::find_first_app_bundle;

        for name in ["Navop.app", "OnetCli.app"] {
            let temp = TestDir::new("find-app-bundle");
            let app = temp.path.join(name);
            std::fs::create_dir_all(&app).expect("创建 app bundle 失败");

            assert_eq!(
                find_first_app_bundle(&temp.path).expect("应接受当前或旧版 app bundle"),
                app
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn find_app_bundle_rejects_unsupported_name() {
        use super::find_first_app_bundle;

        let temp = TestDir::new("find-app-bundle-unsupported");
        std::fs::create_dir_all(temp.path.join("Other.app")).expect("创建 app bundle 失败");

        let err = find_first_app_bundle(&temp.path).expect_err("应拒绝未知 app bundle");

        assert!(err.contains("Navop.app"), "错误应包含当前 bundle 名: {err}");
        assert!(
            err.contains("OnetCli.app"),
            "错误应包含旧版 bundle 名: {err}"
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
