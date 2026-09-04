//! 语法高亮契约测试。
//!
//! 外部 gpui-component 已移除 highlighter 的 wasm 扩展 API
//! (`InstalledExtension`/`LanguageKind`/`register_wasm_manifest` 等),
//! 语言扩展加载统一由本仓库 `extension-runtime` 提供:它把 parser.wasm
//! 经 tree-sitter WasmStore 编译成原生 `tree_sitter::Language` 后注册进
//! `LanguageRegistry`。本测试按新链路验证 fenced Rust 代码块的着色。

use extension_runtime::language_extensions::InstalledExtension;
use gpui_component::highlighter::{HighlightTheme, LanguageRegistry, SyntaxHighlighter};
use ropey::Rope;
use std::path::PathBuf;

#[test]
fn markdown_editor_does_not_bundle_a_native_rust_grammar() {
    let registry = LanguageRegistry::singleton();
    assert!(
        registry.language("rust").is_none(),
        "fenced languages must be supplied by wasm extensions, not native Cargo features"
    );
    assert_eq!(
        "text",
        SyntaxHighlighter::new("rust").language().as_ref(),
        "an unavailable fenced language must safely fall back to text"
    );
}

#[test]
#[ignore = "需要 ONETCLI_TEST_LANGUAGE_EXT 指向真实 Rust 语言扩展目录"]
fn fenced_rust_code_loads_parser_and_queries_from_wasm_extension() {
    let extension_dir = PathBuf::from(
        std::env::var("ONETCLI_TEST_LANGUAGE_EXT")
            .expect("ONETCLI_TEST_LANGUAGE_EXT must point to a Rust language extension"),
    );
    let extension = InstalledExtension::load_from_dir(&extension_dir)
        .expect("real language extension must contain manifest, parser.wasm, and valid queries");
    assert_eq!("rust", extension.manifest.name);

    // 注册:wasm 语法经 tree-sitter WasmStore 编译,连同文件扩展名别名一起写入注册表
    let registry = LanguageRegistry::singleton();
    extension
        .register(registry)
        .expect("registering the rust wasm grammar must compile parser.wasm");
    assert!(
        registry.language("rs").is_some(),
        "the manifest file extension must canonicalize the fenced alias"
    );

    let rust = registry
        .language("rust")
        .expect("registered Rust wasm grammar must resolve");
    assert!(
        !rust.highlights.is_empty(),
        "the wasm extension must supply highlights.scm"
    );

    let source = "fn main() {\n    let answer = 42;\n}\n";
    let text = Rope::from_str(source);
    let mut highlighter = SyntaxHighlighter::new("rust");
    assert_eq!("rust", highlighter.language().as_ref());
    assert!(highlighter.update(None, &text, None));

    let theme = HighlightTheme::default_dark();
    let styles = highlighter.styles(&(0..source.len()), &*theme);
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "the wasm grammar and highlights.scm must produce colored Rust spans"
    );
    // 注意:新 LanguageRegistry 没有 unregister,该 #[ignore] 测试会向进程级
    // 单例注册 rust 语法,与其他测试隔离运行即可。
}
