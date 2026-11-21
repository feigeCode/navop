use std::path::{Path, PathBuf};

use super::{atomic_replace, rewrite_macos_icd};

fn fixture(name: &str) -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join(format!(
        "navop-x11-icd-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let executable = root.join("yserver");
    let package = executable.parent().unwrap();
    std::fs::create_dir_all(package.join("lib")).unwrap();
    std::fs::create_dir_all(package.join("icd.d")).unwrap();
    std::fs::write(&executable, b"yserver").unwrap();
    std::fs::write(package.join("lib/libvulkan_kosmickrisp.dylib"), b"driver").unwrap();
    std::fs::write(
        package.join("icd.d/kosmickrisp_mesa_icd.aarch64.json"),
        br#"{
  "ICD": {
    "library_path": "@@YSERVER_ROOT@@/lib/libvulkan_kosmickrisp.dylib",
    "api_version": "1.3.0"
  },
  "file_format_version": "1.0.0"
}
"#,
    )
    .unwrap();
    (root, executable)
}

fn library_path(executable: &Path) -> String {
    let document: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            executable
                .parent()
                .unwrap()
                .join("icd.d/kosmickrisp_mesa_icd.aarch64.json"),
        )
        .unwrap(),
    )
    .unwrap();
    document["ICD"]["library_path"].as_str().unwrap().into()
}

#[test]
fn rewrite_is_idempotent_and_recovers_after_install_root_moves() {
    let (root, executable) = fixture("restart-move");
    rewrite_macos_icd(&executable).unwrap();
    let first = library_path(&executable);
    assert_eq!(
        first,
        executable
            .parent()
            .unwrap()
            .join("lib/libvulkan_kosmickrisp.dylib")
            .display()
            .to_string()
    );

    rewrite_macos_icd(&executable).unwrap();
    assert_eq!(library_path(&executable), first);

    let moved = root.with_extension("moved");
    std::fs::rename(&root, &moved).unwrap();
    let moved_executable = moved.join("yserver");
    rewrite_macos_icd(&moved_executable).unwrap();
    assert_eq!(
        library_path(&moved_executable),
        moved
            .join("lib/libvulkan_kosmickrisp.dylib")
            .display()
            .to_string()
    );
    std::fs::remove_dir_all(moved).unwrap();
}

#[test]
fn failed_atomic_write_preserves_the_previous_complete_icd() {
    let (root, executable) = fixture("write-failure");
    let icd = executable
        .parent()
        .unwrap()
        .join("icd.d/kosmickrisp_mesa_icd.aarch64.json");
    let original = std::fs::read(&icd).unwrap();
    let temporary_directory = root.join("temporary-is-a-directory");
    std::fs::create_dir(&temporary_directory).unwrap();

    let error = atomic_replace(&icd, b"{\"partial\":", &temporary_directory).unwrap_err();
    assert!(error.to_string().contains("安装失败"));
    assert_eq!(std::fs::read(&icd).unwrap(), original);
    rewrite_macos_icd(&executable).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn truncated_icd_fails_instead_of_being_mistaken_for_a_valid_path() {
    let (root, executable) = fixture("truncated");
    let icd = executable
        .parent()
        .unwrap()
        .join("icd.d/kosmickrisp_mesa_icd.aarch64.json");
    std::fs::write(&icd, b"{\"ICD\":").unwrap();
    assert!(rewrite_macos_icd(&executable).is_err());
    std::fs::remove_dir_all(root).unwrap();
}
