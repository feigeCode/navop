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
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use extension_runtime::GlobalExtensionRuntimeCatalog;
use extension_runtime::extension::manifest::{Manifest, load_from_dir};
#[cfg(feature = "shell-plugins")]
use universal_plugins::gpui_shell_reexport as gpui_shell_scope;

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

    fn project_value(project: &DevProject, watching: bool) -> HostValue {
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
            .field("watching", watching)
            .field("views", HostValue::Array(views))
            .into()
    }

    fn project_result(project: &DevProject) -> HostValue {
        HostObject::new()
            .field("root", project.root.display().to_string())
            .field(
                "id",
                project
                    .manifest
                    .as_ref()
                    .map(|manifest| strip_dev_prefix(&manifest.id))
                    .unwrap_or_default(),
            )
            .field(
                "error",
                project
                    .error
                    .clone()
                    .map(HostValue::Str)
                    .unwrap_or(HostValue::Null),
            )
            .into()
    }

    fn projects_value() -> Result<HostValue, HostError> {
        let projects = gpui_shell_scope::with_current_app(|cx| {
            let projects = DevExtensionRegistry::global(cx)
                .map(|registry| registry.projects().to_vec())
                .unwrap_or_default();
            let watching = cx
                .try_global::<DevWatchSet>()
                .map(|watch_set| {
                    watch_set
                        .0
                        .borrow()
                        .keys()
                        .cloned()
                        .collect::<std::collections::HashSet<_>>()
                })
                .unwrap_or_default();
            projects
                .into_iter()
                .map(|project| project_value(&project, watching.contains(&project.root)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
        Ok(HostValue::Array(projects))
    }

    fn open_project(root: &str) -> Result<HostValue, HostError> {
        let root = std::path::PathBuf::from(root);
        if !root.join("extension.json").is_file() {
            return Err(host_error(format!(
                "extension.json not found in {}",
                root.display()
            )));
        }
        gpui_shell_scope::with_current_app(|cx| {
            cx.update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
                registry.load(root.clone(), cx);
            })
        })
        .ok_or_else(|| host_error("no active app context"))?;
        let project = gpui_shell_scope::with_current_app(|cx| {
            DevExtensionRegistry::global(cx)
                .and_then(|registry| registry.project(&root))
                .cloned()
        })
        .flatten()
        .ok_or_else(|| host_error("development project was not retained"))?;
        gpui_shell_scope::with_current_app(|cx| {
            push_log(
                &*cx,
                &root,
                project
                    .error
                    .as_deref()
                    .map(|error| format!("open failed: {error}"))
                    .unwrap_or_else(|| "project loaded".to_string()),
            );
        });
        Ok(project_result(&project))
    }

    fn remove_project(root: &str) -> Result<HostValue, HostError> {
        let root = std::path::PathBuf::from(root);
        let project = gpui_shell_scope::with_current_app(|cx| {
            if let Some(watch_set) = cx.try_global::<DevWatchSet>() {
                if let Some((_, task, cancelled)) = watch_set.0.borrow_mut().remove(&root) {
                    cancelled.store(true, Ordering::Release);
                    drop(task);
                }
            }
            let project = cx.update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
                let project = registry.project(&root).cloned();
                registry.remove(&root, cx);
                project
            });
            if let Some(extension_id) = project
                .as_ref()
                .and_then(|project| project.manifest.as_ref())
                .map(|manifest| manifest.id.clone())
            {
                close_dev_extension(extension_id, cx);
            }
            project
        })
        .ok_or_else(|| host_error("no active app context"))?;
        Ok(project.as_ref().map(project_result).unwrap_or_else(|| {
            HostObject::new()
                .field("root", root.display().to_string())
                .field("id", "")
                .into()
        }))
    }

    fn open_view(extension_id: &str, view_id: &str) -> Result<HostValue, HostError> {
        let dev_extension_id = format!("{DEV_ID_PREFIX}{extension_id}");
        gpui_shell_scope::with_current_app(|cx| -> Result<(), HostError> {
            let contribution = extension_runtime::global_catalog(cx)
                .and_then(|catalog| catalog.shell_view(&dev_extension_id, view_id).cloned())
                .ok_or_else(|| {
                    host_error(format!(
                        "view not found: {extension_id}/{view_id}; reload the project and try again"
                    ))
                })?;
            if !contribution.entry_path.is_file() {
                return Err(host_error(format!(
                    "view entry not found: {}",
                    contribution.entry_path.display()
                )));
            }
            let extension_id = dev_extension_id.clone();
            let view_id = view_id.to_string();
            cx.defer(move |cx| {
                let Some(window) = crate::app_init::resolve_navop_window(cx) else {
                    tracing::warn!(%extension_id, %view_id, "no Navop window available for dev view");
                    return;
                };
                let _ = window.update(cx, |_, window, cx| {
                    extension_view::open_shell_view(&extension_id, &view_id, window, cx);
                });
            });
            Ok(())
        })
        .ok_or_else(|| host_error("no active app context"))??;
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
        Ok(HostValue::Array(
            logs.into_iter().map(HostValue::Str).collect(),
        ))
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
            let old_dev_id = cx.update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
                let old_dev_id = registry
                    .project(&root)
                    .and_then(|project| project.manifest.as_ref())
                    .map(|manifest| manifest.id.clone());
                registry.load(root.clone(), cx);
                old_dev_id
            });
            // 关闭该 dev 扩展已打开的视图，让下次 open 使用新 manifest。
            if let Some(old_dev_id) = old_dev_id {
                close_dev_extension(old_dev_id, cx);
            }
        })
        .ok_or_else(|| host_error("no active app context"))?;
        let project = gpui_shell_scope::with_current_app(|cx| {
            DevExtensionRegistry::global(cx)
                .and_then(|registry| registry.project(&root))
                .cloned()
        })
        .flatten()
        .ok_or_else(|| host_error("development project was not retained"))?;
        Ok(project_result(&project))
    }

    fn watch_project(root: &str) -> Result<HostValue, HostError> {
        let root = std::path::PathBuf::from(root);
        if !root.join("extension.json").is_file() {
            return Err(host_error(format!(
                "extension.json not found in {}",
                root.display()
            )));
        }
        gpui_shell_scope::with_current_app(|cx| {
            let Some(watch_set) = cx.try_global::<DevWatchSet>() else {
                return watch_result(false);
            };
            let mut watch_set = watch_set.0.borrow_mut();
            if let Some((_, task, cancelled)) = watch_set.remove(&root) {
                cancelled.store(true, Ordering::Release);
                drop(task);
                push_log(&*cx, &root, "watch stopped");
                return watch_result(false);
            }
            let Some(last_sig) = source_fingerprint(&root) else {
                return watch_result(false);
            };
            let watch_root = root.clone();
            let cancelled = Arc::new(AtomicBool::new(false));
            let task_cancelled = cancelled.clone();
            let task = cx.spawn(async move |cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(800))
                        .await;
                    if task_cancelled.load(Ordering::Acquire) {
                        break;
                    }
                    // 变更检测与重载都回主线程做,Global 访问需要 &App。
                    let _ = cx.update(|cx| {
                        let Some(watch_set) = cx.try_global::<DevWatchSet>() else {
                            return;
                        };
                        let Some(current) = source_fingerprint(&watch_root) else {
                            return;
                        };
                        let changed = {
                            let mut watch_set = watch_set.0.borrow_mut();
                            let Some((last, _, _)) = watch_set.get_mut(&watch_root) else {
                                return;
                            };
                            let changed = current != *last;
                            *last = current.clone();
                            changed
                        };
                        if changed {
                            let old_dev_id = cx
                                .try_global::<DevExtensionRegistry>()
                                .and_then(|registry| registry.project(&watch_root))
                                .and_then(|project| project.manifest.as_ref())
                                .map(|manifest| manifest.id.clone());
                            let _ = cx.update_default_global::<DevExtensionRegistry, _>(
                                |registry, cx| {
                                    registry.load(watch_root.clone(), cx);
                                },
                            );
                            if let Some(old_dev_id) = old_dev_id {
                                close_dev_extension(old_dev_id, cx);
                            }
                        }
                    });
                }
            });
            push_log(&*cx, &root, "watch started");
            watch_set.insert(root, (last_sig, task, cancelled));
            watch_result(true)
        })
        .ok_or_else(|| host_error("no active app context"))
    }

    fn pick_directory_start() -> Result<HostValue, HostError> {
        let launched = gpui_shell_scope::with_current_app(|cx| {
            let prompt = cx.prompt_for_paths(gpui::PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("选择扩展工程目录（含 extension.json）".into()),
            });
            if let Some(pick) = cx.try_global::<DevPickState>() {
                *pick.0.borrow_mut() = None;
            }
            cx.spawn(async move |cx| {
                let picked = match prompt.await {
                    Ok(Ok(Some(paths))) => paths.first().map(|path| path.display().to_string()),
                    _ => None,
                };
                let _ = cx.update(|cx| {
                    if let Some(pick) = cx.try_global::<DevPickState>() {
                        *pick.0.borrow_mut() = Some(picked.unwrap_or_default());
                    }
                });
            })
            .detach();
            true
        });
        if launched == Some(true) {
            Ok(HostValue::Str("pending".into()))
        } else {
            Err(host_error("no active app context for directory picker"))
        }
    }

    fn pick_directory_result() -> Result<HostValue, HostError> {
        let picked = gpui_shell_scope::with_current_app(|cx| {
            cx.try_global::<DevPickState>()
                .and_then(|pick| pick.0.borrow().clone())
        })
        .flatten();
        Ok(picked.map(HostValue::Str).unwrap_or(HostValue::Null))
    }

    let ops = Rc::new(universal_plugins::DevHostOps {
        list: Rc::new(projects_value),
        open: Rc::new(open_project),
        remove: Rc::new(remove_project),
        open_view: Rc::new(open_view),
        logs: Rc::new(project_logs),
        reload: Rc::new(reload_project),
        watch: Rc::new(watch_project),
        pick_start: Rc::new(pick_directory_start),
        pick_result: Rc::new(pick_directory_result),
    });
    cx.default_global::<DevExtensionRegistry>();
    cx.default_global::<DevLogRing>();
    cx.default_global::<DevWatchSet>();
    cx.default_global::<DevPickState>();
    universal_plugins::set_dev_host_ops(ops, cx);
}

#[cfg(feature = "shell-plugins")]
fn close_dev_extension(extension_id: String, cx: &mut gpui::App) {
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

#[cfg(feature = "shell-plugins")]
fn watch_result(watching: bool) -> gpui_shell_scope::HostValue {
    gpui_shell_scope::HostObject::new()
        .field("watching", watching)
        .into()
}

fn strip_dev_prefix(id: &str) -> String {
    id.strip_prefix(DEV_ID_PREFIX).unwrap_or(id).to_string()
}

/// 工程目录 source 指纹:recursive scan ui/ + entry,按 (相对路径, mt_s) 排序。
/// dev 视图少、文件小,轮询成本可忽略。
fn source_fingerprint(root: &std::path::Path) -> Option<String> {
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
    std::cell::RefCell<
        std::collections::HashMap<std::path::PathBuf, (String, gpui::Task<()>, Arc<AtomicBool>)>,
    >,
);
impl gpui::Global for DevWatchSet {}

/// 原生目录选择结果槽:pick_start 清空并起 task 等 modal,pick_result 读取。
#[derive(Default)]
pub struct DevPickState(std::cell::RefCell<Option<String>>);
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
