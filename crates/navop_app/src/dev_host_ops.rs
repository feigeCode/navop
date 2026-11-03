//! `navop.dev` 的操作集实现（nsh 形态）。
//!
//! 每个操作都在**主线程**上执行：引擎线程上的 handler 把参数搬过来，
//! 由 [`universal_plugins::NshHost`] 的主线程桥调用这里。所以这一层可以
//! 大方地读写 GPUI 全局、开窗口、弹文件选择器 —— 不需要 `with_current_app`
//! 那种"当前在谁的调用栈里"的技巧。
//!
//! 参数与返回值都是 JSON；失败只有一句话（交给宿主编成 `INVALID_ARGUMENT`）。

use std::path::PathBuf;

use gpui::{App, BorrowAppContext as _};
use serde_json::{Value, json};

use crate::dev_extension_registry::{
    DEV_ID_PREFIX, DevExtensionRegistry, DevLogRing, DevPickState, DevProject, DevWatchSet,
    close_dev_extension, push_log, source_fingerprint, strip_dev_prefix,
};

/// 装配并注入 `navop.dev` 的操作集。
pub fn install(cx: &mut App) {
    let ops = universal_plugins::DevOps {
        list: Some(std::rc::Rc::new(list)),
        open: Some(std::rc::Rc::new(open)),
        reload: Some(std::rc::Rc::new(reload)),
        watch: Some(std::rc::Rc::new(watch)),
        remove: Some(std::rc::Rc::new(remove)),
        open_view: Some(std::rc::Rc::new(open_view)),
        logs: Some(std::rc::Rc::new(logs)),
        pick_start: Some(std::rc::Rc::new(pick_start)),
        pick_result: Some(std::rc::Rc::new(pick_result)),
    };
    universal_plugins::set_dev_ops(std::rc::Rc::new(ops), cx);
}

fn view_value(
    view: &extension_runtime::extension::manifest::contributes::ShellViewContrib,
) -> Value {
    let surface = match view.surface {
        extension_runtime::extension::manifest::ShellSurface::Tab => "tab",
        extension_runtime::extension::manifest::ShellSurface::Toolbox => "toolbox",
    };
    json!({
        "id": view.id,
        "title": view.title,
        "surface": surface,
        "category": view.category,
    })
}

fn project_value(project: &DevProject, watching: bool) -> Value {
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
    json!({
        "root": project.root.display().to_string(),
        "id": id,
        "name": name,
        "version": version,
        "error": project.error,
        "watching": watching,
        "views": views,
    })
}

fn project_result(project: &DevProject) -> Value {
    json!({
        "root": project.root.display().to_string(),
        "id": project
            .manifest
            .as_ref()
            .map(|manifest| strip_dev_prefix(&manifest.id))
            .unwrap_or_default(),
        "error": project.error,
    })
}

fn list(cx: &mut App) -> Result<Value, String> {
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
    Ok(Value::Array(
        projects
            .into_iter()
            .map(|project| project_value(&project, watching.contains(&project.root)))
            .collect(),
    ))
}

fn open(cx: &mut App, root: &str) -> Result<Value, String> {
    let root = PathBuf::from(root);
    if !root.join("extension.json").is_file() {
        return Err(format!("extension.json not found in {}", root.display()));
    }
    cx.update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
        registry.load(root.clone(), cx);
    });
    let project = DevExtensionRegistry::global(cx)
        .and_then(|registry| registry.project(&root))
        .cloned()
        .ok_or_else(|| "development project was not retained".to_string())?;
    push_log(
        cx,
        &root,
        project
            .error
            .as_deref()
            .map(|error| format!("open failed: {error}"))
            .unwrap_or_else(|| "project loaded".to_string()),
    );
    Ok(project_result(&project))
}

fn reload(cx: &mut App, root: &str) -> Result<Value, String> {
    let root = PathBuf::from(root);
    if !root.join("extension.json").is_file() {
        return Err(format!("extension.json not found in {}", root.display()));
    }
    let old_dev_id = cx.update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
        let old_dev_id = registry
            .project(&root)
            .and_then(|project| project.manifest.as_ref())
            .map(|manifest| manifest.id.clone());
        registry.load(root.clone(), cx);
        old_dev_id
    });
    // 关掉该 dev 扩展已打开的视图，让下次打开用新 manifest。
    if let Some(old_dev_id) = old_dev_id {
        close_dev_extension(old_dev_id, cx);
    }
    let project = DevExtensionRegistry::global(cx)
        .and_then(|registry| registry.project(&root))
        .cloned()
        .ok_or_else(|| "development project was not retained".to_string())?;
    Ok(project_result(&project))
}

fn watch(cx: &mut App, root: &str) -> Result<Value, String> {
    let root = PathBuf::from(root);
    if !root.join("extension.json").is_file() {
        return Err(format!("extension.json not found in {}", root.display()));
    }
    let Some(watch_set) = cx.try_global::<DevWatchSet>() else {
        return Ok(json!({"watching": false}));
    };
    let mut watch_set = watch_set.0.borrow_mut();
    if let Some((_, task, cancelled)) = watch_set.remove(&root) {
        cancelled.store(true, std::sync::atomic::Ordering::Release);
        drop(task);
        push_log(cx, &root, "watch stopped");
        return Ok(json!({"watching": false}));
    }
    let Some(last_sig) = source_fingerprint(&root) else {
        return Ok(json!({"watching": false}));
    };
    let watch_root = root.clone();
    let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let task_cancelled = cancelled.clone();
    let task = cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(800))
                .await;
            if task_cancelled.load(std::sync::atomic::Ordering::Acquire) {
                break;
            }
            // 变更检测与重载都回主线程：Global 访问需要 `&mut App`。
            cx.update(|cx| {
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
                    *last = current;
                    changed
                };
                if changed {
                    let old_dev_id = cx
                        .try_global::<DevExtensionRegistry>()
                        .and_then(|registry| registry.project(&watch_root))
                        .and_then(|project| project.manifest.as_ref())
                        .map(|manifest| manifest.id.clone());
                    cx.update_default_global::<DevExtensionRegistry, _>(|registry, cx| {
                        registry.load(watch_root.clone(), cx);
                    });
                    push_log(&*cx, &watch_root, "source changed, reloaded");
                    if let Some(old_dev_id) = old_dev_id {
                        close_dev_extension(old_dev_id, cx);
                    }
                }
            });
        }
    });
    push_log(cx, &root, "watch started");
    watch_set.insert(root, (last_sig, task, cancelled));
    Ok(json!({"watching": true}))
}

fn remove(cx: &mut App, root: &str) -> Result<Value, String> {
    let root = PathBuf::from(root);
    if let Some(watch_set) = cx.try_global::<DevWatchSet>()
        && let Some((_, task, cancelled)) = watch_set.0.borrow_mut().remove(&root)
    {
        cancelled.store(true, std::sync::atomic::Ordering::Release);
        drop(task);
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
    Ok(project
        .as_ref()
        .map(project_result)
        .unwrap_or_else(|| json!({"root": root.display().to_string(), "id": ""})))
}

fn open_view(cx: &mut App, extension_id: &str, view_id: &str) -> Result<Value, String> {
    let dev_extension_id = format!("{DEV_ID_PREFIX}{extension_id}");
    let contribution = extension_runtime::global_catalog(cx)
        .and_then(|catalog| catalog.shell_view(&dev_extension_id, view_id).cloned())
        .ok_or_else(|| {
            format!("view not found: {extension_id}/{view_id}; reload the project and try again")
        })?;
    if !contribution.entry_path.is_file() {
        return Err(format!(
            "view entry not found: {}",
            contribution.entry_path.display()
        ));
    }
    let view_id = view_id.to_string();
    // 打开视图要窗口，等这一帧的借用结束再做。
    cx.defer(move |cx| {
        let Some(window) = crate::app_init::resolve_navop_window(cx) else {
            tracing::warn!(%dev_extension_id, %view_id, "no Navop window available for dev view");
            return;
        };
        let _ = window.update(cx, |_, window, cx| {
            extension_view::open_shell_view(&dev_extension_id, &view_id, window, cx);
        });
    });
    Ok(Value::Null)
}

fn logs(cx: &mut App, root: &str, tail: usize) -> Result<Value, String> {
    let root = PathBuf::from(root);
    let logs = cx
        .try_global::<DevLogRing>()
        .map(|ring| ring.tail(&root, tail))
        .unwrap_or_default();
    Ok(Value::Array(logs.into_iter().map(Value::String).collect()))
}

fn pick_start(cx: &mut App) -> Result<Value, String> {
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
    Ok(Value::String("pending".into()))
}

fn pick_result(cx: &mut App) -> Result<Value, String> {
    let picked = cx
        .try_global::<DevPickState>()
        .and_then(|pick| pick.0.borrow().clone());
    Ok(picked.map(Value::String).unwrap_or(Value::Null))
}
