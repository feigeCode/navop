use anyhow::Result;
use rusqlite::{Transaction, TransactionBehavior, params};

use crate::crypto;

use super::connection::SqliteConnection;
use super::models::re_encrypt_sensitive_json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasterKeyRotationStats {
    pub connections: usize,
    pub credentials: usize,
}

struct CredentialSecrets {
    id: i64,
    password: Option<String>,
    private_key_content: Option<String>,
    passphrase: Option<String>,
    ssh_expect: Option<String>,
}

pub fn re_encrypt_secrets(
    connection: &SqliteConnection,
    old_key: &str,
    new_key: &str,
) -> Result<MasterKeyRotationStats> {
    connection.with_connection_mut(|connection| {
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let connections = load_connections(&transaction)?;
        let credentials = load_credentials(&transaction)?;
        let credential_count = credentials.len();
        rotate_connections(&transaction, &connections, old_key, new_key)?;
        rotate_credentials(&transaction, credentials, old_key, new_key)?;
        let stats = MasterKeyRotationStats {
            connections: connections.len(),
            credentials: credential_count,
        };
        transaction.commit()?;
        Ok(stats)
    })
}

fn load_connections(transaction: &Transaction<'_>) -> Result<Vec<(i64, String)>> {
    let mut statement = transaction.prepare("SELECT id, params FROM connections ORDER BY id")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_credentials(transaction: &Transaction<'_>) -> Result<Vec<CredentialSecrets>> {
    let mut statement = transaction.prepare(
        "SELECT id, password, private_key_content, passphrase, ssh_expect
         FROM credential_entries ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(CredentialSecrets {
            id: row.get(0)?,
            password: row.get(1)?,
            private_key_content: row.get(2)?,
            passphrase: row.get(3)?,
            ssh_expect: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn rotate_connections(
    transaction: &Transaction<'_>,
    connections: &[(i64, String)],
    old_key: &str,
    new_key: &str,
) -> Result<()> {
    let mut statement = transaction.prepare("UPDATE connections SET params = ?1 WHERE id = ?2")?;
    for (id, params) in connections {
        let rotated = re_encrypt_sensitive_json(params, old_key, new_key)?;
        statement.execute(params![rotated, id])?;
    }
    Ok(())
}

fn rotate_credentials(
    transaction: &Transaction<'_>,
    credentials: Vec<CredentialSecrets>,
    old_key: &str,
    new_key: &str,
) -> Result<()> {
    let mut statement = transaction.prepare(
        "UPDATE credential_entries
         SET password = ?1, private_key_content = ?2, passphrase = ?3, ssh_expect = ?4
         WHERE id = ?5",
    )?;
    for credential in credentials {
        let password = rotate_optional(credential.password, old_key, new_key)?;
        let private_key = rotate_optional(credential.private_key_content, old_key, new_key)?;
        let passphrase = rotate_optional(credential.passphrase, old_key, new_key)?;
        let ssh_expect = rotate_optional(credential.ssh_expect, old_key, new_key)?;
        statement.execute(params![
            password,
            private_key,
            passphrase,
            ssh_expect,
            credential.id
        ])?;
    }
    Ok(())
}

fn rotate_optional(value: Option<String>, old_key: &str, new_key: &str) -> Result<Option<String>> {
    value
        .map(|value| {
            if value.is_empty() {
                Ok(value)
            } else {
                Ok(crypto::re_encrypt_data(&value, old_key, new_key)?)
            }
        })
        .transpose()
}
