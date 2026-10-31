//! i18n 契约：源码里 `t!("...")` 用到的键，必须在**调用方 crate 自己**的语言包里存在。
//!
//! rust-i18n 的 `i18n!` 宏为每个调用它的 crate 生成独立后端，`t!` 查不到键时会把键名原样
//! 返回，界面上就直接显示 `ConnectionForm.ai_access` 这种原始 key。各 crate 的语言包互相
//! 不可见，所以「同一个通用组件被几个表单复用、只有其中几个表单补了 key」必须在测试里拦住。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn every_t_key_exists_in_its_crate_locales() {
    // `CARGO_MANIFEST_DIR` 是 `crates/navop_app`，工作区根在两级之上。
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf();

    // navop_app 自己就在 `crates/` 下，会被下面的遍历自动收录。
    let mut crate_dirs: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(workspace.join("crates")).expect("crates dir") {
        let path = entry.expect("crate entry").path();
        if path.join("src").is_dir() {
            crate_dirs.push(path);
        }
    }

    let mut checked = 0usize;
    let mut problems: Vec<String> = Vec::new();

    for crate_dir in &crate_dirs {
        let source_dir = crate_dir.join("src");
        if !source_dir.is_dir() {
            continue;
        }
        let mut sources = Vec::new();
        collect_rust_sources(&source_dir, &mut sources);

        // 语言包目录来自 crate 内的 `i18n!("...")` 声明：语言包可以被多个 crate 共享
        // （例如 mongodb-runtime / redis-runtime 复用对应 view crate 的语言包）。
        let mut locales: BTreeSet<String> = BTreeSet::new();
        let mut declared = false;
        for file in &sources {
            let text = fs::read_to_string(file).unwrap_or_default();
            for locale_dir in macro_string_literals(&text, "i18n") {
                let dir = crate_dir.join(locale_dir);
                if !dir.is_dir() {
                    continue;
                }
                declared = true;
                for entry in fs::read_dir(&dir).expect("locale dir") {
                    let path = entry.expect("locale entry").path();
                    if path.extension().and_then(|ext| ext.to_str()) == Some("yml") {
                        let text = fs::read_to_string(&path).unwrap_or_default();
                        locales.extend(locale_keys(&text));
                    }
                }
            }
        }
        // 协议层 / 驱动层 crate 不声明语言包，也不会调用 `t!`。
        if !declared {
            continue;
        }

        let crate_name = crate_dir
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        for file in &sources {
            if is_test_only(file) {
                continue;
            }
            let text = fs::read_to_string(file).unwrap_or_default();
            for key in macro_string_literals(&text, "t") {
                checked += 1;
                if !locales.contains(&key) {
                    problems.push(format!(
                        "{}: {crate_name} 的语言包缺少键 {key}",
                        display(file, &workspace)
                    ));
                }
            }
        }
    }

    assert!(
        checked > 500,
        "只扫到 {checked} 个 t! 键，扫描逻辑可能已经失效"
    );
    assert!(
        problems.is_empty(),
        "以下 t! 键在调用方 crate 的语言包里不存在（界面会显示原始 key）：\n{}",
        problems.join("\n")
    );
}

/// 递归收集 `src` 下的 Rust 源文件。
fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir).expect("read source dir");
    for entry in entries {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// 测试专用文件里可能写死假键名（如 `SqlLibrary.*` 的键名抽取逻辑），不参与扫描。
fn is_test_only(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "tests.rs")
        || path
            .components()
            .any(|component| component.as_os_str() == "tests")
}

fn display(path: &Path, workspace: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// 取出 `名字!(` 后面紧跟的字符串字面量（`t!("Key")` / `i18n!("locales")`）。
fn macro_string_literals(source: &str, macro_name: &str) -> Vec<String> {
    let pattern = format!("{macro_name}!(");
    let mut literals = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = source[cursor..].find(&pattern) {
        let start = cursor + offset;
        cursor = start + pattern.len();
        // 排除 `mi18n!(` / `xt!(`,但允许 `rust_i18n::i18n!(`。
        let boundary_ok = source[..start]
            .chars()
            .next_back()
            .is_none_or(|prev| !(prev.is_alphanumeric() || prev == '_'));
        let Some((literal, next)) = string_literal_after(source, cursor) else {
            continue;
        };
        cursor = next;
        if boundary_ok {
            literals.push(literal);
        }
    }
    literals
}

/// 从 `index` 起跳过空白后读取一个字符串字面量，返回字面量内容与字面量结束位置。
fn string_literal_after(source: &str, mut index: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    if bytes.get(index) != Some(&b'"') {
        return None;
    }
    let start = index + 1;
    let end = source[start..].find('"')? + start;
    Some((source[start..end].to_string(), end + 1))
}

/// 按缩进把 `a:\n  b:` 形式的语言包拼成 `a.b` 键集合。
fn locale_keys(source: &str) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    let mut stack: Vec<(usize, String)> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        let Some((key, _value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() {
            continue;
        }
        while stack.last().is_some_and(|(depth, _)| *depth >= indent) {
            stack.pop();
        }
        let mut full = stack
            .iter()
            .map(|(_, segment)| segment.as_str())
            .collect::<Vec<_>>()
            .join(".");
        if !full.is_empty() {
            full.push('.');
        }
        full.push_str(key);
        keys.insert(full);
        stack.push((indent, key.to_string()));
    }
    keys
}
