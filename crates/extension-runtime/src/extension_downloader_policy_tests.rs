use std::{fs, sync::Arc};

use crate::{
    extension::{CompositeExtensionProvider, ExtensionKind, ExtensionRegistry},
    extension_downloader::{install_from_staging_with_high_risk_permissions, stage_local_tarball},
};

#[test]
fn stage_local_tarball_supports_permission_approved_composite_install() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tarball = tmp.path().join("shell.tar.gz");
    write_composite_tarball(&tarball);
    let staging = stage_local_tarball(&tarball).unwrap();
    let root = tmp.path().join("extensions");
    let mut registry = ExtensionRegistry::new(root.clone());
    registry.register_provider(Arc::new(CompositeExtensionProvider));

    let summary = install_from_staging_with_high_risk_permissions(
        &staging,
        &registry,
        Some(ExtensionKind::Composite),
    )
    .unwrap();
    let _ = fs::remove_dir_all(&staging);

    assert_eq!("acme.shell", summary.name);
    assert!(root.join("composite/acme.shell/extension.json").exists());
}

fn write_composite_tarball(path: &std::path::Path) {
    fs::write(path, composite_tarball_bytes()).unwrap();
}

fn composite_tarball_bytes() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_bytes(
        &mut archive,
        "extension.json",
        br#"{
            "schema_version": 1,
            "id": "acme.shell",
            "name": "Shell Runner",
            "version": "1.0.0",
            "engines": { "onetcli": ">=0.0.0" },
            "permissions": ["shell:exec"]
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
