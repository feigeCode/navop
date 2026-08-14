mod models_tests;
mod protected_delete_edge_tests;
mod protected_delete_tests;
mod reference_scan_edge_tests;
mod reference_scan_tests;
mod repository_tests;
mod resolver_tests;
mod runtime_resolver_tests;

use std::sync::{Mutex, MutexGuard, OnceLock};

use tempfile::TempDir;

use crate::crypto;
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;

use super::super::CredentialRepository;

pub(super) fn crypto_guard() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(super) fn test_repository() -> (TempDir, SqliteConnection, CredentialRepository) {
    let temp = tempfile::tempdir().expect("create temporary directory");
    let connection = SqliteConnection::open_with_pool_size(temp.path().join("credentials.db"), 1)
        .expect("open credential test database");
    connection
        .with_connection(|connection| run_migrations(connection))
        .expect("run migrations");
    let repository = CredentialRepository::new(connection.clone());
    (temp, connection, repository)
}

pub(super) fn with_master_key<T>(operation: impl FnOnce() -> T) -> T {
    let _guard = crypto_guard();
    crypto::set_master_key_for_session("credential-vault-test-key");
    let result = operation();
    crypto::clear_master_key();
    result
}
