use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, TransactionBehavior};

use crate::storage::{
    ConnectionType, CredentialReference, DbConnectionConfig, MongoDBParams, RedisParams,
    RemoteDesktopParams, SshParams, TelnetParams,
};

use super::tunnel_reference_scanner::append_tunnel_hits;
use super::{
    CredentialReferenceHit, CredentialReferenceLocation, CredentialRepository,
    DeleteCredentialOutcome,
};

#[derive(Debug)]
pub(super) struct ScannedConnection {
    pub(super) id: i64,
    pub(super) name: String,
    pub(super) connection_type: ConnectionType,
    pub(super) params: String,
}

#[derive(Debug)]
pub(super) struct CredentialIdentity {
    pub(super) local_id: i64,
    cloud_id: Option<String>,
}

impl CredentialRepository {
    pub fn referencing_connections(
        &self,
        credential_id: i64,
    ) -> Result<Vec<CredentialReferenceHit>> {
        self.conn.with_connection(|connection| {
            let identity = load_credential_identity(connection, credential_id)?;
            scan_references(connection, &identity)
        })
    }

    pub fn delete_checked(&self, credential_id: i64) -> Result<DeleteCredentialOutcome> {
        self.conn.with_connection(|connection| {
            let transaction =
                rusqlite::Transaction::new_unchecked(connection, TransactionBehavior::Immediate)?;
            let identity = load_credential_identity(&transaction, credential_id)?;
            if identity.cloud_id.is_none()
                && !transaction
                    .query_row(
                        "SELECT 1 FROM credential_entries WHERE id = ?1",
                        [credential_id],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some()
            {
                transaction.commit()?;
                return Ok(DeleteCredentialOutcome::NotFound);
            }
            let references = scan_references(&transaction, &identity)?;
            if !references.is_empty() {
                transaction.commit()?;
                return Ok(DeleteCredentialOutcome::Referenced(references));
            }
            transaction.execute(
                "DELETE FROM credential_entries WHERE id = ?1",
                [credential_id],
            )?;
            transaction.commit()?;
            Ok(DeleteCredentialOutcome::Deleted)
        })
    }
}

fn load_credential_identity(
    connection: &rusqlite::Connection,
    credential_id: i64,
) -> Result<CredentialIdentity> {
    let cloud_id = connection
        .query_row(
            "SELECT cloud_id FROM credential_entries WHERE id = ?1",
            [credential_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten()
        .filter(|cloud_id| !cloud_id.is_empty());
    Ok(CredentialIdentity {
        local_id: credential_id,
        cloud_id,
    })
}

fn scan_references(
    connection: &rusqlite::Connection,
    identity: &CredentialIdentity,
) -> Result<Vec<CredentialReferenceHit>> {
    let connections = load_connections(connection)?;
    let by_id = connections
        .iter()
        .map(|connection| (connection.id, connection))
        .collect::<HashMap<_, _>>();
    let mut hits = Vec::new();
    for connection in &connections {
        append_direct_hits(&mut hits, connection, identity)?;
        append_tunnel_hits(&mut hits, connection, identity, &by_id)?;
    }
    Ok(hits)
}

fn load_connections(connection: &rusqlite::Connection) -> Result<Vec<ScannedConnection>> {
    let mut statement =
        connection.prepare("SELECT id, name, connection_type, params FROM connections")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>("id")?,
            row.get::<_, String>("name")?,
            row.get::<_, String>("connection_type")?,
            row.get::<_, String>("params")?,
        ))
    })?;
    rows.map(|row| {
        let (id, name, connection_type, params) = row?;
        Ok(ScannedConnection {
            id,
            name,
            connection_type: parse_connection_type(&connection_type)?,
            params,
        })
    })
    .collect()
}

fn parse_connection_type(value: &str) -> Result<ConnectionType> {
    match value {
        "Database" => Ok(ConnectionType::Database),
        "SshSftp" => Ok(ConnectionType::SshSftp),
        "Redis" => Ok(ConnectionType::Redis),
        "MongoDB" => Ok(ConnectionType::MongoDB),
        "Mqtt" => Ok(ConnectionType::Mqtt),
        "Serial" => Ok(ConnectionType::Serial),
        "Telnet" => Ok(ConnectionType::Telnet),
        "PortForwarding" => Ok(ConnectionType::PortForwarding),
        "Rdp" => Ok(ConnectionType::Rdp),
        "Vnc" => Ok(ConnectionType::Vnc),
        _ => bail!("Unknown connection type {value} while scanning credentials"),
    }
}

fn append_direct_hits(
    hits: &mut Vec<CredentialReferenceHit>,
    connection: &ScannedConnection,
    identity: &CredentialIdentity,
) -> Result<()> {
    for location in direct_locations(connection, identity)? {
        hits.push(reference_hit(connection, identity.local_id, location, None));
    }
    Ok(())
}

pub(super) fn reference_hit(
    connection: &ScannedConnection,
    credential_id: i64,
    location: CredentialReferenceLocation,
    via_ssh_connection_id: Option<i64>,
) -> CredentialReferenceHit {
    CredentialReferenceHit {
        credential_id,
        connection_id: connection.id,
        connection_name: connection.name.clone(),
        connection_type: connection.connection_type,
        location,
        via_ssh_connection_id,
    }
}

fn direct_locations(
    connection: &ScannedConnection,
    identity: &CredentialIdentity,
) -> Result<Vec<CredentialReferenceLocation>> {
    let locations = match connection.connection_type {
        ConnectionType::SshSftp => ssh_locations(connection, identity)?,
        ConnectionType::Database => database_locations(connection, identity)?,
        ConnectionType::Redis => redis_locations(connection, identity)?,
        ConnectionType::MongoDB => mongodb_locations(connection, identity)?,
        ConnectionType::Telnet => telnet_locations(connection, identity)?,
        ConnectionType::Rdp | ConnectionType::Vnc => {
            remote_desktop_locations(connection, identity)?
        }
        _ => Vec::new(),
    };
    Ok(locations)
}

pub(super) fn ssh_locations(
    connection: &ScannedConnection,
    identity: &CredentialIdentity,
) -> Result<Vec<CredentialReferenceLocation>> {
    let params: SshParams = parse_params(connection)?;
    let references = [
        (
            CredentialReferenceLocation::Primary,
            params.credential_reference.as_ref(),
        ),
        (
            CredentialReferenceLocation::JumpServer,
            params
                .jump_server
                .as_ref()
                .and_then(|jump| jump.credential_reference.as_ref()),
        ),
        (
            // 停用保留的跳板机仍持有凭据引用，删除凭据前必须一并提示
            CredentialReferenceLocation::JumpServer,
            params
                .disabled_jump_server
                .as_ref()
                .and_then(|jump| jump.credential_reference.as_ref()),
        ),
        (
            CredentialReferenceLocation::Proxy,
            params
                .proxy
                .as_ref()
                .and_then(|proxy| proxy.credential_reference.as_ref()),
        ),
    ];
    Ok(matching_locations(references, identity))
}

fn telnet_locations(
    connection: &ScannedConnection,
    identity: &CredentialIdentity,
) -> Result<Vec<CredentialReferenceLocation>> {
    let params: TelnetParams = parse_params(connection)?;
    Ok(matching_locations(
        [(
            CredentialReferenceLocation::Primary,
            params.credential_reference.as_ref(),
        )],
        identity,
    ))
}

fn database_locations(
    connection: &ScannedConnection,
    identity: &CredentialIdentity,
) -> Result<Vec<CredentialReferenceLocation>> {
    let params: DbConnectionConfig = parse_params(connection)?;
    Ok(primary_and_proxy_locations(
        params.credential_reference.as_ref(),
        params
            .proxy
            .as_ref()
            .and_then(|proxy| proxy.credential_reference.as_ref()),
        identity,
    ))
}

fn redis_locations(
    connection: &ScannedConnection,
    identity: &CredentialIdentity,
) -> Result<Vec<CredentialReferenceLocation>> {
    let params: RedisParams = parse_params(connection)?;
    let references = [
        (
            CredentialReferenceLocation::Primary,
            params.credential_reference.as_ref(),
        ),
        (
            CredentialReferenceLocation::Sentinel,
            params
                .sentinel
                .as_ref()
                .and_then(|sentinel| sentinel.credential_reference.as_ref()),
        ),
    ];
    Ok(matching_locations(references, identity))
}

fn mongodb_locations(
    connection: &ScannedConnection,
    identity: &CredentialIdentity,
) -> Result<Vec<CredentialReferenceLocation>> {
    let params: MongoDBParams = parse_params(connection)?;
    Ok(matching_locations(
        [(
            CredentialReferenceLocation::Primary,
            params.credential_reference.as_ref(),
        )],
        identity,
    ))
}

fn remote_desktop_locations(
    connection: &ScannedConnection,
    identity: &CredentialIdentity,
) -> Result<Vec<CredentialReferenceLocation>> {
    let params: RemoteDesktopParams = parse_params(connection)?;
    Ok(primary_and_proxy_locations(
        params.credential_reference.as_ref(),
        params
            .proxy
            .as_ref()
            .and_then(|proxy| proxy.credential_reference.as_ref()),
        identity,
    ))
}

fn primary_and_proxy_locations(
    primary: Option<&CredentialReference>,
    proxy: Option<&CredentialReference>,
    identity: &CredentialIdentity,
) -> Vec<CredentialReferenceLocation> {
    matching_locations(
        [
            (CredentialReferenceLocation::Primary, primary),
            (CredentialReferenceLocation::Proxy, proxy),
        ],
        identity,
    )
}

fn matching_locations<const N: usize>(
    references: [(CredentialReferenceLocation, Option<&CredentialReference>); N],
    identity: &CredentialIdentity,
) -> Vec<CredentialReferenceLocation> {
    references
        .into_iter()
        .filter_map(|(location, reference)| {
            reference_matches(reference?, identity).then_some(location)
        })
        .collect()
}

fn reference_matches(reference: &CredentialReference, identity: &CredentialIdentity) -> bool {
    match reference
        .credential_cloud_id
        .as_deref()
        .filter(|cloud_id| !cloud_id.is_empty())
    {
        Some(cloud_id) => identity.cloud_id.as_deref() == Some(cloud_id),
        None => reference.credential_id == identity.local_id,
    }
}

pub(super) fn parse_params<T>(connection: &ScannedConnection) -> Result<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(&connection.params).with_context(|| {
        format!(
            "Failed to parse connection {} ({}) while scanning credentials",
            connection.name, connection.id
        )
    })
}
