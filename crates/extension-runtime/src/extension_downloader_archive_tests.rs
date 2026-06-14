use std::{fs, sync::Arc};

use crate::extension::{DatabaseDriverExtensionProvider, ExtensionKind, ExtensionRegistry};
use crate::extension_downloader::{install_from_staging_generic, stage_local_tarball};

#[test]
fn stage_local_tarball_then_install_staging_installs_database_driver() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tarball = tmp.path().join("driver.tar.gz");
    write_database_driver_tarball(&tarball);

    let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let staging = stage_local_tarball(&tarball).unwrap();
    let summary =
        install_from_staging_generic(&staging, &registry, Some(ExtensionKind::DatabaseDriver))
            .unwrap();
    let _ = fs::remove_dir_all(staging);

    assert_eq!(ExtensionKind::DatabaseDriver, summary.kind);
    assert_eq!("fake_pg", summary.name);
    assert!(summary.path.join("driver.json").exists());
    assert!(summary.path.join("driver-bin").exists());
}

#[cfg(unix)]
#[test]
fn install_from_staging_generic_rejects_symlinks_in_staging() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().join("extensions");
    let outside = tmp.path().join("outside-driver-bin");
    let staging = tmp.path().join("staging");
    fs::create_dir_all(&staging).unwrap();
    write_driver_manifest(&staging, "fake_pg", "Fake PostgreSQL");
    fs::write(&outside, b"outside").unwrap();
    symlink(&outside, staging.join("driver-bin")).unwrap();

    let mut registry = ExtensionRegistry::new(root.clone());
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let err =
        install_from_staging_generic(&staging, &registry, Some(ExtensionKind::DatabaseDriver))
            .unwrap_err();

    assert!(err.to_string().contains("symlink"));
    assert!(!root.join("database_drivers/fake_pg").exists());
}

fn write_driver_manifest(dir: &std::path::Path, id: &str, name: &str) {
    fs::write(
        dir.join("driver.json"),
        format!(
            r#"{{
                "id": "{id}",
                "name": "{name}",
                "description": "Test database driver",
                "version": "1.2.3",
                "entry": {{ "command": "./driver-bin" }},
                "transport": {{ "name": "{id}.sock" }}
            }}"#
        ),
    )
    .unwrap();
}

fn write_database_driver_tarball(path: &std::path::Path) {
    fs::write(path, database_driver_tarball_bytes()).unwrap();
}

fn database_driver_tarball_bytes() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_bytes(&mut archive, "driver-bin", b"driver");
    append_bytes(
        &mut archive,
        "driver.json",
        br#"{
            "id": "fake_pg",
            "name": "Fake PostgreSQL",
            "description": "Test database driver",
            "version": "1.2.3",
            "entry": { "command": "./driver-bin" },
            "transport": { "name": "fake_pg.sock" }
        }"#,
    );
    let encoder = archive.into_inner().unwrap();
    encoder.finish().unwrap()
}

fn append_bytes(
    archive: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
) {
    let mut header = tar::Header::new_gnu();
    header.set_path(name).unwrap();
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    archive.append(&header, bytes).unwrap();
}
