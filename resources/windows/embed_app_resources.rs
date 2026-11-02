// Windows 资源段（图标 + 内嵌清单）的共享构建代码。
//
// **必须由二进制 crate 自己的 build.rs 调用**：`cargo:rustc-link-arg` 只作用于
// 本包的 targets，`crates/navop_app` 是库，它的 build.rs 嵌出来的资源到不了
// `apps/navop*/` 里的 exe（产物会缺 `.rsrc`，加载器在 `main` 之前就以
// STATUS_ENTRYPOINT_NOT_FOUND / 0xC0000139 退出）。
//
// 由 `apps/navop/build.rs`、`apps/navop-shell/build.rs`、`apps/navop-data/build.rs`
// 各 include 一次，传入自己的产物名。

/// 嵌入 Common-Controls v6 清单、图标与版本信息。
///
/// gpui 的 Windows 层静态导入 `comctl32!TaskDialogIndirect`，该导出只存在于
/// Common-Controls v6 的 SxS 程序集：只有清单声明了 `Microsoft.Windows.Common-Controls
/// 6.0.0.0` 才会加载 v6，否则挂的是 System32 里的 5.82，进程起不来。
/// 上游 gpui-fast 已经不再无条件嵌清单（`windows-manifest-provided-by-host` 特性由
/// gpui-kit 声明，特性并集无法关闭），所以清单只能由产品自己提供；更新 helper
/// 又是单独跑 staging 目录里的 exe，同目录不会有外挂 `.manifest`，必须是内嵌清单。
pub fn embed_windows_resources(icon: &str, product_name: &str, file_description: &str) {
    #[cfg(target_os = "windows")]
    {
        const VFT_APP: u64 = 1;

        let mut res = winresource::WindowsResource::new();
        res.set_icon(icon);
        // winresource 只在 `VersionInfo::FILETYPE` 存在时才会写清单块
        // （见其 `write_resource_file`），因此下面这行不是可选项：漏了会静默不嵌清单。
        res.set_version_info(winresource::VersionInfo::FILETYPE, VFT_APP);
        res.set_manifest(include_str!("navop.manifest.xml"));
        println!("cargo:rerun-if-changed={icon}");
        println!("cargo:rerun-if-changed=../../resources/windows/navop.manifest.xml");

        res.set("ProductName", product_name);
        res.set("FileDescription", file_description);
        res.set("LegalCopyright", "Copyright (c) 2025 Navop");
        res.compile().expect("Failed to compile Windows resources");
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (icon, product_name, file_description);
    }
}
