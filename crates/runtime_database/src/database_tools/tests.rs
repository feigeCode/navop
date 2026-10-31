use super::{DatabaseTool, DatabaseToolHandler};
use one_core::storage::connection::SqliteConnection;
use one_core::storage::migration::run_migrations;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, CredentialEntry, CredentialReference, DatabaseType, DbConnectionConfig,
    StoredConnection,
};
use std::sync::Arc;

#[test]
fn database_config_resolves_vault_username_before_connecting() {
    let repo = repository();
    let credential_id = insert_username_credential(&repo, "database-user");
    let mut config = database_config();
    config.credential_reference = Some(username_reference(credential_id));
    let mut connection = StoredConnection::new_database("vault database".to_string(), config, None);
    repo.insert(&mut connection)
        .expect("database connection should insert");

    let handler = DatabaseToolHandler::new(repo, DatabaseTool::Query);
    let resolved = handler
        .database_config("vault database")
        .expect("vault reference should resolve");

    assert_eq!("database-user", resolved.username);
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
    let mut credential = CredentialEntry::new("shared database account");
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

fn database_config() -> DbConnectionConfig {
    DbConnectionConfig {
        id: String::new(),
        database_type: DatabaseType::MySQL,
        name: "vault database".to_string(),
        host: "127.0.0.1".to_string(),
        port: 3306,
        username: "manual-user".to_string(),
        password: String::new(),
        credential_reference: None,
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: Default::default(),
    }
}
