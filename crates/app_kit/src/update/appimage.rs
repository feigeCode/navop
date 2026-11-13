use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// 当前进程是否由 AppImage runtime 启动，并返回应被替换的原始镜像路径。
pub(crate) fn runtime_image_path() -> Option<PathBuf> {
    runtime_image_path_from_parts(
        std::env::var_os("APPIMAGE"),
        std::env::var_os("APPDIR"),
        &std::env::current_exe().ok()?,
    )
}

pub(crate) fn runtime_image_path_from_parts(
    appimage: Option<OsString>,
    appdir: Option<OsString>,
    current_exe: &Path,
) -> Option<PathBuf> {
    let image_path = PathBuf::from(appimage?);
    let appdir = PathBuf::from(appdir?);

    // AppImage runtime 会导出这两个变量；只有当前进程确实从 APPDIR 挂载树中启动时，
    // APPIMAGE 才能作为可信的替换目标。普通安装误带 APPIMAGE 环境变量时不得命中。
    if !image_path.is_absolute()
        || !appdir.is_absolute()
        || !current_exe.starts_with(&appdir)
        || !image_path.is_file()
    {
        return None;
    }

    Some(image_path)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::runtime_image_path_from_parts;

    #[test]
    fn appimage_runtime_uses_the_original_image_not_the_mount_binary() {
        let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
        let appdir = temp_dir.path().join(".mount_Navop");
        let mounted_exe = appdir.join("usr/bin/navop");
        let appimage = temp_dir.path().join("Navop.AppImage");
        std::fs::create_dir_all(mounted_exe.parent().expect("挂载二进制缺少父目录"))
            .expect("创建挂载目录失败");
        std::fs::write(&mounted_exe, b"mounted binary").expect("写入挂载二进制失败");
        std::fs::write(&appimage, b"appimage").expect("写入 AppImage 失败");

        assert_eq!(
            Some(appimage.clone()),
            runtime_image_path_from_parts(
                Some(OsString::from(&appimage)),
                Some(OsString::from(&appdir)),
                &mounted_exe,
            )
        );
    }

    #[test]
    fn regular_installed_process_is_not_mistaken_for_appimage() {
        let temp_dir = tempfile::TempDir::new().expect("创建临时目录失败");
        let appimage = temp_dir.path().join("Navop.AppImage");
        let installed_exe = temp_dir.path().join("usr/bin/navop");
        std::fs::create_dir_all(installed_exe.parent().expect("安装二进制缺少父目录"))
            .expect("创建安装目录失败");
        std::fs::write(&appimage, b"appimage").expect("写入 AppImage 失败");
        std::fs::write(&installed_exe, b"installed binary").expect("写入安装二进制失败");

        assert_eq!(
            None,
            runtime_image_path_from_parts(
                Some(OsString::from(appimage)),
                Some(OsString::from(temp_dir.path().join(".mount_Other"))),
                &installed_exe,
            )
        );
    }
}
