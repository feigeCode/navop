use super::{OverwritePolicy, parse_overwrite_policy, prepare_local_target, sftp_tool_registry};
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, DatabaseType, DbConnectionConfig, StoredConnection};
use serde_json::json;
use std::fs;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tool_runtime::{ResourceCapability, ToolAdapter, ToolContext};

#[test]
fn sftp_registry_exposes_file_transfer_tools() {
    let registry = sftp_tool_registry(repo());
    let tools = registry.list(ToolAdapter::Mcp);
    let ids = tools.iter().map(|tool| tool.id.clone()).collect::<Vec<_>>();

    assert!(ids.contains(&"sftp.list".to_string()));
    assert!(ids.contains(&"sftp.read".to_string()));
    assert!(ids.contains(&"sftp.write".to_string()));
    assert!(ids.contains(&"sftp.stat".to_string()));
    assert!(ids.contains(&"sftp.upload".to_string()));
    assert!(ids.contains(&"sftp.download".to_string()));

    let write = tools
        .iter()
        .find(|tool| tool.id == "sftp.write")
        .expect("write tool should be registered");
    assert_eq!(
        json!(["connection", "content_base64"]),
        write.input_schema["required"]
    );
    assert!(write.description.contains("canonical file operation"));
    assert!(!write.description.contains("ssh.remote_exec"));

    let upload = tools
        .iter()
        .find(|tool| tool.id == "sftp.upload")
        .expect("upload tool should be registered");
    assert_eq!(
        json!(["connection", "local_path", "remote_path"]),
        upload.input_schema["required"]
    );
    assert_eq!(
        json!(["fail", "overwrite", "skip"]),
        upload.input_schema["properties"]["on_exists"]["enum"]
    );
    assert!(upload.description.contains("on_exists"));

    let download = tools
        .iter()
        .find(|tool| tool.id == "sftp.download")
        .expect("download tool should be registered");
    assert_eq!(
        json!(["connection", "remote_path", "local_path"]),
        download.input_schema["required"]
    );
    assert_eq!(
        json!(["fail", "overwrite", "skip"]),
        download.input_schema["properties"]["on_exists"]["enum"]
    );
    assert!(download.description.contains("on_exists"));

    let stat = tools
        .iter()
        .find(|tool| tool.id == "sftp.stat")
        .expect("stat tool should be registered");
    assert_eq!(json!(["connection", "path"]), stat.input_schema["required"]);
    assert!(stat.description.contains("exists"));
}

#[test]
fn sftp_tools_target_ssh_sftp_resources_by_capability() {
    let registry = sftp_tool_registry(repo());

    for (tool_id, capability) in [
        ("sftp.list", ResourceCapability::List),
        ("sftp.read", ResourceCapability::ReadFile),
        ("sftp.write", ResourceCapability::WriteFile),
        ("sftp.stat", ResourceCapability::ReadFile),
        ("sftp.upload", ResourceCapability::WriteFile),
        ("sftp.download", ResourceCapability::ReadFile),
    ] {
        let tool = registry
            .get_runtime(tool_id, ToolAdapter::FunctionCalling)
            .expect("sftp tool should be registered");
        assert!(tool.target.required, "{tool_id} should require target");
        assert_eq!(
            vec![capability],
            tool.target.required_capabilities,
            "{tool_id} should target resources with the expected file capability"
        );
    }
}

#[test]
fn sftp_overwrite_policy_defaults_to_fail() {
    let input = json!({});

    let policy = parse_overwrite_policy(&input).expect("default policy should parse");

    assert_eq!(OverwritePolicy::Fail, policy);
}

#[test]
fn sftp_overwrite_policy_accepts_explicit_values() {
    assert_eq!(
        OverwritePolicy::Fail,
        parse_overwrite_policy(&json!({ "on_exists": "fail" })).unwrap()
    );
    assert_eq!(
        OverwritePolicy::Overwrite,
        parse_overwrite_policy(&json!({ "on_exists": "overwrite" })).unwrap()
    );
    assert_eq!(
        OverwritePolicy::Skip,
        parse_overwrite_policy(&json!({ "on_exists": "skip" })).unwrap()
    );
}

#[test]
fn sftp_overwrite_policy_rejects_unknown_values() {
    let error = parse_overwrite_policy(&json!({ "on_exists": "merge" }))
        .expect_err("unknown overwrite policy should fail");

    assert!(error.to_string().contains("invalid on_exists"));
}

#[test]
fn sftp_prepare_local_target_fails_when_target_exists_by_default() {
    let dir = temp_dir();
    let target = dir.join("download.txt");
    fs::write(&target, "existing").unwrap();

    let error = prepare_local_target(target.to_str().unwrap(), OverwritePolicy::Fail)
        .expect_err("existing local target should require explicit policy");

    assert!(error.to_string().contains("target already exists"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sftp_prepare_local_target_skips_existing_target() {
    let dir = temp_dir();
    let target = dir.join("download.txt");
    fs::write(&target, "existing").unwrap();

    let skipped = prepare_local_target(target.to_str().unwrap(), OverwritePolicy::Skip).unwrap();

    assert!(skipped);
    assert!(target.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sftp_prepare_local_target_removes_existing_directory_for_overwrite() {
    let dir = temp_dir();
    let target = dir.join("download");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("stale.txt"), "stale").unwrap();

    let skipped = prepare_local_target(target.to_str().unwrap(), OverwritePolicy::Overwrite)
        .expect("overwrite should prepare target");

    assert!(!skipped);
    assert!(!target.exists());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn sftp_tools_reject_non_sftp_connections_before_connecting() {
    let repo = repo();
    let registry = sftp_tool_registry(repo.clone());
    let mut connection =
        StoredConnection::new_database("prod mysql".to_string(), mysql_config(), None);
    repo.insert(&mut connection)
        .expect("database connection should insert");

    let error = futures::executor::block_on(registry.call(
        "sftp.list",
        json!({ "connection": "prod mysql", "path": "/" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("non-sftp connection should be rejected");

    assert!(
        error
            .to_string()
            .contains("connection is not ssh_sftp: prod mysql")
    );
}

#[test]
fn sftp_tools_resolve_connections_by_id_before_type_check() {
    let repo = repo();
    let registry = sftp_tool_registry(repo.clone());
    let mut connection =
        StoredConnection::new_database("prod mysql".to_string(), mysql_config(), None);
    repo.insert(&mut connection)
        .expect("database connection should insert");

    let error = futures::executor::block_on(registry.call(
        "sftp.list",
        json!({ "connection": connection.id.unwrap().to_string(), "path": "/" }),
        ToolContext::for_adapter(ToolAdapter::Mcp),
    ))
    .expect_err("non-sftp connection should be rejected");

    assert!(!error.to_string().contains("unknown connection"));
    assert!(error.to_string().contains("connection is not ssh_sftp"));
}

fn repo() -> Arc<ConnectionRepository> {
    let conn = SqliteConnection::open_with_pool_size(":memory:", 1).expect("sqlite should open");
    conn.with_connection(|db| {
        run_migrations(db)?;
        Ok(())
    })
    .expect("migrations should run");
    Arc::new(ConnectionRepository::new(conn))
}

fn mysql_config() -> DbConnectionConfig {
    DbConnectionConfig {
        id: String::new(),
        database_type: DatabaseType::MySQL,
        name: "prod mysql".to_string(),
        host: "127.0.0.1".to_string(),
        port: 3306,
        username: "app".to_string(),
        password: String::new(),
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: Default::default(),
    }
}

fn temp_dir() -> std::path::PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("onetcli-sftp-tools-test-{nanos}"));
    fs::create_dir_all(&path).unwrap();
    path
}
