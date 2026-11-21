use semver::Version;

use super::ComponentManifest;
use crate::X11ComponentError;

fn parse(text: &str) -> Result<ComponentManifest, X11ComponentError> {
    ComponentManifest::parse(text, &Version::new(0, 23, 4))
}

fn valid_manifest() -> String {
    serde_json::json!({
        "schema": 1,
        "components": {
            "x11-server": {
                "channel": "stable",
                "pinned_for_navop": ">=0.16.0, <1.0.0",
                "rollout_percent": 25,
                "artifacts": {
                    "windows-x86_64": artifact(),
                    "windows-aarch64": artifact()
                }
            }
        }
    })
    .to_string()
}

fn artifact() -> serde_json::Value {
    serde_json::json!({
        "version": "0.15.2",
        "sha256": "a".repeat(64),
        "signature": "c2ln",
        "min_vulkan": "1.3.0",
        "format": "zip",
        "executable_path": "bin/yserver.exe"
    })
}

fn change_manifest(update: impl FnOnce(&mut serde_json::Value)) -> String {
    let mut value: serde_json::Value = serde_json::from_str(&valid_manifest()).unwrap();
    update(&mut value);
    value.to_string()
}

#[test]
fn parses_stable_channel_and_platform_artifact() {
    let manifest = parse(&valid_manifest()).unwrap();
    assert_eq!(manifest.schema, 1);
    assert_eq!(
        manifest.artifact("windows-aarch64").unwrap().version,
        "0.15.2"
    );
    assert!(matches!(
        manifest.artifact("linux-x86_64"),
        Err(X11ComponentError::MissingArtifact(_))
    ));
}

#[test]
fn pinned_requirement_matches_navop_version() {
    parse(&valid_manifest()).expect("yserver 0.15.2 不应被要求满足 Navop >=0.16");
    let error = ComponentManifest::parse(&valid_manifest(), &Version::new(0, 15, 9))
        .expect_err("旧 Navop 必须被拒绝");
    assert!(error.to_string().contains("pinned_for_navop"));
    let invalid = change_manifest(|value| {
        value["components"]["x11-server"]["artifacts"]["windows-x86_64"]["version"] =
            "latest".into();
    });
    assert!(parse(&invalid).is_err());
}

#[test]
fn rejects_unknown_schema_and_nonstable_channel() {
    for text in [
        change_manifest(|value| value["schema"] = 2.into()),
        change_manifest(|value| value["components"]["x11-server"]["channel"] = "beta".into()),
    ] {
        assert!(matches!(
            parse(&text),
            Err(X11ComponentError::InvalidManifest(_))
        ));
    }
}

#[test]
fn artifact_format_and_executable_path_are_validated() {
    for path in ["", "../yserver.exe", "/tmp/yserver", "bin/../../yserver"] {
        let text = change_manifest(|value| {
            value["components"]["x11-server"]["artifacts"]["windows-x86_64"]["executable_path"] =
                path.into();
        });
        assert!(parse(&text).is_err(), "危险路径：{path}");
    }
    let missing = change_manifest(|value| {
        let artifact = &mut value["components"]["x11-server"]["artifacts"]["windows-x86_64"];
        artifact.as_object_mut().unwrap().remove("format");
    });
    assert!(parse(&missing).is_err());
}

#[test]
fn duplicate_target_is_rejected_instead_of_overwritten() {
    let artifact = artifact().to_string();
    let text = format!(
        r#"{{
            "schema": 1,
            "components": {{
                "x11-server": {{
                    "channel": "stable",
                    "pinned_for_navop": ">=0.16.0, <1.0.0",
                    "artifacts": {{
                        "windows-x86_64": {artifact},
                        "windows-x86_64": {artifact}
                    }}
                }}
            }}
        }}"#
    );
    let error = parse(&text).unwrap_err();
    assert!(
        error.to_string().contains("重复"),
        "重复 target 必须 fail-closed：{error}"
    );
}

#[test]
fn runtime_manifest_rejects_legacy_url_metadata() {
    let text = change_manifest(|value| {
        value["components"]["x11-server"]["artifacts"]["windows-x86_64"]["url"] =
            "https://example.invalid/yserver.zip".into();
    });
    assert!(matches!(
        parse(&text),
        Err(X11ComponentError::InvalidManifest(_))
    ));
}

#[test]
fn artifact_hash_vulkan_and_rollout_are_validated() {
    let cases = [
        ("sha256", "not-sha".into()),
        ("min_vulkan", "latest".into()),
    ];
    for (field, value) in cases {
        let text = change_manifest(|root| {
            root["components"]["x11-server"]["artifacts"]["windows-x86_64"][field] = value;
        });
        assert!(parse(&text).is_err(), "字段：{field}");
    }
    let text = change_manifest(|value| {
        value["components"]["x11-server"]["rollout_percent"] = 101.into();
    });
    assert!(parse(&text).is_err());
}
