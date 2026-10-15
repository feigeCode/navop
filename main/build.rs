fn main() {
    #[cfg(target_os = "windows")]
    {
        const VFT_APP: u64 = 1;

        let mut res = winresource::WindowsResource::new();
        res.set_icon("../resources/windows/navop.ico");

        // Common-Controls v6 清单**必须由宿主自己嵌**，缺了它 navop.exe 在 Windows 上根本
        // 起不来（加载器在 `main` 之前就以 STATUS_ENTRYPOINT_NOT_FOUND / 0xC0000139 退出，
        // 弹「无法定位程序输入点 TaskDialogIndirect 于动态链接库 …navop.exe 上」）：
        //
        // - gpui 的 Windows 层静态导入 `comctl32!TaskDialogIndirect`（它在 gpui_windows 的
        //   `PlatformWindow::prompt` 里，navop 多处 `window.prompt(PromptLevel::…)` 会用到），
        //   而该导出只存在于 Common-Controls v6 的 SxS 程序集：只有清单声明了
        //   `Microsoft.Windows.Common-Controls 6.0.0.0` 才会加载 v6，否则挂的是 System32 里
        //   的 5.82，没有这个导出。
        // - 上游 `gpui-fast` 的 build.rs 已经不无条件嵌清单了：它是
        //   `embed_resource_unless_host_provides_it()`，一旦启用 `windows-manifest-provided-by-host`
        //   就直接返回；而 gpui-kit（gpui-base/gpui-component/gpui-shell/gpui-fps/gpui-kit-assets）
        //   声明了该特性。Cargo 特性只做并集、下游无法关闭 ⇒ 这份清单只能由 navop 提供。
        // - 更新 helper 是**单独跑 staging 目录里的 navop.exe**，同目录不会有外部
        //   `navop.exe.manifest`，所以必须是内嵌清单而不是外挂文件。
        //
        // winresource 只在 `VersionInfo::FILETYPE` 存在时才会写清单块（见其
        // `write_resource_file`），因此下面这行不是可选项：漏了会静默不嵌清单。
        res.set_version_info(winresource::VersionInfo::FILETYPE, VFT_APP);
        res.set_manifest(include_str!("../resources/windows/navop.manifest.xml"));
        println!("cargo:rerun-if-changed=../resources/windows/navop.manifest.xml");

        res.set("ProductName", "Navop");
        res.set("FileDescription", "Navop - Database and Remote Operations");
        res.set("LegalCopyright", "Copyright (c) 2025 Navop");
        res.compile().expect("Failed to compile Windows resources");
    }
}
