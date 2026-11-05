//! i18n 契约：源码里 `t!("...")` 用到的键，必须在**调用方 crate 自己**的语言包里存在；
//! 传进去的插值参数，也必须在文案里以 `%{参数}` 的形式真的被引用。
//!
//! rust-i18n 的 `i18n!` 宏为每个调用它的 crate 生成独立后端，`t!` 查不到键时会把键名原样
//! 返回，界面上就直接显示 `ConnectionForm.ai_access` 这种原始 key。各 crate 的语言包互相
//! 不可见，所以「同一个通用组件被几个表单复用、只有其中几个表单补了 key」必须在测试里拦住。
//!
//! 插值同理：rust-i18n 的占位符语法是 `%{name}`（不是 `{name}`），写错了不会报错，弹窗里就
//! 直接露出 `{app}`；参数名和占位符对不上时参数被静默丢弃，界面少一半信息。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_yaml::Value;

#[test]
fn every_t_key_exists_in_its_crate_locales() {
    let workspace = workspace_root();
    // navop_app 自己就在 `crates/` 下，会被下面的遍历自动收录。
    let crate_dirs = crate_dirs(&workspace);

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
        let Some(locale_files) = crate_locale_files(crate_dir, &sources) else {
            // 协议层 / 驱动层 crate 不声明语言包，也不会调用 `t!`。
            continue;
        };
        let mut locales: BTreeSet<String> = BTreeSet::new();
        for path in &locale_files {
            let text = fs::read_to_string(path).unwrap_or_default();
            locales.extend(locale_keys(&text));
        }

        let crate_name = crate_name(crate_dir);
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

/// 回归：`t!("Key", name = …)` 传的插值参数与文案里的 `%{name}` 占位符必须一一对上。
///
/// 两个方向都要查：
/// - 传了参数但文案没写 `%{name}`：反例是 `Quit.confirm_title_app` 把占位符写成 `{app}`，
///   rust-i18n 不认，退出弹窗直接显示字面量 `{app}`；漏写 `%{shortcut}` 则参数被静默丢掉。
/// - 文案写了 `%{name}` 但调用没传：界面上会原样露出 `%{name}`。
#[test]
fn every_t_interpolation_argument_matches_its_template() {
    let workspace = workspace_root();
    let crate_dirs = crate_dirs(&workspace);

    let mut checked = 0usize;
    let mut problems: Vec<String> = Vec::new();

    for crate_dir in &crate_dirs {
        let source_dir = crate_dir.join("src");
        if !source_dir.is_dir() {
            continue;
        }
        let mut sources = Vec::new();
        collect_rust_sources(&source_dir, &mut sources);
        let Some(locale_files) = crate_locale_files(crate_dir, &sources) else {
            continue;
        };
        // 运行时 rust-i18n 会把同 crate 的多个 yml 合并，这里同样按文件顺序合并成多个文档。
        let documents: Vec<Value> = locale_files
            .iter()
            .map(|path| {
                let text = fs::read_to_string(path).unwrap_or_default();
                serde_yaml::from_str(&text).unwrap_or_else(|error| {
                    panic!(
                        "{}: 语言包不是合法 YAML（{error}）",
                        display(path, &workspace)
                    )
                })
            })
            .collect();

        let crate_name = crate_name(crate_dir);
        for file in &sources {
            if is_test_only(file) {
                continue;
            }
            let text = fs::read_to_string(file).unwrap_or_default();
            for (key, arguments) in t_calls(&text) {
                checked += 1;
                let Some(node) = documents.iter().find_map(|doc| yaml_lookup(doc, &key)) else {
                    // 键不存在由上面的测试负责报；这里只关心插值。
                    continue;
                };
                for (locale, template) in yaml_templates(node) {
                    for argument in &arguments {
                        let placeholder = format!("%{{{argument}}}");
                        if !template.contains(&placeholder) {
                            problems.push(format!(
                                "{}: {crate_name} 的 {key}（{locale}）文案里没有 {placeholder} 插值，传入的 {argument} 会被丢掉：{template}",
                                display(file, &workspace)
                            ));
                        }
                    }
                    for placeholder in placeholders(&template) {
                        if !arguments.contains(&placeholder) {
                            problems.push(format!(
                                "{}: {crate_name} 的 {key}（{locale}）文案需要 %{{{placeholder}}} 插值，但这次调用没传：{template}",
                                display(file, &workspace)
                            ));
                        }
                    }
                }
            }
        }
    }

    assert!(
        checked > 500,
        "只扫到 {checked} 个 t! 调用，扫描逻辑可能已经失效"
    );
    assert!(
        problems.is_empty(),
        "以下 t! 的插值参数与文案占位符对不上（界面会露出 `%{{参数}}` 或丢信息）：\n{}",
        problems.join("\n")
    );
}

/// `CARGO_MANIFEST_DIR` 是 `crates/navop_app`，工作区根在两级之上。
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

/// `crates/` 下有 `src` 的 crate 目录。
fn crate_dirs(workspace: &Path) -> Vec<PathBuf> {
    let mut crate_dirs = Vec::new();
    for entry in fs::read_dir(workspace.join("crates")).expect("crates dir") {
        let path = entry.expect("crate entry").path();
        if path.join("src").is_dir() {
            crate_dirs.push(path);
        }
    }
    crate_dirs
}

fn crate_name(crate_dir: &Path) -> String {
    crate_dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string()
}

/// crate 声明的语言包 yml 文件；`None` 表示这个 crate 不声明语言包。
fn crate_locale_files(crate_dir: &Path, sources: &[PathBuf]) -> Option<Vec<PathBuf>> {
    let mut files = BTreeSet::new();
    let mut declared = false;
    for file in sources {
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
                    files.insert(path);
                }
            }
        }
    }
    declared.then(|| files.into_iter().collect())
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

/// 取出 `t!("Key", name = value, …)` 的键与命名插值参数（不含 `locale` / `_minify_key*` 这类控制参数）。
fn t_calls(source: &str) -> Vec<(String, Vec<String>)> {
    let pattern = "t!(";
    let mut calls = Vec::new();
    let mut cursor = 0;
    while let Some(offset) = source[cursor..].find(pattern) {
        let start = cursor + offset;
        cursor = start + pattern.len();
        let boundary_ok = source[..start]
            .chars()
            .next_back()
            .is_none_or(|prev| !(prev.is_alphanumeric() || prev == '_'));
        if !boundary_ok {
            continue;
        }
        let Some((key, next)) = string_literal_after(source, cursor) else {
            continue;
        };
        let Some((arguments, end)) = call_arguments(source, next) else {
            continue;
        };
        cursor = end;
        let arguments: Vec<String> = named_arguments(&arguments)
            .into_iter()
            .filter(|name| name != "locale" && !name.starts_with("_minify_key"))
            .collect();
        calls.push((key, arguments));
    }
    calls
}

/// 文案里的 `%{name}` 占位符（rust-i18n 的插值语法）。
fn placeholders(template: &str) -> Vec<String> {
    let mut placeholders = Vec::new();
    let mut rest = template;
    while let Some(offset) = rest.find("%{") {
        let after = &rest[offset + 2..];
        let Some(end) = after.find('}') else {
            break;
        };
        placeholders.push(after[..end].to_string());
        rest = &after[end + 1..];
    }
    placeholders
}

/// 从键字面量之后读实参：`()` 返回空串，`(a, b)` 返回逗号后的原文与右括号之后的位置。
fn call_arguments(source: &str, mut index: usize) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    while bytes
        .get(index)
        .is_some_and(|byte| byte.is_ascii_whitespace())
    {
        index += 1;
    }
    if bytes.get(index) == Some(&b')') {
        return Some((String::new(), index + 1));
    }
    if bytes.get(index) != Some(&b',') {
        return None;
    }
    index += 1;
    let start = index;
    let mut depth = 0usize;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'"' => {
                index = skip_string(source, index)?;
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b']' | b'}' => depth = depth.saturating_sub(1),
            b')' if depth == 0 => return Some((source[start..index].to_string(), index + 1)),
            b')' => depth -= 1,
            _ => {}
        }
        index += 1;
    }
    None
}

/// 从开引号跳到闭引号之后，`"` 里的逗号/括号不该影响实参切分。
fn skip_string(source: &str, mut index: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    index += 1;
    while let Some(&byte) = bytes.get(index) {
        match byte {
            b'\\' => index += 2,
            b'"' => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

/// 实参原文里顶层 `名字 = 值` 的名字，`==` / `<=` / `!=` 不算赋值。
fn named_arguments(arguments: &str) -> Vec<String> {
    let bytes = arguments.as_bytes();
    let mut names = Vec::new();
    let mut depth = 0usize;
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                index = skip_string(arguments, index).unwrap_or(bytes.len());
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            byte if depth == 0 && (byte.is_ascii_alphabetic() || byte == b'_') => {
                let start = index;
                while bytes
                    .get(index)
                    .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
                {
                    index += 1;
                }
                let mut lookahead = index;
                while bytes
                    .get(lookahead)
                    .is_some_and(|byte| byte.is_ascii_whitespace())
                {
                    lookahead += 1;
                }
                let is_assignment =
                    bytes.get(lookahead) == Some(&b'=') && bytes.get(lookahead + 1) != Some(&b'=');
                if is_assignment {
                    names.push(arguments[start..index].to_string());
                }
            }
            _ => index += 1,
        }
    }
    names
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

/// 在语言包文档里按点号定位一个键。
fn yaml_lookup<'a>(document: &'a Value, key: &str) -> Option<&'a Value> {
    let mut node = document;
    for segment in key.split('.') {
        node = node.get(segment)?;
    }
    Some(node)
}

/// 展平一个键节点下的字符串叶子：`(语言, 模板)`。
fn yaml_templates(node: &Value) -> Vec<(String, String)> {
    fn walk(node: &Value, path: &mut Vec<String>, out: &mut Vec<(String, String)>) {
        match node {
            Value::Mapping(map) => {
                for (key, value) in map {
                    let Some(segment) = key.as_str() else {
                        continue;
                    };
                    path.push(segment.to_string());
                    walk(value, path, out);
                    path.pop();
                }
            }
            Value::Null | Value::Sequence(_) => {}
            Value::String(text) => out.push((locale_label(path), text.clone())),
            // 数字/布尔叶子语言包里少见，但别让它们静默漏检。
            Value::Bool(value) => out.push((locale_label(path), value.to_string())),
            Value::Number(value) => out.push((locale_label(path), value.to_string())),
            Value::Tagged(tagged) => walk(&tagged.value, path, out),
        }
    }
    fn locale_label(path: &[String]) -> String {
        if path.is_empty() {
            "?".to_string()
        } else {
            path.join(".")
        }
    }
    let mut templates = Vec::new();
    walk(node, &mut Vec::new(), &mut templates);
    templates
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
