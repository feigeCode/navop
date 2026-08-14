use std::collections::HashMap;

use anyhow::{Context, Result, anyhow, bail};

use crate::storage::{
    ConnectionType, DbConnectionConfig, MongoDBParams, PortForwardingParams, RedisParams,
};

use super::CredentialReferenceHit;
use super::reference_scanner::{ScannedConnection, parse_params, reference_hit, ssh_locations};

pub(super) fn append_tunnel_hits(
    hits: &mut Vec<CredentialReferenceHit>,
    connection: &ScannedConnection,
    credential_id: i64,
    by_id: &HashMap<i64, &ScannedConnection>,
) -> Result<()> {
    let Some(ssh_id) = tunnel_ssh_id(connection)? else {
        return Ok(());
    };
    let ssh = by_id
        .get(&ssh_id)
        .ok_or_else(|| anyhow!("Referenced SSH connection {ssh_id} was not found"))?;
    if ssh.connection_type != ConnectionType::SshSftp {
        bail!("Referenced connection {ssh_id} is not an SSH connection");
    }
    for location in ssh_locations(ssh, credential_id)? {
        hits.push(reference_hit(
            connection,
            credential_id,
            location,
            Some(ssh_id),
        ));
    }
    Ok(())
}

fn tunnel_ssh_id(connection: &ScannedConnection) -> Result<Option<i64>> {
    match connection.connection_type {
        ConnectionType::Database => database_tunnel_id(connection),
        ConnectionType::Redis => {
            let params: RedisParams = parse_params(connection)?;
            enabled_tunnel_id(params.ssh_tunnel.as_ref())
        }
        ConnectionType::MongoDB => {
            let params: MongoDBParams = parse_params(connection)?;
            enabled_tunnel_id(params.ssh_tunnel.as_ref())
        }
        ConnectionType::PortForwarding => {
            let params: PortForwardingParams = parse_params(connection)?;
            Ok(Some(params.ssh_connection_id))
        }
        _ => Ok(None),
    }
}

fn database_tunnel_id(connection: &ScannedConnection) -> Result<Option<i64>> {
    let params: DbConnectionConfig = parse_params(connection)?;
    if !params.get_param_bool("ssh_tunnel_enabled") {
        return Ok(None);
    }
    let value = params
        .get_param("ssh_connection_id")
        .context("Enabled database SSH tunnel has no connection ID")?;
    Ok(Some(value.parse().with_context(|| {
        format!("Invalid database SSH connection ID {value}")
    })?))
}

fn enabled_tunnel_id(tunnel: Option<&connection_tunnel::SshTunnelConfig>) -> Result<Option<i64>> {
    let Some(tunnel) = tunnel.filter(|tunnel| tunnel.enabled) else {
        return Ok(None);
    };
    Ok(Some(
        tunnel
            .connection_id
            .context("Enabled SSH tunnel has no connection ID")?,
    ))
}
