//! 开发者工具的 dev 工程注册表。
//!
//! dev 工程 = 任意含 extension.json 的目录。加载时解析 manifest,与
//! 正式安装的 composite manifests 合并后整体重建
//! `GlobalExtensionRuntimeCatalog`。manifest.id 加 `dev.` 前缀,保证
//! view_key/runtime_key 与正式安装隔离。开发贡献由全局 catalog 保留，
//! 常规安装刷新也不会丢失；同 id 的两个开发工程会显示冲突。
//!
//! 不复制工程文件:entry/图标全部从工程目录解析。移除 dev 工程只重建
//! catalog,不动磁盘。

use gpui::{AppContext as _, BorrowAppContext as _};

use std::path::PathBuf;
use std::sync::{Arc, atomic::AtomicBool};

use extension_runtime::GlobalExtensionRuntimeCatalog;
use extension_runtime::extension::manifest::{Manifest, load_from_dir};

/// dev 工程扩展 id 前缀(同时用于 view_key 隔离)。
pub const DEV_ID_PREFIX: &str = "dev.";

#[derive(Debug, Clone)]
pub struct DevProject {
    /// 工程根目录(含 extension.json)。
    pub root: PathBuf,
    /// 解析后的 manifest(id 已加 dev 前缀);解析失败为 None。
    pub manifest: Option<Manifest>,
    /// 解析/校验错误(失败项目保留在列表里供展示)。
    pub error: Option<String>,
}

#[derive(Default)]
pub struct DevExtensionRegistry {
    projects: Vec<DevProject>,
}

impl gpui::Global for DevExtensionRegistry {}

/// dev 面板操作日志环(按 root 记;上限 1000,超限丢最旧)。
#[derive(Default)]
pub struct DevLogRing(std::cell::RefCell<std::collections::VecDeque<String>>);

impl DevLogRing {
    const MAX: usize = 1000;
    pub fn push(&self, root: &std::path::Path, line: String) {
        let mut queue = self.0.borrow_mut();
        if queue.len() >= Self::MAX {
            queue.pop_front();
        }
        let root = root.display().to_string();
        queue.push_back(format!("[{root}] {line}"));
    }
    /// 返回某 root 的日志尾部(倒序新→旧)。
    pub fn tail(&self, root: &std::path::Path, limit: usize) -> Vec<String> {
        let root_str = root.display().to_string();
        let prefix = format!("[{root_str}] ");
        self.0
            .borrow()
            .iter()
            .filter(|line| line.starts_with(&prefix))
            .rev()
            .take(limit)
            .map(|line| line[prefix.len()..].to_string())
            .collect()
    }
}

impl gpui::Global for DevLogRing {}

pub(crate) fn push_log(cx: &gpui::App, root: &std::path::Path, message: impl Into<String>) {
    if let Some(ring) = cx.try_global::<DevLogRing>() {
        ring.push(root, message.into());
    }
}

impl DevExtensionRegistry {
    pub fn global(cx: &gpui::App) -> Option<&Self> {
        cx.try_global::<Self>()
    }

    pub fn projects(&self) -> &[DevProject] {
        &self.projects
    }

    /// 加载/重载一个工程目录。解析或注册失败也保留项目供修复重试。
    pub fn load(&mut self, root: PathBuf, cx: &mut gpui::App) {
        self.projects.retain(|project| project.root != root);
        let project = match load_dev_manifest(&root) {
            Ok(manifest) => DevProject {
                root,
                manifest: Some(manifest),
                error: None,
            },
            Err(error) => DevProject {
                root,
                manifest: None,
                error: Some(error),
            },
        };
        self.projects.push(project);
        self.rebuild_catalog(cx);
    }

    pub fn remove(&mut self, root: &std::path::Path, cx: &mut gpui::App) {
        self.projects.retain(|project| project.root != root);
        self.rebuild_catalog(cx);
    }

    pub fn project(&self, root: &std::path::Path) -> Option<&DevProject> {
        self.projects.iter().find(|project| project.root == root)
    }

    /// 正式安装 manifests ∪ dev manifests → 整体重建 global catalog。
    fn rebuild_catalog(&mut self, cx: &mut gpui::App) {
        let mut manifests = Vec::new();
        let mut seen = std::collections::HashMap::<String, PathBuf>::new();
        for project in &mut self.projects {
            let Some(manifest) = &project.manifest else {
                continue;
            };
            project.error = None;
            if let Some(first_root) = seen.get(&manifest.id) {
                project.error = Some(format!(
                    "开发扩展 ID 冲突：{} 已由 {} 加载",
                    strip_dev_prefix(&manifest.id),
                    first_root.display()
                ));
                continue;
            }
            seen.insert(manifest.id.clone(), project.root.clone());
            manifests.push(manifest.clone());
        }
        let result = cx.update_default_global::<GlobalExtensionRuntimeCatalog, _>(|global, _| {
            global.set_development_manifests(manifests)
        });
        match result {
            Ok(report) => {
                for failure in report.failures {
                    if let Some(project) = self
                        .projects
                        .iter_mut()
                        .find(|project| project.root == failure.root)
                    {
                        project.error = Some(format!("扩展注册失败：{}", failure.error));
                    }
                }
            }
            Err(error) => {
                tracing::warn!(%error, "dev extension catalog rebuild failed");
                for project in &mut self.projects {
                    if project.error.is_none() {
                        project.error =
                            Some(format!("扩展目录刷新失败，仍在使用上一次有效版本：{error}"));
                    }
                }
            }
        }
    }
}

/// 解析工程 manifest 并加 dev 前缀。
fn load_dev_manifest(root: &std::path::Path) -> Result<Manifest, String> {
    let mut manifest = load_from_dir(root).map_err(|error| error.to_string())?;
    manifest.id = format!("{DEV_ID_PREFIX}{}", manifest.id);
    Ok(manifest)
}

// ---------------------------------------------------------------------------
// navop.dev host 操作集实现
// ---------------------------------------------------------------------------

/// 构造 main 层 DevHostOps,注入到 universal-plugins Global。
#[cfg(feature = "nsh-host")]
pub fn install_dev_host_ops(cx: &mut gpui::App) {
    cx.default_global::<DevExtensionRegistry>();
    cx.default_global::<DevLogRing>();
    cx.default_global::<DevWatchSet>();
    cx.default_global::<DevPickState>();
    // 操作集本身在 `dev_host_ops`：每个函数都在主线程上跑，参数与返回值
    // 都是 JSON，不再依赖 gpui-shell 的 HostValue/with_current_app。
    crate::dev_host_ops::install(cx);
}

#[cfg(feature = "nsh-host")]
pub(crate) fn close_dev_extension(extension_id: String, cx: &mut gpui::App) {
    let Some(window) = crate::app_init::resolve_navop_window(cx) else {
        extension_view::finish_shell_extension(&extension_id, cx);
        return;
    };
    let close_task = match cx.update_window(window, |_, window, cx| {
        extension_view::close_shell_extension(&extension_id, window, cx)
    }) {
        Ok(task) => task,
        Err(error) => {
            tracing::warn!(%error, %extension_id, "close dev extension view failed");
            extension_view::finish_shell_extension(&extension_id, cx);
            return;
        }
    };
    cx.spawn(async move |cx| {
        let closed = close_task.await;
        let _ = cx.update(|cx| {
            extension_view::finish_shell_extension(&extension_id, cx);
            if !closed {
                tracing::warn!(%extension_id, "dev extension view refused to close");
            }
        });
    })
    .detach();
}

pub(crate) fn strip_dev_prefix(id: &str) -> String {
    id.strip_prefix(DEV_ID_PREFIX).unwrap_or(id).to_string()
}

/// 工程目录 source 指纹:recursive scan ui/ + entry,按 (相对路径, mt_s) 排序。
/// dev 视图少、文件小,轮询成本可忽略。
pub(crate) fn source_fingerprint(root: &std::path::Path) -> Option<String> {
    let entry_rel = {
        let manifest = load_from_dir(root).ok()?;
        manifest.contributes.shell_views.first()?.entry.clone()
    };
    let mut hashes = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for item in entries.flatten() {
            let path = item.path();
            if path.is_dir() {
                if is_ignored_watch_dir(&path) {
                    continue;
                }
                dirs.push(path);
                continue;
            }
            let rel = path.strip_prefix(root).ok()?.to_string_lossy().to_string();
            // 只指纹 ui/、entry、manifest 与构建元数据。
            let interesting = rel.starts_with("ui/")
                || rel == entry_rel
                || rel.ends_with(".json")
                || rel.ends_with(".js");
            if !interesting {
                continue;
            }
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            let ms = modified
                .duration_since(std::time::UNIX_EPOCH)
                .ok()?
                .as_millis();
            hashes.push(format!("{rel}:{ms}"));
        }
    }
    hashes.sort();
    Some(hashes.join(","))
}

fn is_ignored_watch_dir(path: &std::path::Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(".git" | "node_modules" | "target")
    )
}

/// 按工程目录的变更轮询 watch 任务表(root → (last_sig, task))。
/// 自动重载已开启的工程。
#[derive(Default)]
pub struct DevWatchSet(
    pub(crate)  std::cell::RefCell<
        std::collections::HashMap<std::path::PathBuf, (String, gpui::Task<()>, Arc<AtomicBool>)>,
    >,
);
impl gpui::Global for DevWatchSet {}

/// 原生目录选择结果槽:pick_start 清空并起 task 等 modal,pick_result 读取。
#[derive(Default)]
pub struct DevPickState(pub(crate) std::cell::RefCell<Option<String>>);
impl gpui::Global for DevPickState {}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_manifest(root: &std::path::Path, id: &str, surface: &str) {
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::write(root.join("ui/tool.js"), "export default class V {}").unwrap();
        std::fs::write(
            root.join("extension.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "schema_version": 1,
                "id": id,
                "name": id,
                "version": "0.1.0",
                "engines": { "onetcli": ">=0.1.0", "nsh": "0.3.0" },
                "api": { "shell": "1.0" },
                "permissions": ["shell:exec"],
                "contributes": { "shellViews": [{
                    "id": "view",
                    "title": "View",
                    "entry": "ui/tool.js",
                    "surface": surface,
                    "singleton": true,
                    "modules": ["log"]
                }]}
            }))
            .unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn load_adds_dev_prefix_and_keeps_root() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("my-tool");
        std::fs::create_dir_all(&root).unwrap();
        write_manifest(&root, "com.example.my-tool", "tab");
        let manifest = load_dev_manifest(&root).expect("manifest loads");
        assert_eq!(manifest.id, "dev.com.example.my-tool");
        assert_eq!(strip_dev_prefix(&manifest.id), "com.example.my-tool");
        assert_eq!(manifest.manifest_dir, root);
        assert_eq!(manifest.contributes.shell_views[0].entry, "ui/tool.js");
    }

    #[test]
    fn load_reports_missing_extension_json() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("empty");
        std::fs::create_dir_all(&root).unwrap();
        let error = load_dev_manifest(&root).unwrap_err();
        assert!(!error.is_empty());
    }

    #[gpui::test]
    fn loading_dev_project_registers_shell_views_without_existing_catalog(
        cx: &mut gpui::TestAppContext,
    ) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("tool");
        std::fs::create_dir_all(&root).unwrap();
        write_manifest(&root, "com.example.tool", "tab");

        cx.update(|cx| {
            assert!(cx.try_global::<GlobalExtensionRuntimeCatalog>().is_none());
            let mut registry = DevExtensionRegistry::default();
            registry.load(root, cx);

            let catalog = cx
                .global::<GlobalExtensionRuntimeCatalog>()
                .get()
                .expect("dev load should initialize the runtime catalog");
            assert!(catalog.shell_view("dev.com.example.tool", "view").is_some());
        });
    }

    #[gpui::test]
    fn installed_catalog_refresh_keeps_development_views(cx: &mut gpui::TestAppContext) {
        let tmp = tempfile::TempDir::new().unwrap();
        write_manifest(tmp.path(), "com.example.refresh", "toolbox");
        cx.update(|cx| {
            let mut registry = DevExtensionRegistry::default();
            registry.load(tmp.path().to_path_buf(), cx);
            extension_runtime::refresh_global_runtime_catalog(cx);
            let catalog = extension_runtime::global_catalog(cx).unwrap();
            assert!(
                catalog
                    .shell_view("dev.com.example.refresh", "view")
                    .is_some()
            );
            registry.remove(tmp.path(), cx);
            extension_runtime::refresh_global_runtime_catalog(cx);
            assert!(
                extension_runtime::global_catalog(cx)
                    .unwrap()
                    .shell_view("dev.com.example.refresh", "view")
                    .is_none()
            );
        });
    }

    #[gpui::test]
    fn duplicate_extension_id_reports_error_instead_of_silently_skipping(
        cx: &mut gpui::TestAppContext,
    ) {
        let tmp = tempfile::TempDir::new().unwrap();
        let first = tmp.path().join("first");
        let second = tmp.path().join("second");
        write_manifest(&first, "com.example.duplicate", "tab");
        write_manifest(&second, "com.example.duplicate", "tab");
        cx.update(|cx| {
            let mut registry = DevExtensionRegistry::default();
            registry.load(first.clone(), cx);
            registry.load(second.clone(), cx);
            assert!(registry.projects()[0].error.is_none());
            assert!(
                registry.projects()[1]
                    .error
                    .as_deref()
                    .unwrap()
                    .contains("duplicate")
            );
            registry.remove(&first, cx);
            assert!(registry.projects()[0].error.is_none());
            let catalog = extension_runtime::global_catalog(cx).unwrap();
            assert_eq!(
                catalog
                    .shell_view("dev.com.example.duplicate", "view")
                    .unwrap()
                    .entry_path,
                second.join("ui/tool.js")
            );
        });
    }

    #[gpui::test]
    fn duplicate_load_replaces_previous_entry(cx: &mut gpui::TestAppContext) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("tool");
        std::fs::create_dir_all(&root).unwrap();
        write_manifest(&root, "com.example.tool", "tab");

        cx.update(|cx| {
            let mut registry = DevExtensionRegistry::default();
            registry.load(root.clone(), cx);
            registry.load(root.clone(), cx);
            assert_eq!(registry.projects().len(), 1);
            assert_eq!(
                registry.projects()[0].manifest.as_ref().unwrap().id,
                "dev.com.example.tool"
            );

            let other = tmp.path().join("other");
            std::fs::create_dir_all(&other).unwrap();
            write_manifest(&other, "com.example.other", "tab");
            registry.load(other.clone(), cx);
            assert_eq!(registry.projects().len(), 2);
            registry.remove(&root, cx);
            assert_eq!(registry.projects().len(), 1);
            assert_eq!(registry.projects()[0].root, other);
        });
    }

    #[gpui::test]
    fn error_project_is_kept_without_manifest(cx: &mut gpui::TestAppContext) {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("broken");
        std::fs::create_dir_all(&root).unwrap();

        cx.update(|cx| {
            let mut registry = DevExtensionRegistry::default();
            registry.load(root.clone(), cx);
            assert_eq!(registry.projects().len(), 1);
            assert!(registry.projects()[0].error.is_some());
            assert!(registry.projects()[0].manifest.is_none());
        });
    }

    #[gpui::test]
    fn reload_replaces_manifest_in_place(cx: &mut gpui::TestAppContext) {
        use extension_runtime::extension::manifest::ShellSurface;

        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("tool");
        std::fs::create_dir_all(&root).unwrap();
        write_manifest(&root, "com.example.tool", "tab");

        cx.update(|cx| {
            let mut registry = DevExtensionRegistry::default();
            registry.load(root.clone(), cx);
            let first_surface = registry.projects()[0]
                .manifest
                .as_ref()
                .unwrap()
                .contributes
                .shell_views[0]
                .surface;
            assert_eq!(first_surface, ShellSurface::Tab);

            write_manifest(&root, "com.example.tool", "toolbox");
            registry.load(root.clone(), cx);
            assert_eq!(registry.projects().len(), 1);
            let after = registry.projects()[0]
                .manifest
                .as_ref()
                .unwrap()
                .contributes
                .shell_views[0]
                .surface;
            assert_eq!(after, ShellSurface::Toolbox);
        });
    }

    #[test]
    fn fingerprint_changes_when_ui_source_changes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path().join("tool");
        std::fs::create_dir_all(root.join("ui")).unwrap();
        write_manifest(&root, "com.example.tool", "tab");

        let before = source_fingerprint(&root).expect("fingerprint");
        let ui_entry = root.join("ui/tool.js");
        std::fs::write(&ui_entry, "export default class V {}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&ui_entry, "export default class V { render(){} }").unwrap();
        let after = source_fingerprint(&root).expect("fingerprint after");
        assert_ne!(before, after);
    }
}
