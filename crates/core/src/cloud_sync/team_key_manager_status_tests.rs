use std::sync::{Arc, RwLock};

use super::team_key_envelope::{TeamKeyKdfParams, create_team_key_envelope};
use super::team_key_manager::{
    TeamKeyCacheStatus, TeamKeyLoadStatus, TeamKeyManager, team_key_cache_status,
};
use super::{CloudAccountScope, CloudSyncService, Team};
use crate::crypto;
use crate::storage::connection::SqliteConnection;
use crate::storage::migration::run_migrations;
use crate::storage::{TeamKeyCache, TeamKeyCacheRepository};

#[test]
fn cache_status_distinguishes_version_mismatch_and_invalid_envelope() {
    let mut cache = cached_team();
    cache.cached_key_version = Some(3);
    assert_eq!(
        TeamKeyCacheStatus::VersionMismatch,
        team_key_cache_status(&cache)
    );

    cache.cached_key_version = Some(4);
    cache.key_verification = Some(crypto::generate_key_verification("legacy-key"));
    assert_eq!(TeamKeyCacheStatus::Invalid, team_key_cache_status(&cache));
}

#[test]
fn v2_cached_passphrase_unlocks_random_data_key() {
    let repo = test_repo("v2");
    let scope = test_scope();
    let created = create_team_key_envelope(
        "correct horse battery staple",
        TeamKeyKdfParams::for_tests(),
    )
    .expect("create v2 envelope");
    repo.upsert(&TeamKeyCache {
        scope: scope.clone(),
        key_verification: Some(created.verification.clone()),
        encrypted_team_key: Some(crypto::encrypt_with_key(
            "correct horse battery staple",
            "personal-key",
        )),
        ..cached_team()
    })
    .expect("seed v2 cache");
    let service = Arc::new(RwLock::new(CloudSyncService::new()));
    let manager = TeamKeyManager::new(repo, service.clone(), scope);

    let status = manager
        .load_cached_team_key(&team(created.verification), "personal-key")
        .expect("load v2 key");

    assert_eq!(TeamKeyLoadStatus::Unlocked, status);
    assert_eq!(
        Some(created.data_key),
        service
            .read()
            .expect("service lock")
            .get_team_key("team-1")
            .cloned()
    );
}

#[test]
fn non_v2_cached_passphrase_is_rejected() {
    let repo = test_repo("legacy");
    let scope = test_scope();
    let verification = crypto::generate_key_verification("legacy-key");
    repo.upsert(&TeamKeyCache {
        scope: scope.clone(),
        key_verification: Some(verification.clone()),
        encrypted_team_key: Some(crypto::encrypt_with_key("legacy-key", "personal-key")),
        ..cached_team()
    })
    .expect("seed legacy cache");
    let service = Arc::new(RwLock::new(CloudSyncService::new()));
    let manager = TeamKeyManager::new(repo, service.clone(), scope);

    assert!(
        manager
            .load_cached_team_key(&team(verification), "personal-key")
            .is_err()
    );
    assert!(
        !service
            .read()
            .expect("service lock")
            .is_team_unlocked("team-1")
    );
}

fn cached_team() -> TeamKeyCache {
    TeamKeyCache {
        scope: test_scope(),
        team_id: "team-1".to_string(),
        team_name: "Platform".to_string(),
        key_version: 4,
        cached_key_version: Some(4),
        key_verification: None,
        encrypted_team_key: Some("encrypted".to_string()),
        last_verified_at: Some(123),
        updated_at: 200,
        role: Some("member".to_string()),
    }
}

fn team(verification: String) -> Team {
    Team {
        id: "team-1".to_string(),
        name: "Platform".to_string(),
        owner_id: "owner-1".to_string(),
        description: None,
        key_verification: Some(verification),
        key_version: 4,
        created_at: 100,
        updated_at: 200,
    }
}

fn test_scope() -> CloudAccountScope {
    CloudAccountScope::new("https://project.supabase.co", "user-1")
}

fn test_repo(label: &str) -> TeamKeyCacheRepository {
    let path = std::env::temp_dir().join(format!(
        "navop-team-key-manager-status-{label}-{}-{}.db",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time")
            .as_nanos()
    ));
    let connection = SqliteConnection::open_with_pool_size(path, 1).expect("open sqlite");
    connection
        .with_connection(run_migrations)
        .expect("run migrations");
    TeamKeyCacheRepository::new(connection)
}
