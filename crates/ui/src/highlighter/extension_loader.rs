//! 扫描扩展根目录、按依赖拓扑顺序注册所有语言扩展。
//!
//! 调用方负责提供扩展根目录(例如 `~/.config/navop/extensions/languages/`),
//! 本模块只关心目录布局与加载流程。每个直接子目录都被视为一个候选扩展。
//!
//! 加载流程:
//! 1. 列出根目录下的全部子目录
//! 2. 解析各自的 `manifest.json`
//! 3. 根据 `requires` 字段做拓扑排序(检测循环)
//! 4. 顺序调用 [`InstalledExtension::register`] 注入到全局 [`LanguageRegistry`]
//!
//! 单个扩展加载失败不会阻断其他扩展的加载,失败信息会通过 tracing 上报。

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::Result;

use crate::highlighter::{InstalledExtension, LanguageRegistry};

/// 一次加载尝试的结果摘要。
#[derive(Debug, Default, Clone)]
pub struct LoadReport {
    pub loaded: Vec<String>,
    pub failed: Vec<(String, String)>,
}

impl LoadReport {
    pub fn is_empty(&self) -> bool {
        self.loaded.is_empty() && self.failed.is_empty()
    }
}

/// 扫描 `root` 下所有子目录,加载并注册到 `registry`。
///
/// `root` 不存在时返回空 [`LoadReport`](无错误)。
pub fn load_extensions_dir(root: &Path, registry: &LanguageRegistry) -> Result<LoadReport> {
    if !root.exists() {
        return Ok(LoadReport::default());
    }

    let candidate_dirs = list_subdirs(root)?;
    let mut extensions: HashMap<String, InstalledExtension> = HashMap::new();
    let mut report = LoadReport::default();

    for dir in candidate_dirs {
        match InstalledExtension::load_from_dir(&dir) {
            Ok(ext) => {
                if extensions.contains_key(&ext.manifest.name) {
                    tracing::warn!(
                        "duplicate language extension {:?} at {}; keeping earlier one",
                        ext.manifest.name,
                        dir.display()
                    );
                    report
                        .failed
                        .push((ext.manifest.name.clone(), "duplicate".into()));
                    continue;
                }
                extensions.insert(ext.manifest.name.clone(), ext);
            }
            Err(e) => {
                let id = dir
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| dir.display().to_string());
                tracing::warn!(
                    "failed to load language extension at {}: {:?}",
                    dir.display(),
                    e
                );
                report.failed.push((id, format!("{e:?}")));
            }
        }
    }

    let ordered = topological_sort(&extensions)?;
    for name in ordered {
        let Some(ext) = extensions.get(&name) else {
            continue;
        };
        match ext.register(registry) {
            Ok(()) => {
                tracing::info!(
                    "loaded language extension {:?} from {}",
                    name,
                    ext.source_path.display()
                );
                report.loaded.push(name);
            }
            Err(e) => {
                tracing::warn!("failed to register language {:?}: {:?}", name, e);
                report.failed.push((name, format!("{e:?}")));
            }
        }
    }

    Ok(report)
}

/// 扫描 `root` 下的语言扩展 manifest,只注册语言名与文件后缀映射。
///
/// 该路径不读取 `parser.wasm`,用于启动时快速建立后缀索引;真实 wasm
/// parser 会在首次请求该语言时由 [`LanguageRegistry`] 按需加载。
pub fn register_extension_manifests_dir(
    root: &Path,
    registry: &LanguageRegistry,
) -> Result<LoadReport> {
    if !root.exists() {
        return Ok(LoadReport::default());
    }

    let mut report = LoadReport::default();
    let mut seen = HashSet::new();
    for dir in list_subdirs(root)? {
        match crate::highlighter::extension::read_manifest_only(&dir) {
            Ok(manifest) => {
                if !seen.insert(manifest.name.clone()) {
                    report
                        .failed
                        .push((manifest.name.clone(), "duplicate".into()));
                    continue;
                }
                registry.register_wasm_manifest(manifest.clone(), dir);
                report.loaded.push(manifest.name);
            }
            Err(error) => {
                let id = dir
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| dir.display().to_string());
                tracing::warn!(
                    "failed to read language extension manifest at {}: {:?}",
                    dir.display(),
                    error
                );
                report.failed.push((id, format!("{error:?}")));
            }
        }
    }
    Ok(report)
}

/// 列出 `root` 下所有可读的扩展(仅解析 manifest,不加载 wasm)。
///
/// 失败的子目录会被静默跳过(通过 tracing 报告),返回的列表只包含
/// manifest 解析成功的扩展。用于 UI/CLI 展示已安装清单。
pub fn list_installed(root: &Path) -> Result<Vec<InstalledExtensionSummary>> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for dir in list_subdirs(root)? {
        match crate::highlighter::extension::read_manifest_only(&dir) {
            Ok(manifest) => {
                out.push(InstalledExtensionSummary {
                    name: manifest.name.clone(),
                    version: manifest.version.clone(),
                    file_extensions: manifest.file_extensions.clone(),
                    path: dir,
                });
            }
            Err(e) => {
                tracing::warn!("failed to read manifest for {}: {:?}", dir.display(), e);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// `list_installed` 返回的轻量摘要(不含 wasm 字节)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledExtensionSummary {
    pub name: String,
    pub version: String,
    pub file_extensions: Vec<String>,
    pub path: PathBuf,
}

fn list_subdirs(root: &Path) -> Result<Vec<PathBuf>> {
    let mut dirs = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            dirs.push(path);
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// 基于 `requires` 字段对扩展做拓扑排序。
///
/// - 依赖中引用了未安装的扩展时,该缺失依赖会被忽略(不阻断本扩展加载)
/// - 检测到循环依赖时返回错误
pub(crate) fn topological_sort(
    extensions: &HashMap<String, InstalledExtension>,
) -> Result<Vec<String>> {
    enum Mark {
        Visiting,
        Done,
    }

    let mut order = Vec::with_capacity(extensions.len());
    let mut marks: HashMap<&str, Mark> = HashMap::new();

    fn visit<'a>(
        name: &'a str,
        extensions: &'a HashMap<String, InstalledExtension>,
        marks: &mut HashMap<&'a str, Mark>,
        order: &mut Vec<String>,
        path: &mut HashSet<String>,
    ) -> Result<()> {
        match marks.get(name) {
            Some(Mark::Done) => return Ok(()),
            Some(Mark::Visiting) => {
                anyhow::bail!("cyclic dependency in language extensions involving {name}");
            }
            None => {}
        }
        let Some(ext) = extensions.get(name) else {
            return Ok(()); // 缺失依赖: 静默跳过,由上层 tracing 报告
        };
        marks.insert(name, Mark::Visiting);
        path.insert(name.to_string());
        for dep in &ext.manifest.requires {
            visit(dep, extensions, marks, order, path)?;
        }
        path.remove(name);
        marks.insert(name, Mark::Done);
        order.push(name.to_string());
        Ok(())
    }

    let mut names: Vec<&str> = extensions.keys().map(|s| s.as_str()).collect();
    names.sort();
    let mut path = HashSet::new();
    for name in names {
        visit(name, extensions, &mut marks, &mut order, &mut path)?;
    }
    Ok(order)
}
