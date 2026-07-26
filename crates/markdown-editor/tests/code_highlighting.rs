use gpui_component::highlighter::{
    HighlightTheme, InstalledExtension, LanguageKind, LanguageRegistry, SyntaxHighlighter,
};
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

    let registry = LanguageRegistry::singleton();
    registry.register_wasm_manifest(extension.manifest.clone(), extension_dir);
    assert_eq!(
        Some("rust".to_string()),
        registry.resolve_language_name("rs"),
        "the manifest file extension must canonicalize the fenced alias"
    );

    let rust = registry
        .language("rust")
        .expect("registered Rust wasm grammar must lazy-load");
    let LanguageKind::Wasm { wasm_bytes } = &rust.kind else {
        panic!("the registered Rust grammar must be backed by parser.wasm");
    };
    assert_eq!(extension.wasm_bytes.as_slice(), wasm_bytes.as_ref());
    assert_eq!(extension.highlights, rust.highlights.as_ref());
    assert_eq!(extension.injections, rust.injections.as_ref());
    assert_eq!(extension.locals, rust.locals.as_ref());
    assert!(
        !rust.highlights.is_empty(),
        "the wasm extension must supply highlights.scm"
    );
    assert_eq!(Some(true), registry.is_wasm("rust"));

    let source = "fn main() {\n    let answer = 42;\n}\n";
    let text = Rope::from_str(source);
    let mut highlighter = SyntaxHighlighter::new("rust");
    assert_eq!("rust", highlighter.language().as_ref());
    assert!(highlighter.update(None, &text, None));

    let styles = highlighter.styles(&(0..source.len()), &HighlightTheme::default_dark());
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "the wasm grammar and highlights.scm must produce colored Rust spans"
    );
    assert!(registry.unregister("rust"));
}
