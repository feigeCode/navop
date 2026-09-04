use std::path::{Path, PathBuf};

#[test]
fn configured_dialog_buttons_enable_the_default_footer() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = Vec::new();
    collect_rust_files(&workspace.join("main/src"), &mut files);
    collect_rust_files(&workspace.join("crates"), &mut files);

    let mut missing = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("Rust source should be readable");
        let lines = source.lines().collect::<Vec<_>>();
        for (index, line) in lines.iter().enumerate() {
            if !line.contains(".button_props(") || line.contains(".find(") {
                continue;
            }
            let context = lines[index.saturating_sub(12)..index].join("\n");
            if !context.contains(".confirm()") && !context.contains(".alert()") {
                missing.push(format!("{}:{}", file.display(), index + 1));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "Dialog button_props requires confirm() or alert(): {}",
        missing.join(", ")
    );
}

fn collect_rust_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}
