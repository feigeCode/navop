use super::{RedisTool, RedisToolHandler};
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, CredentialEntry, CredentialReference, RedisMode, RedisParams,
    StoredConnection,
};
use std::sync::Arc;

#[test]
fn redis_params_resolve_vault_username_before_connecting() {
    let repo = repository();
    let credential_id = insert_username_credential(&repo, "redis-user");
    let mut params = redis_params();
    params.credential_reference = Some(username_reference(credential_id));
    let mut connection = StoredConnection::new_redis("vault redis".to_string(), params, None);
    repo.insert(&mut connection)
        .expect("redis connection should insert");

    let handler = RedisToolHandler::new(repo, RedisTool::Get);
    let resolved = handler
        .redis_params("vault redis")
        .expect("vault reference should resolve");

    assert_eq!(Some("redis-user".to_string()), resolved.username);
}

fn repository() -> Arc<ConnectionRepository> {
    let connection =
        SqliteConnection::open_with_pool_size(":memory:", 1).expect("sqlite should open");
    connection
        .with_connection(run_migrations)
        .expect("migrations should run");
    Arc::new(ConnectionRepository::new(connection))
}

fn insert_username_credential(repo: &ConnectionRepository, username: &str) -> i64 {
    let mut credential = CredentialEntry::new("shared redis account");
    credential.username = Some(username.to_string());
    repo.credential_repository()
        .insert(&mut credential)
        .expect("credential should insert")
}

fn username_reference(credential_id: i64) -> CredentialReference {
    CredentialReference {
        credential_id,
        credential_cloud_id: None,
        username: true,
        password: false,
        private_key: false,
        passphrase: false,
    }
}

fn redis_params() -> RedisParams {
    RedisParams {
        host: "127.0.0.1".to_string(),
        port: 6379,
        password: None,
        username: Some("manual-user".to_string()),
        credential_reference: None,
        db_index: 0,
        mode: RedisMode::Standalone,
        use_tls: false,
        connect_timeout: None,
        sentinel: None,
        cluster: None,
        ssh_tunnel: None,
    }
}
