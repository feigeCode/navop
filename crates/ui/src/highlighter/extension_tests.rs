use std::{fs, io::Write, path::Path};

use tempfile::TempDir;

use crate::highlighter::{InstalledExtension, sha256_hex, verify_sha256};

fn write(path: &Path, content: &[u8]) {
    let mut f = fs::File::create(path).unwrap();
    f.write_all(content).unwrap();
}

fn make_extension(dir: &Path, name: &str) {
    fs::create_dir_all(dir).unwrap();
    let manifest = serde_json::json!({
        "name": name,
        "version": "0.1.0",
        "file_extensions": ["xx"],
    });
    write(
        &dir.join("manifest.json"),
        serde_json::to_string(&manifest).unwrap().as_bytes(),
    );
    write(&dir.join("parser.wasm"), &[0u8, 1, 2, 3]);
    write(&dir.join("highlights.scm"), b"; highlights");
}

#[test]
fn load_from_dir_reads_required_files() {
    let tmp = TempDir::new().unwrap();
    make_extension(tmp.path(), "demo");

    let ext = InstalledExtension::load_from_dir(tmp.path()).unwrap();
    assert_eq!(ext.manifest.name, "demo");
    assert_eq!(ext.manifest.version, "0.1.0");
    assert_eq!(ext.wasm_bytes, vec![0u8, 1, 2, 3]);
    assert_eq!(ext.highlights, "; highlights");
    assert_eq!(ext.injections, "");
    assert_eq!(ext.locals, "");
}

#[test]
fn load_from_dir_rejects_missing_manifest() {
    let tmp = TempDir::new().unwrap();
    let err = InstalledExtension::load_from_dir(tmp.path()).unwrap_err();
    assert!(err.to_string().contains("manifest.json"));
}

#[test]
fn load_from_dir_rejects_empty_name() {
    let tmp = TempDir::new().unwrap();
    write(
        &tmp.path().join("manifest.json"),
        br#"{"name": "", "version": "0.1.0"}"#,
    );
    write(&tmp.path().join("parser.wasm"), &[0u8]);
    let err = InstalledExtension::load_from_dir(tmp.path()).unwrap_err();
    assert!(err.to_string().contains("empty `name`"));
}

#[test]
fn sha256_hex_known_vector() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn verify_sha256_accepts_matching_value_and_prefix() {
    let bytes = b"hello world";
    let h = sha256_hex(bytes);
    verify_sha256(bytes, &h).unwrap();
    verify_sha256(bytes, &format!("sha256:{h}")).unwrap();
    verify_sha256(bytes, &h.to_uppercase()).unwrap();
}

#[test]
fn verify_sha256_rejects_mismatch() {
    let err = verify_sha256(b"hello", &"0".repeat(64)).unwrap_err();
    assert!(err.to_string().contains("sha256 mismatch"));
}

#[test]
fn verify_sha256_rejects_invalid_length() {
    let err = verify_sha256(b"x", "abcd").unwrap_err();
    assert!(err.to_string().contains("invalid sha256 length"));
}

#[test]
fn load_from_dir_with_correct_sha256_passes() {
    let tmp = TempDir::new().unwrap();
    let wasm = vec![0u8, 1, 2, 3, 4, 5];
    let hash = sha256_hex(&wasm);
    let manifest = serde_json::json!({
        "name": "demo",
        "version": "0.1.0",
        "sha256_wasm": hash,
    });
    write(
        &tmp.path().join("manifest.json"),
        serde_json::to_string(&manifest).unwrap().as_bytes(),
    );
    write(&tmp.path().join("parser.wasm"), &wasm);

    let ext = InstalledExtension::load_from_dir(tmp.path()).unwrap();
    assert_eq!(ext.wasm_bytes, wasm);
}

#[test]
fn load_from_dir_with_wrong_sha256_fails() {
    let tmp = TempDir::new().unwrap();
    let manifest = serde_json::json!({
        "name": "demo",
        "version": "0.1.0",
        "sha256_wasm": "0".repeat(64),
    });
    write(
        &tmp.path().join("manifest.json"),
        serde_json::to_string(&manifest).unwrap().as_bytes(),
    );
    write(&tmp.path().join("parser.wasm"), &[1u8, 2, 3]);

    let err = InstalledExtension::load_from_dir(tmp.path()).unwrap_err();
    assert!(format!("{err:?}").contains("sha256 mismatch"));
}
