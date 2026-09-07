//! 开发者工具的 dev 工程注册表。
//!
//! dev 工程 = 任意含 extension.json 的目录。加载时解析 manifest,与
//! 正式安装的 composite manifests 合并后整体重建
//! `GlobalExtensionRuntimeCatalog`。manifest.id 加 `dev.` 前缀,保证
//! view_key/runtime_key 与正式安装隔离;同 id 的正式安装永远优先
//!（dev 副本被跳过,不会遮蔽已安装扩展）。
//!
//! 不复制工程文件:entry/图标全部从工程目录解析。移除 dev 工程只重建
//! catalog,不动磁盘。

use gpui::BorrowAppContext as _;

use std::path::PathBuf;
use std::sync::Arc;

use extension_runtime::extension::manifest::{Manifest, load_from_dir};
#[cfg(feature = "shell-plugins")]
use universal_plugins::gpui_shell_reexport as gpui_shell_scope;
use extension_runtime::{ExtensionRuntimeCatalog, GlobalExtensionRuntimeCatalog};

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

fn push_log(cx: &gpui::App, root: &std::path::Path, message: impl Into<String>) {
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

    /// 加载/重载一个工程目录,返回展示名。解析失败也入列(带错误)。
    pub fn load(&mut self, root: PathBuf, cx: &mut gpui::App) -> String {
        let display_id = root
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| root.display().to_string());
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
        display_id
    }

    pub fn remove(&mut self, root: &std::path::Path, cx: &mut gpui::App) {
        self.projects.retain(|project| project.root != root);
        self.rebuild_catalog(cx);
    }

    /// 正式安装 manifests ∪ dev manifests → 整体重建 global catalog。
    fn rebuild_catalog(&self, cx: &mut gpui::App) {
        let mut manifests = installed_composite_manifests();
        let mut seen = manifests
            .iter()
            .map(|manifest| manifest.id.clone())
            .collect::<std::collections::HashSet<_>>();
        for project in &self.projects {
            let Some(manifest) = &project.manifest else {
                continue;
            };
            // dev. 前缀天然去重;同工程重复加载在 load() 已替换。
            if !seen.insert(manifest.id.clone()) {
                continue;
            }
            manifests.push(manifest.clone());
        }
        match ExtensionRuntimeCatalog::from_manifests(manifests) {
            Ok(catalog) => {
                if let Some(global) = cx.try_global::<GlobalExtensionRuntimeCatalog>() {
                    global.replace_arc(Arc::new(catalog));
                }
            }
            Err(error) => tracing::warn!(%error, "dev extension catalog rebuild failed"),
        }
    }
}

/// 已安装 composite 的 manifests(从正式安装根读取;失败时为空,即只剩
/// dev 条目——不让安装根的临时故障吞掉 dev 工程)。
fn installed_composite_manifests() -> Vec<Manifest> {
    let Some(root) = extension_runtime::extension::extensions_root().map(|root| {
        root.join(extension_runtime::extension::ExtensionKind::Composite.dir_name())
    }) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter_map(|path| load_from_dir(&path).ok())
        .collect()
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

fn host_error(message: impl Into<String>) -> gpui_shell_scope::HostError {
    gpui_shell_scope::HostError::new(message.into())
}

/// 构造 main 层 DevHostOps,注入到 universal-plugins Global。
#[cfg(feature = "shell-plugins")]
pub fn install_dev_host_ops(cx: &mut gpui::App) {
    use std::rc::Rc;
    use universal_plugins::gpui_shell_reexport::{HostError, HostObject, HostValue};

    fn view_value(
        view: &extension_runtime::extension::manifest::contributes::ShellViewContrib,
    ) -> HostValue {
        let surface = match view.surface {
            extension_runtime::extension::manifest::ShellSurface::Tab => "tab",
            extension_runtime::extension::manifest::ShellSurface::Toolbox => "toolbox",
        };
        HostObject::new()
            .field("id", view.id.clone())
            .field("title", view.title.clone())
            .field("surface", surface)
            .field(
                "category",
                view.category
                    .clone()
                    .map(HostValue::Str)
                    .unwrap_or(HostValue::Null),
            )
            .into()
    }

    fn project_value(project: &DevProject) -> HostValue {
        let (id, name, version, views) = match &project.manifest {
            Some(manifest) => (
                strip_dev_prefix(&manifest.id),
                manifest.name.clone(),
                manifest.version.clone(),
                manifest
                    .contributes
                    .shell_views
                    .iter()
                    .map(view_value)
                    .collect::<Vec<_>>(),
            ),
            None => (String::new(), String::new(), String::new(), Vec::new()),
        };
        HostObject::new()
            .field("root", project.root.display().to_string())
            .field("id", id)
            .field("name", name)
            .field("version", version)
            .field(
                "error",
                project
                    .error
                    .clone()
                    .map(HostValue::Str)
                    .unwrap_or(HostValue::Null),
            )
            .field("views", HostValue::Array(views))
            .into()
    }

    fn projects_value() -> Result<HostValue, HostError> {
        let projects = gpui_shell_scope::with_current_app(|cx| {
            DevExtensionRegistry::global(cx)
                .map(|registry| registry.projects().to_vec())
        })
        .flatten()
        .unwrap_or_default();
        let items = projects
            .iter()
            .map(project_value)
            .collect::<Vec<_>>();
        Ok(HostValue::Array(items))
    }

    fn open_project(root: &str) -> Result<HostValue, HostError> {
        let root = std::path::PathBuf::from(root);
        if !root.join("extension.json").is_file() {
            return Err(host_error(format!(
                "extension.json not found in {}",
                root.display()
            )));
        }
        let id = gpui_shell_scope::with_current_app(|cx| {
            cx.update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
                registry.load(root.clone(), cx)
            })
        })
        .ok_or_else(|| host_error("no active app context"))?;
        let error = gpui_shell_scope::with_current_app(|cx| {
            DevExtensionRegistry::global(cx)
                .and_then(|registry| {
                    registry
                        .projects()
                        .iter()
                        .find(|project| project.root == root)
                })
                .and_then(|project| project.error.clone())
        })
        .flatten();
        gpui_shell_scope::with_current_app(|cx| {
            push_log(
                &*cx,
                &root,
                error
                    .as_deref()
                    .map(|error| format!("open failed: {error}"))
                    .unwrap_or_else(|| "project loaded".to_string()),
            );
        });
        Ok(HostObject::new()
            .field("id", id)
            .field(
                "error",
                error.map(HostValue::Str).unwrap_or(HostValue::Null),
            )
            .into())
    }

    fn remove_project(root: &str) -> Result<HostValue, HostError> {
        let root = std::path::PathBuf::from(root);
        gpui_shell_scope::with_current_app(|cx| {
            cx.update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
                registry.remove(&root, cx);
            });
        })
        .ok_or_else(|| host_error("no active app context"))?;
        Ok(HostValue::Null)
    }

    fn open_view(extension_id: &str, view_id: &str) -> Result<HostValue, HostError> {
        let dev_extension_id = format!("{DEV_ID_PREFIX}{extension_id}");
        gpui_shell_scope::with_current_app(|cx| {
            let Some(window) = cx.windows().first().copied() else {
                return;
            };
            let _ = window.update(cx, |_, window, cx| {
                extension_view::open_shell_view(&dev_extension_id, view_id, window, cx);
            });
        })
        .ok_or_else(|| host_error("no active app context"))?;
        Ok(HostValue::Null)
    }

    fn project_logs(root: &str, tail: f64) -> Result<HostValue, HostError> {
        let root = std::path::PathBuf::from(root);
        let logs = gpui_shell_scope::with_current_app(|cx| {
            cx.try_global::<DevLogRing>()
                .map(|ring| ring.tail(&root, tail.max(0.0) as usize))
                .unwrap_or_default()
        })
        .unwrap_or_default();
        Ok(HostValue::Array(logs.into_iter().map(HostValue::Str).collect()))
    }

    fn reload_project(root: &str) -> Result<HostValue, HostError> {
        let root = std::path::PathBuf::from(root);
        if !root.join("extension.json").is_file() {
            return Err(host_error(format!(
                "extension.json not found in {}",
                root.display()
            )));
        }
        gpui_shell_scope::with_current_app(|cx| {
            let dev_id = cx
                .update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
                    registry.load(root.clone(), cx);
                    registry
                        .projects()
                        .iter()
                        .find(|project| project.root == root)
                        .and_then(|project| project.manifest.as_ref())
                        .map(|manifest| manifest.id.clone())
                });
            // 关闭该 dev 扩展已打开的视图(dev 前缀 id),让下次 open 用新 manifest。
            if let Some(dev_id) = dev_id {
                for window in cx.windows().to_vec() {
                    let _ = window.update(cx, |_, window, cx| {
                        let _ = extension_view::close_shell_extension(&dev_id, window, cx);
                    });
                }
            }
        })
        .ok_or_else(|| host_error("no active app context"))?;
        Ok(HostValue::Null)
    }

    let ops = Rc::new(universal_plugins::DevHostOps {
        list: Rc::new(projects_value),
        open: Rc::new(open_project),
        remove: Rc::new(remove_project),
        open_view: Rc::new(open_view),
        logs: Rc::new(project_logs),
        reload: Rc::new(reload_project),
    });
    cx.default_global::<DevExtensionRegistry>();
    cx.default_global::<DevLogRing>();
    universal_plugins::set_dev_host_ops(ops, cx);
}

fn strip_dev_prefix(id: &str) -> String {
    id.strip_prefix(DEV_ID_PREFIX)
        .unwrap_or(id)
        .to_string()
}

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
                "engines": { "onetcli": ">=0.1.0", "gpui_shell": "0.2.0" },
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
            let display = registry.load(root.clone(), cx);
            assert_eq!(display, "broken");
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

            // 改写 extension.json 后 reload:同目录替换而非追加。
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
}
