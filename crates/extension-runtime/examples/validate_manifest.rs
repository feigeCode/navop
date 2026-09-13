use std::path::PathBuf;

use extension_runtime::{ExtensionRuntimeCatalog, extension::manifest::load_from_dir};

fn main() {
    let mut args = std::env::args_os().skip(1);
    let Some(extension_dir) = args.next().map(PathBuf::from) else {
        eprintln!("usage: validate_manifest <extension-dir>");
        std::process::exit(2);
    };
    extension_runtime::set_current_host_version("0.17.0")
        .expect("validator host version must be valid");
    let manifest = load_from_dir(&extension_dir).unwrap_or_else(|error| {
        eprintln!("manifest validation failed: {error}");
        std::process::exit(1);
    });
    let id = manifest.id.clone();
    let workbench_count = manifest.contributes.resource_workbenches.len();
    ExtensionRuntimeCatalog::from_manifests(vec![manifest]).unwrap_or_else(|error| {
        eprintln!("catalog registration failed: {error}");
        std::process::exit(1);
    });
    println!("validated {id}: {workbench_count} resource workbench(es)");
}
