use std::{collections::HashMap, fs, io::Write, path::Path};

use tempfile::TempDir;

use crate::highlighter::{
    HighlightTheme, InstalledExtension, LanguageConfig, LanguageRegistry, SyntaxHighlighter,
    list_installed, load_extensions_dir, register_extension_manifests_dir,
};

use super::extension_loader::topological_sort;

fn make_extension(dir: &Path, name: &str, requires: &[&str]) {
    fs::create_dir_all(dir).unwrap();
    let manifest = serde_json::json!({
        "name": name,
        "version": "0.1.0",
        "requires": requires,
    });
    let mut manifest_file = fs::File::create(dir.join("manifest.json")).unwrap();
    manifest_file
        .write_all(serde_json::to_string(&manifest).unwrap().as_bytes())
        .unwrap();
    let mut wasm = fs::File::create(dir.join("parser.wasm")).unwrap();
    wasm.write_all(&[0u8; 4]).unwrap();
}

fn load_test_extensions(root: &Path, names: &[&str]) -> HashMap<String, InstalledExtension> {
    let mut exts = HashMap::new();
    for name in names {
        exts.insert(
            (*name).to_string(),
            InstalledExtension::load_from_dir(&root.join(name)).unwrap(),
        );
    }
    exts
}

#[test]
fn load_extensions_dir_returns_empty_when_root_missing() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().join("does-not-exist");
    let report = load_extensions_dir(&root, LanguageRegistry::singleton()).unwrap();
    assert!(report.is_empty());
}

#[test]
fn load_extensions_dir_topological_order() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    make_extension(&root.join("alpha"), "alpha", &["beta"]);
    make_extension(&root.join("beta"), "beta", &[]);

    let order = topological_sort(&load_test_extensions(root, &["alpha", "beta"])).unwrap();
    let pos_alpha = order.iter().position(|n| n == "alpha").unwrap();
    let pos_beta = order.iter().position(|n| n == "beta").unwrap();
    assert!(pos_beta < pos_alpha, "beta should load before alpha");
}

#[test]
fn topological_sort_detects_cycles() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    make_extension(&root.join("a"), "a", &["b"]);
    make_extension(&root.join("b"), "b", &["a"]);

    let err = topological_sort(&load_test_extensions(root, &["a", "b"])).unwrap_err();
    assert!(err.to_string().contains("cyclic"));
}

#[test]
fn topological_sort_tolerates_missing_dependency() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    make_extension(&root.join("alpha"), "alpha", &["nonexistent"]);

    let order = topological_sort(&load_test_extensions(root, &["alpha"])).unwrap();
    assert_eq!(order, vec!["alpha".to_string()]);
}

#[test]
fn list_installed_returns_summaries_sorted() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    make_extension(&root.join("zoo"), "zoo", &[]);
    make_extension(&root.join("apple"), "apple", &[]);

    let list = list_installed(root).unwrap();
    let names: Vec<_> = list.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, vec!["apple".to_string(), "zoo".to_string()]);
    assert_eq!(list[0].version, "0.1.0");
}

#[test]
fn list_installed_returns_empty_when_root_missing() {
    let tmp = TempDir::new().unwrap();
    let list = list_installed(&tmp.path().join("does-not-exist")).unwrap();
    assert!(list.is_empty());
}

#[test]
fn list_installed_skips_dirs_without_manifest() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    fs::create_dir_all(root.join("empty")).unwrap();
    make_extension(&root.join("valid"), "valid", &[]);

    let list = list_installed(root).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "valid");
}

#[test]
fn register_extension_manifests_dir_registers_file_extensions_without_loading_wasm() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    let ext_dir = root.join("__test_lazy_manifest__");
    fs::create_dir_all(&ext_dir).unwrap();
    let manifest = serde_json::json!({
        "name": "__test_lazy_manifest__",
        "version": "0.1.0",
        "file_extensions": ["lazy_manifest"],
    });
    fs::write(
        ext_dir.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();

    let registry = LanguageRegistry::singleton();
    let report = register_extension_manifests_dir(root, registry).unwrap();

    assert!(
        report
            .loaded
            .contains(&"__test_lazy_manifest__".to_string())
    );
    assert_eq!(
        registry
            .language_name_for_extension("lazy_manifest")
            .as_deref(),
        Some("__test_lazy_manifest__")
    );
    assert!(registry.unregister("__test_lazy_manifest__"));
}

#[test]
fn uninstall_removes_directory_and_unregisters() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path();
    make_extension(&root.join("foo_ext"), "__test_uninstall_foo__", &[]);
    let dir = root.join("foo_ext");
    let registry = LanguageRegistry::singleton();
    registry.register(
        "__test_uninstall_foo__",
        &LanguageConfig::new(
            "__test_uninstall_foo__",
            tree_sitter_json::LANGUAGE.into(),
            vec![],
            "",
            "",
            "",
        ),
    );

    let removed_name = InstalledExtension::uninstall(&dir, registry).unwrap();
    assert_eq!(removed_name, "__test_uninstall_foo__");
    assert!(!dir.exists());
    assert!(!registry.unregister("__test_uninstall_foo__"));
}

#[test]
#[ignore = "需要 ONETCLI_TEST_WASM_RUST 指向真实 wasm fixture"]
fn load_extensions_dir_loads_real_wasm_extension() {
    let Ok(wasm_path) = std::env::var("ONETCLI_TEST_WASM_RUST") else {
        eprintln!("ONETCLI_TEST_WASM_RUST not set; skipping");
        return;
    };

    let tmp = TempDir::new().unwrap();
    let ext_dir = tmp.path().join("rust");
    fs::create_dir_all(&ext_dir).unwrap();
    let manifest = serde_json::json!({
        "name": "rust",
        "version": "0.24.0",
        "file_extensions": ["rs"],
    });
    fs::write(
        ext_dir.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(ext_dir.join("parser.wasm"), fs::read(&wasm_path).unwrap()).unwrap();
    fs::write(
        ext_dir.join("highlights.scm"),
        "(identifier) @variable\n(string_literal) @string\n",
    )
    .unwrap();

    let report = load_extensions_dir(tmp.path(), LanguageRegistry::singleton()).unwrap();
    assert!(report.loaded.contains(&"rust".to_string()));

    let mut hl = SyntaxHighlighter::new("rust");
    assert_eq!(hl.language().as_ref(), "rust");
    assert!(hl.update(None, &ropey::Rope::from_str("fn x() {}"), None));
}

#[test]
#[ignore = "需要 ONETCLI_TEST_LANGUAGE_EXT 指向真实语言扩展目录"]
fn language_extension_smoke_highlights_real_extension() {
    let Ok(ext_dir) = std::env::var("ONETCLI_TEST_LANGUAGE_EXT") else {
        eprintln!("ONETCLI_TEST_LANGUAGE_EXT not set; skipping");
        return;
    };
    let ext_dir = std::path::PathBuf::from(ext_dir);
    let manifest = crate::highlighter::extension::read_manifest_only(&ext_dir).unwrap();
    let registry = LanguageRegistry::singleton();
    registry.register_wasm_manifest(manifest.clone(), ext_dir);

    let language = registry
        .language_name_for_extension(&manifest.file_extensions[0])
        .unwrap();
    assert_eq!(language, manifest.name);

    let html = "<!doctype html><html><body><script>var x = 1;</script></body></html>";
    let mut highlighter = SyntaxHighlighter::new(&manifest.name);
    assert_eq!(highlighter.language().as_ref(), manifest.name.as_str());
    assert!(highlighter.update(None, &ropey::Rope::from_str(html), None));
    let styles = highlighter.styles(&(0..html.len()), &HighlightTheme::default_dark());
    assert!(
        styles.iter().any(|(_, style)| style.color.is_some()),
        "expected real highlight styles for {:?}",
        manifest.name
    );
    registry.unregister(&manifest.name);
}
