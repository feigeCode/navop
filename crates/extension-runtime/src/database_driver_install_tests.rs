use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
};

use extension_protocol::error::{ProtocolError, error_codes};
use futures::FutureExt;
use gpui::http_client::{self, AsyncBody, HttpClient, Url, http};
use one_core::storage::{DatabaseType, DbConnectionConfig};

use crate::database_driver_install::{
    DriverRequirement, NativeDriverBackend, NativeDriverRequirement,
    fallback_native_driver_for_error, find_database_driver_entry,
    install_database_driver_from_marketplace_with_registry, required_driver_for_config,
    required_native_driver,
};
use crate::extension::{DatabaseDriverExtensionProvider, ExtensionKind, ExtensionRegistry};
use crate::extension_downloader::MarketplaceEntry;

#[test]
fn builtin_mysql_does_not_require_marketplace_driver() {
    assert_eq!(
        DriverRequirement::NotRequired,
        required_driver_for_config(&config(DatabaseType::MySQL))
    );
}

#[test]
fn legacy_fallback_requires_structured_server_incompatibility() {
    let incompatible = ProtocolError::new(
        error_codes::SERVER_INCOMPATIBLE,
        "server wire version is too old",
    );
    assert_eq!(
        Some(NativeDriverRequirement::Required {
            api: "mongodb".to_string(),
            driver_id: "mongodb-legacy".to_string(),
        }),
        fallback_native_driver_for_error("mongodb", "mongodb-legacy", &incompatible)
    );

    let auth = ProtocolError::new(error_codes::AUTH_FAILED, "bad password");
    assert_eq!(
        None,
        fallback_native_driver_for_error("mongodb", "mongodb-legacy", &auth)
    );
}

#[test]
fn native_driver_requirement_is_shared_by_redis_and_mongodb() {
    assert_eq!(
        NativeDriverRequirement::NotRequired,
        required_native_driver("redis", NativeDriverBackend::Builtin)
    );
    assert_eq!(
        NativeDriverRequirement::Required {
            api: "redis".to_string(),
            driver_id: "redis".to_string(),
        },
        required_native_driver(
            "redis",
            NativeDriverBackend::Ipc {
                driver_id: "redis".to_string(),
            },
        )
    );
    assert_eq!(
        NativeDriverRequirement::Required {
            api: "mongodb".to_string(),
            driver_id: "mongodb-modern".to_string(),
        },
        required_native_driver(
            "mongodb",
            NativeDriverBackend::Ipc {
                driver_id: "mongodb-modern".to_string(),
            },
        )
    );
}

#[test]
fn duckdb_requires_duckdb_marketplace_driver() {
    assert_eq!(
        DriverRequirement::Required {
            driver_id: "duckdb".to_string()
        },
        required_driver_for_config(&config(DatabaseType::DuckDB))
    );
}

#[test]
fn external_database_requires_its_driver_id() {
    assert_eq!(
        DriverRequirement::Required {
            driver_id: "custom".to_string()
        },
        required_driver_for_config(&external_config(" custom "))
    );
}

#[test]
fn external_database_without_driver_id_is_invalid() {
    assert!(matches!(
        required_driver_for_config(&external_config("")),
        DriverRequirement::InvalidConfig { .. }
    ));
}

#[test]
fn external_database_does_not_fallback_to_extra_params_driver_id() {
    let mut config = external_config("");
    config
        .extra_params
        .insert("external_driver_id".to_string(), "custom".to_string());

    assert!(matches!(
        required_driver_for_config(&config),
        DriverRequirement::InvalidConfig { .. }
    ));
}

#[test]
fn find_database_driver_entry_matches_database_driver_kind_and_id() {
    let entries = vec![
        entry("duckdb", ExtensionKind::Language),
        entry("custom", ExtensionKind::DatabaseDriver),
    ];

    let found = find_database_driver_entry(&entries, "custom");

    assert_eq!(Some("custom"), found.map(|entry| entry.id.as_str()));
}

#[test]
fn install_database_driver_from_marketplace_installs_matching_entry() {
    let tmp = tempfile::TempDir::new().unwrap();
    let tarball = database_driver_tarball_bytes();
    let sha256 = sha256_hex(&tarball);
    let manifest = format!(
        r#"{{
            "extensions": [{{
                "id": "custom",
                "kind": "database_driver",
                "name": "Custom",
                "version": "1.2.3",
                "release_tag": "custom-v1.2.3",
                "artifacts": {{
                    "universal": {{
                        "file": "custom-driver-universal.tar.gz",
                        "sha256": "{sha256}"
                    }}
                }}
            }}]
        }}"#
    );
    let client = Arc::new(FakeHttpClient::new(vec![
        FakeHttpClient::response(200, &manifest),
        binary_response(200, tarball),
    ]));
    let mut registry = ExtensionRegistry::new(tmp.path().join("extensions"));
    registry.register_provider(Arc::new(DatabaseDriverExtensionProvider));

    let summary = smol::block_on(install_database_driver_from_marketplace_with_registry(
        client,
        "https://example.test/manifest.json",
        "custom",
        &registry,
    ))
    .unwrap();

    assert_eq!(ExtensionKind::DatabaseDriver, summary.kind);
    assert_eq!("custom", summary.name);
    assert!(summary.path.join("driver.json").exists());
}

fn config(database_type: DatabaseType) -> DbConnectionConfig {
    DbConnectionConfig {
        id: String::new(),
        database_type,
        name: String::new(),
        host: String::new(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: HashMap::new(),
    }
}

fn external_config(driver_id: &str) -> DbConnectionConfig {
    config(DatabaseType::external(driver_id))
}

fn entry(id: &str, kind: ExtensionKind) -> MarketplaceEntry {
    MarketplaceEntry::from_resolved_urls(
        id,
        kind,
        id,
        "1.0.0",
        "",
        Vec::new(),
        vec![format!("https://example.test/{id}.tar.gz")],
        Some("hash".to_string()),
    )
}

fn database_driver_tarball_bytes() -> Vec<u8> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut archive = tar::Builder::new(encoder);
    append_bytes(&mut archive, "driver-bin", b"driver");
    append_bytes(
        &mut archive,
        "driver.json",
        br#"{
            "id": "custom",
            "name": "Custom",
            "description": "Custom database driver",
            "version": "1.2.3",
            "entry": { "command": "./driver-bin" },
            "transport": { "name": "custom.sock" }
        }"#,
    );
    let encoder = archive.into_inner().unwrap();
    encoder.finish().unwrap()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

fn binary_response(status: u16, body: Vec<u8>) -> anyhow::Result<http::Response<AsyncBody>> {
    http::Response::builder()
        .status(status)
        .body(AsyncBody::from(body))
        .map_err(|error| anyhow::anyhow!("构建响应失败: {}", error))
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

struct FakeHttpClient {
    responses: Mutex<VecDeque<anyhow::Result<http_client::Response<AsyncBody>>>>,
}

impl FakeHttpClient {
    fn new(responses: Vec<anyhow::Result<http_client::Response<AsyncBody>>>) -> Self {
        Self {
            responses: Mutex::new(VecDeque::from(responses)),
        }
    }

    fn response(status: u16, body: &str) -> anyhow::Result<http_client::Response<AsyncBody>> {
        http::Response::builder()
            .status(status)
            .body(AsyncBody::from(body.as_bytes().to_vec()))
            .map_err(|error| anyhow::anyhow!("构建响应失败: {}", error))
    }
}

impl HttpClient for FakeHttpClient {
    fn proxy(&self) -> Option<&Url> {
        None
    }

    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }

    fn send(
        &self,
        _req: http::Request<AsyncBody>,
    ) -> futures::future::BoxFuture<'static, anyhow::Result<http_client::Response<AsyncBody>>> {
        let result = self
            .responses
            .lock()
            .expect("responses 锁失败")
            .pop_front()
            .unwrap_or_else(|| Err(anyhow::anyhow!("缺少 fake response")));

        async move { result }.boxed()
    }
}
