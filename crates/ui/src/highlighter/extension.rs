//! 语言扩展包格式与单个扩展加载逻辑。
//!
//! 一个语言扩展是磁盘上的一个目录,包含:
//!
//! ```text
//! {lang}/
//! ├── manifest.json    # 必需,包含 name/version/file_extensions/...
//! ├── parser.wasm      # 必需,tree-sitter 编译产物
//! ├── highlights.scm   # 可选,高亮 query
//! ├── injections.scm   # 可选,语言注入 query
//! └── locals.scm       # 可选,局部作用域 query
//! ```
//!
//! 调用方拿到目录路径后通过 [`InstalledExtension::load_from_dir`] 解析,
//! 再交给 [`LanguageRegistry::register_wasm`] 注册到全局高亮系统。

use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use gpui::SharedString;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::highlighter::LanguageRegistry;

/// 单个语言扩展的元数据,对应 `manifest.json`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageManifest {
    /// 语言名,必须与 wasm 模块中导出的 `tree_sitter_<name>` 函数名匹配。
    pub name: String,

    /// 扩展自身版本(SemVer 推荐,但不强校验)。
    #[serde(default)]
    pub version: String,

    /// 该语言对应的文件扩展名列表(不带点,如 `["rs"]`)。
    #[serde(default)]
    pub file_extensions: Vec<String>,

    /// 该语言可能注入的其他语言(必须先于本扩展加载)。
    #[serde(default)]
    pub injection_languages: Vec<String>,

    /// 该扩展依赖的其他扩展(用于拓扑排序)。
    #[serde(default)]
    pub requires: Vec<String>,

    /// 可选的 `parser.wasm` SHA-256 校验和(十六进制小写,64 字符)。
    /// 如果提供,加载时会校验文件内容;不匹配则拒绝加载。
    #[serde(default)]
    pub sha256_wasm: Option<String>,
}

/// 从磁盘读取的一个完整扩展,等待注册。
#[derive(Debug, Clone)]
pub struct InstalledExtension {
    pub manifest: LanguageManifest,
    pub wasm_bytes: Vec<u8>,
    pub highlights: String,
    pub injections: String,
    pub locals: String,
    pub source_path: PathBuf,
}

impl InstalledExtension {
    /// 从指定目录加载扩展。目录必须包含 `manifest.json` 与 `parser.wasm`,
    /// 其余文件均为可选(缺失时视为空字符串)。
    pub fn load_from_dir(dir: &Path) -> Result<Self> {
        let manifest_path = dir.join("manifest.json");
        let manifest_raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?;
        let manifest: LanguageManifest = serde_json::from_str(&manifest_raw)
            .with_context(|| format!("parse {}", manifest_path.display()))?;

        if manifest.name.trim().is_empty() {
            anyhow::bail!(
                "manifest.json at {} has empty `name`",
                manifest_path.display()
            );
        }

        let wasm_path = dir.join("parser.wasm");
        let wasm_bytes =
            fs::read(&wasm_path).with_context(|| format!("read {}", wasm_path.display()))?;

        if let Some(expected) = &manifest.sha256_wasm {
            verify_sha256(&wasm_bytes, expected)
                .with_context(|| format!("verify sha256 for {}", wasm_path.display()))?;
        }

        let highlights = read_optional(&dir.join("highlights.scm"))?;
        let injections = read_optional(&dir.join("injections.scm"))?;
        let locals = read_optional(&dir.join("locals.scm"))?;

        Ok(Self {
            manifest,
            wasm_bytes,
            highlights,
            injections,
            locals,
            source_path: dir.to_path_buf(),
        })
    }

    /// 将本扩展注册到给定的 [`LanguageRegistry`]。
    pub fn register(&self, registry: &LanguageRegistry) -> Result<()> {
        let injections_langs: Vec<SharedString> = self
            .manifest
            .injection_languages
            .iter()
            .map(|s| SharedString::from(s.clone()))
            .collect();

        registry.register_wasm(
            &self.manifest.name,
            self.wasm_bytes.clone(),
            &self.manifest.file_extensions,
            injections_langs,
            &self.highlights,
            &self.injections,
            &self.locals,
        )
    }

    /// 卸载磁盘上的扩展目录,同时从全局注册表中移除对应语言。
    ///
    /// `dir` 必须指向一个完整的扩展目录(同 [`load_from_dir`]),会先读取
    /// `manifest.json` 获取语言名,再执行删除。这样即便目录名与语言名不一致
    /// 也能正确取消注册。
    ///
    /// 删除是不可恢复的,调用方负责确认。
    pub fn uninstall(dir: &Path, registry: &LanguageRegistry) -> Result<String> {
        let manifest_path = dir.join("manifest.json");
        let manifest_raw = fs::read_to_string(&manifest_path)
            .with_context(|| format!("read {}", manifest_path.display()))?;
        let manifest: LanguageManifest = serde_json::from_str(&manifest_raw)
            .with_context(|| format!("parse {}", manifest_path.display()))?;
        let name = manifest.name.clone();

        registry.unregister(&name);
        fs::remove_dir_all(dir).with_context(|| format!("remove {}", dir.display()))?;
        Ok(name)
    }
}

fn read_optional(path: &Path) -> Result<String> {
    if path.exists() {
        fs::read_to_string(path).with_context(|| format!("read {}", path.display()))
    } else {
        Ok(String::new())
    }
}

/// 仅读取 `manifest.json`,不加载 wasm 等大文件。用于扩展列表展示等场景。
pub(crate) fn read_manifest_only(dir: &Path) -> Result<LanguageManifest> {
    let manifest_path = dir.join("manifest.json");
    let raw = fs::read_to_string(&manifest_path)
        .with_context(|| format!("read {}", manifest_path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("parse {}", manifest_path.display()))
}

/// 计算字节流的 SHA-256 摘要(十六进制小写)。
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest.iter() {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{:02x}", b);
    }
    s
}

/// 验证字节流的 SHA-256 是否与期望值匹配。比较时:
/// - 忽略大小写
/// - 容许 `sha256:` 前缀
/// - 不匹配时返回 `Err`,错误消息包含实际与期望值
pub fn verify_sha256(bytes: &[u8], expected: &str) -> Result<()> {
    let normalized = expected.trim().trim_start_matches("sha256:").to_lowercase();
    if normalized.len() != 64 {
        anyhow::bail!(
            "invalid sha256 length: expected 64 hex chars, got {}",
            normalized.len()
        );
    }
    let actual = sha256_hex(bytes);
    if actual != normalized {
        anyhow::bail!("sha256 mismatch: expected {normalized}, got {actual}");
    }
    Ok(())
}
