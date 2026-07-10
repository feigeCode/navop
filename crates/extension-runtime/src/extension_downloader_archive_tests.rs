use std::{fs, sync::Arc};

use crate::extension::{
    DatabaseDriverExtensionProvider, ExtensionKind, ExtensionRegistry, McpHelperExtensionProvider,
    RemoteDesktopProviderExtensionProvider,
};
use crate::extension_downloader::{
    detect_package_kind, install_from_staging_generic, stage_local_tarball,
};

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

#[test]
fn stage_local_tarball_supports_single_top_level_driver_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tarball = tmp.path().join("duckdb-driver.tar.gz");
    fs::write(&tarball, wrapped_database_driver_tarball_bytes()).unwrap();

    let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let staging = stage_local_tarball(&tarball).unwrap();
    assert_eq!(
        ExtensionKind::DatabaseDriver,
        detect_package_kind(&staging).unwrap()
    );

    let summary = install_from_staging_generic(&staging, &registry, None).unwrap();
    let _ = fs::remove_dir_all(staging);

    assert_eq!(ExtensionKind::DatabaseDriver, summary.kind);
    assert_eq!("duckdb", summary.name);
    assert!(summary.path.join("driver.json").exists());
    assert!(summary.path.join("duckdb_driver").exists());
    assert!(!summary.path.join("duckdb").exists());
}

#[test]
fn stage_local_tarball_then_install_staging_installs_remote_desktop_provider() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tarball = tmp.path().join("rdp-provider.tar.gz");
    fs::write(&tarball, remote_desktop_provider_tarball_bytes()).unwrap();

    let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
    registry.register_provider(Arc::new(RemoteDesktopProviderExtensionProvider));

    let staging = stage_local_tarball(&tarball).unwrap();
    assert_eq!(
        ExtensionKind::RemoteDesktopProvider,
        detect_package_kind(&staging).unwrap()
    );
    let summary = install_from_staging_generic(&staging, &registry, None).unwrap();
    let _ = fs::remove_dir_all(staging);

    assert_eq!(ExtensionKind::RemoteDesktopProvider, summary.kind);
    assert_eq!("rdp", summary.name);
    assert!(summary.path.join("remote_desktop_provider.json").exists());
    assert!(summary.path.join("onetcli-rdp-helper").exists());
}

#[test]
fn stage_local_tarball_then_install_staging_installs_mcp_helper() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tarball = tmp.path().join("mcp-helper.tar.gz");
    fs::write(&tarball, mcp_helper_tarball_bytes()).unwrap();

    let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
    registry.register_provider(Arc::new(McpHelperExtensionProvider));

    let staging = stage_local_tarball(&tarball).unwrap();
    assert_eq!(
        ExtensionKind::McpHelper,
        detect_package_kind(&staging).unwrap()
    );
    let summary = install_from_staging_generic(&staging, &registry, None).unwrap();
    let _ = fs::remove_dir_all(staging);

    assert_eq!(ExtensionKind::McpHelper, summary.kind);
    assert_eq!("onetcli-public-mcp", summary.name);
    assert!(summary.path.join("mcp_helper.json").exists());
    assert!(summary.path.join("onetcli-public-mcp").exists());
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

fn wrapped_database_driver_tarball_bytes() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_bytes(&mut archive, "._duckdb", b"appledouble metadata");
    append_bytes(&mut archive, "duckdb/duckdb_driver", b"driver");
    append_bytes(
        &mut archive,
        "duckdb/._driver.json",
        b"appledouble metadata",
    );
    append_bytes(
        &mut archive,
        "duckdb/driver.json",
        br#"{
            "id": "duckdb",
            "name": "DuckDB",
            "description": "DuckDB IPC driver",
            "version": "1.0.0",
            "entry": { "command": "./duckdb_driver" },
            "transport": { "name": "duckdb.sock" }
        }"#,
    );
    let encoder = archive.into_inner().unwrap();
    encoder.finish().unwrap()
}

fn remote_desktop_provider_tarball_bytes() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_bytes(&mut archive, "onetcli-rdp-helper", b"helper");
    append_bytes(
        &mut archive,
        "remote_desktop_provider.json",
        br#"{
            "id": "rdp",
            "name": "RDP",
            "description": "RDP provider",
            "version": "1.2.3",
            "protocol": "rdp",
            "entry": { "command": "./onetcli-rdp-helper" },
            "capabilities": {
                "resize": "remote_resize",
                "clipboard_text": true,
                "cursor_shape": true,
                "audio": false,
                "file_transfer": false
            },
            "ui": {
                "icon": "Monitor",
                "default_port": 3389
            }
        }"#,
    );
    let encoder = archive.into_inner().unwrap();
    encoder.finish().unwrap()
}

fn mcp_helper_tarball_bytes() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_executable_bytes(&mut archive, "onetcli-public-mcp", b"helper");
    append_bytes(
        &mut archive,
        "mcp_helper.json",
        br#"{
            "id": "onetcli-public-mcp",
            "name": "Navop MCP Helper",
            "description": "OnetCli MCP stdio bridge",
            "version": "1.2.3",
            "entry": { "command": "./onetcli-public-mcp" }
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
    append_bytes_with_mode(archive, name, bytes, 0o644);
}

fn append_executable_bytes(
    archive: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
) {
    append_bytes_with_mode(archive, name, bytes, 0o755);
}

fn append_bytes_with_mode(
    archive: &mut tar::Builder<flate2::write::GzEncoder<Vec<u8>>>,
    name: &str,
    bytes: &[u8],
    mode: u32,
) {
    let mut header = tar::Header::new_gnu();
    header.set_path(name).unwrap();
    header.set_mode(mode);
    header.set_size(bytes.len() as u64);
    header.set_cksum();
    archive.append(&header, bytes).unwrap();
}
