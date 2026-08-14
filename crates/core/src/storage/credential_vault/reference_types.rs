use serde::{Deserialize, Serialize};

use crate::storage::ConnectionType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialReferenceLocation {
    Primary,
    JumpServer,
    Proxy,
    Sentinel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CredentialReferenceHit {
    pub credential_id: i64,
    pub connection_id: i64,
    pub connection_name: String,
    pub connection_type: ConnectionType,
    pub location: CredentialReferenceLocation,
    pub via_ssh_connection_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeleteCredentialOutcome {
    Deleted,
    NotFound,
    Referenced(Vec<CredentialReferenceHit>),
}
