use serde_json::{Value, json};

use crate::crypto;
use crate::storage::connection::SqliteConnection;
use crate::storage::master_key_rotation::re_encrypt_secrets;
use crate::storage::migration::run_migrations;

const OLD_KEY: &str = "old-master-key";
const NEW_KEY: &str = "new-master-key";
const CREATED_AT: i64 = 100;
const UPDATED_AT: i64 = 200;
const CREDENTIAL_REVISION: i64 = 7;

#[derive(Debug, PartialEq)]
struct RawCredential {
    password: Option<String>,
    private_key_path: Option<String>,
    private_key_content: Option<String>,
    passphrase: Option<String>,
    ssh_expect: Option<String>,
    updated_at: i64,
}

#[test]
fn rotation_re_encrypts_connection_and_credential_secrets_without_metadata_changes() {
    let (_temp, connection) = test_connection();
    let original_params = connection_params(OLD_KEY);
    insert_connection(&connection, &original_params);
    let original_credential = credential_values(OLD_KEY);
    insert_credential(&connection, "valid", &original_credential);

    let stats = re_encrypt_secrets(&connection, OLD_KEY, NEW_KEY).expect("rotate secrets");

    assert_eq!(stats.connections, 1);
    assert_eq!(stats.credentials, 1);

    let rotated_params = raw_connection_params(&connection);
    let rotated_credential = raw_credential(&connection, "valid");
    assert_ne!(rotated_params, original_params);
    assert_ne!(rotated_credential.password, original_credential.password);
    assert_ne!(
        rotated_credential.private_key_content,
        original_credential.private_key_content
    );
    assert_ne!(
        rotated_credential.passphrase,
        original_credential.passphrase
    );
    assert_ne!(
        rotated_credential.ssh_expect,
        original_credential.ssh_expect
    );

    assert_connection_secrets(&rotated_params, NEW_KEY);
    assert_credential_secrets(&rotated_credential, NEW_KEY);
    assert_old_key_rejected(&rotated_params, &rotated_credential);
    assert_connection_metadata(&connection);
    assert_eq!(
        rotated_credential.private_key_path.as_deref(),
        Some("/tmp/id_ed25519")
    );
    assert_eq!(rotated_credential.updated_at, UPDATED_AT);
}

#[test]
fn rotation_rolls_back_every_change_when_a_later_secret_is_invalid() {
    let (_temp, connection) = test_connection();
    let original_params = connection_params(OLD_KEY);
    insert_connection(&connection, &original_params);
    let first = credential_values(OLD_KEY);
    insert_credential(&connection, "first", &first);
    let mut invalid = credential_values(OLD_KEY);
    invalid.password = Some("ENC:not-valid-base64".to_string());
    insert_credential(&connection, "second", &invalid);

    let error = re_encrypt_secrets(&connection, OLD_KEY, NEW_KEY)
        .expect_err("invalid later row must abort rotation");

    assert!(!error.to_string().is_empty());
    assert_eq!(raw_connection_params(&connection), original_params);
    assert_eq!(raw_credential(&connection, "first"), first);
    assert_eq!(raw_credential(&connection, "second"), invalid);
    assert_connection_secrets(&original_params, OLD_KEY);
    assert_credential_secrets(&first, OLD_KEY);
}

fn test_connection() -> (tempfile::TempDir, SqliteConnection) {
    let temp = tempfile::tempdir().expect("create temp directory");
    let connection = SqliteConnection::open_with_pool_size(temp.path().join("rotation.db"), 1)
        .expect("open database");
    connection
        .with_connection(run_migrations)
        .expect("run migrations");
    (temp, connection)
}

fn connection_params(key: &str) -> String {
    json!({
        "host": "example.com",
        "password": crypto::encrypt_with_key("connection-password", key),
        "proxy": {
            "proxy_password": crypto::encrypt_with_key("proxy-password", key)
        },
        "login_script": [{
            "expect": "Password:",
            "send": crypto::encrypt_with_key("telnet-send", key)
        }]
    })
    .to_string()
}

fn credential_values(key: &str) -> RawCredential {
    let expect = json!({
        "username": {"expect": "login:", "send": "deploy"},
        "password": {"expect": "Password:", "send": "expect-secret"}
    })
    .to_string();
    RawCredential {
        password: Some(crypto::encrypt_with_key("credential-password", key)),
        private_key_path: Some("/tmp/id_ed25519".to_string()),
        private_key_content: Some(crypto::encrypt_with_key("private-key", key)),
        passphrase: Some(crypto::encrypt_with_key("key-passphrase", key)),
        ssh_expect: Some(crypto::encrypt_with_key(&expect, key)),
        updated_at: UPDATED_AT,
    }
}

fn insert_connection(connection: &SqliteConnection, params: &str) {
    connection
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO connections
                 (name, connection_type, params, created_at, updated_at, credential_revision)
                 VALUES ('rotation', 'Telnet', ?1, ?2, ?3, ?4)",
                rusqlite::params![params, CREATED_AT, UPDATED_AT, CREDENTIAL_REVISION],
            )?;
            Ok(())
        })
        .expect("insert connection");
}

fn insert_credential(connection: &SqliteConnection, name: &str, value: &RawCredential) {
    connection
        .with_connection(|conn| {
            conn.execute(
                "INSERT INTO credential_entries
                 (name, password, private_key_path, private_key_content, passphrase, ssh_expect,
                  created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![
                    name,
                    value.password,
                    value.private_key_path,
                    value.private_key_content,
                    value.passphrase,
                    value.ssh_expect,
                    CREATED_AT,
                    value.updated_at,
                ],
            )?;
            Ok(())
        })
        .expect("insert credential");
}

fn raw_connection_params(connection: &SqliteConnection) -> String {
    connection
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT params FROM connections WHERE name = 'rotation'",
                [],
                |row| row.get(0),
            )?)
        })
        .expect("read connection params")
}

fn raw_credential(connection: &SqliteConnection, name: &str) -> RawCredential {
    connection
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT password, private_key_path, private_key_content, passphrase,
                        ssh_expect, updated_at
                 FROM credential_entries WHERE name = ?1",
                [name],
                |row| {
                    Ok(RawCredential {
                        password: row.get(0)?,
                        private_key_path: row.get(1)?,
                        private_key_content: row.get(2)?,
                        passphrase: row.get(3)?,
                        ssh_expect: row.get(4)?,
                        updated_at: row.get(5)?,
                    })
                },
            )?)
        })
        .expect("read credential")
}

fn assert_connection_secrets(params: &str, key: &str) {
    let value: Value = serde_json::from_str(params).expect("valid params");
    assert_decrypts_to(&value["password"], key, "connection-password");
    assert_decrypts_to(&value["proxy"]["proxy_password"], key, "proxy-password");
    assert_decrypts_to(&value["login_script"][0]["send"], key, "telnet-send");
}

fn assert_credential_secrets(value: &RawCredential, key: &str) {
    assert_secret(value.password.as_deref(), key, "credential-password");
    assert_secret(value.private_key_content.as_deref(), key, "private-key");
    assert_secret(value.passphrase.as_deref(), key, "key-passphrase");
    let expect = crypto::decrypt_with_key(value.ssh_expect.as_deref().expect("ssh expect"), key)
        .expect("decrypt ssh expect");
    assert!(expect.contains("expect-secret"));
}

fn assert_old_key_rejected(params: &str, credential: &RawCredential) {
    let value: Value = serde_json::from_str(params).expect("valid params");
    assert!(crypto::decrypt_with_key(value["password"].as_str().unwrap(), OLD_KEY).is_err());
    assert!(crypto::decrypt_with_key(credential.password.as_deref().unwrap(), OLD_KEY).is_err());
}

fn assert_connection_metadata(connection: &SqliteConnection) {
    let metadata = connection
        .with_connection(|conn| {
            Ok(conn.query_row(
                "SELECT created_at, updated_at, credential_revision
                 FROM connections WHERE name = 'rotation'",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?)
        })
        .expect("read metadata");
    assert_eq!(metadata, (CREATED_AT, UPDATED_AT, CREDENTIAL_REVISION));
}

fn assert_decrypts_to(value: &Value, key: &str, expected: &str) {
    assert_secret(value.as_str(), key, expected);
}

fn assert_secret(value: Option<&str>, key: &str, expected: &str) {
    assert_eq!(
        crypto::decrypt_with_key(value.expect("encrypted value"), key).expect("decrypt value"),
        expected
    );
}
