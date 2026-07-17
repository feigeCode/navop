use crate::approval::PublicMcpApprovalRequest;
use crate::permissions::PublicMcpOperationKind;
use serde_json::Value;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PublicMcpApprovalGrantId(String);

#[derive(Clone)]
pub struct PublicMcpApprovalGrantStore {
    state: Arc<Mutex<PublicMcpApprovalGrantState>>,
}

impl PublicMcpApprovalGrantStore {
    pub fn new(ttl: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(PublicMcpApprovalGrantState::new(ttl))),
        }
    }

    pub fn register(&self, arguments: Value) -> Option<PublicMcpApprovalGrantId> {
        self.state
            .lock()
            .expect("public MCP approval grant lock poisoned")
            .register(arguments, Instant::now())
    }

    pub fn consume(&self, request: &PublicMcpApprovalRequest) -> bool {
        self.state
            .lock()
            .expect("public MCP approval grant lock poisoned")
            .consume(request, Instant::now())
    }

    pub fn revoke(&self, grant_id: &PublicMcpApprovalGrantId) -> bool {
        self.state
            .lock()
            .expect("public MCP approval grant lock poisoned")
            .revoke(grant_id)
    }
}

pub fn redact_approval_arguments(value: Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .into_iter()
                .map(|(key, value)| {
                    if is_secret_key(&key) {
                        (key, Value::String("<redacted>".to_string()))
                    } else {
                        (key, redact_approval_arguments(value))
                    }
                })
                .collect(),
        ),
        Value::Array(items) => {
            Value::Array(items.into_iter().map(redact_approval_arguments).collect())
        }
        value => value,
    }
}

fn is_secret_key(key: &str) -> bool {
    let normalized = key.to_ascii_lowercase();
    normalized.contains("password")
        || normalized.contains("passphrase")
        || normalized.contains("secret")
        || normalized.contains("token")
        || normalized.contains("private_key")
}

struct PublicMcpApprovalGrant {
    id: PublicMcpApprovalGrantId,
    arguments: Value,
    expires_at: Instant,
}

struct PublicMcpApprovalGrantState {
    ttl: Duration,
    grants: Vec<PublicMcpApprovalGrant>,
}

impl PublicMcpApprovalGrantState {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            grants: Vec::new(),
        }
    }

    fn register(&mut self, arguments: Value, now: Instant) -> Option<PublicMcpApprovalGrantId> {
        if !arguments.is_object() {
            return None;
        }
        self.prune_expired(now);
        let id = PublicMcpApprovalGrantId(uuid::Uuid::new_v4().to_string());
        self.grants.push(PublicMcpApprovalGrant {
            id: id.clone(),
            arguments: redact_approval_arguments(arguments),
            expires_at: now + self.ttl,
        });
        Some(id)
    }

    fn consume(&mut self, request: &PublicMcpApprovalRequest, now: Instant) -> bool {
        self.prune_expired(now);
        if request.operation != PublicMcpOperationKind::CallToolRuntimeTool {
            return false;
        }
        let Some(arguments) = request
            .details
            .get("requestArguments")
            .or_else(|| request.details.get("arguments"))
        else {
            return false;
        };
        let Some(index) = self
            .grants
            .iter()
            .position(|grant| grant.arguments == *arguments)
        else {
            return false;
        };
        self.grants.remove(index);
        true
    }

    fn revoke(&mut self, grant_id: &PublicMcpApprovalGrantId) -> bool {
        let Some(index) = self.grants.iter().position(|grant| &grant.id == grant_id) else {
            return false;
        };
        self.grants.remove(index);
        true
    }

    fn prune_expired(&mut self, now: Instant) {
        self.grants.retain(|grant| grant.expires_at > now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::approval::PublicMcpApprovalRequest;
    use crate::permissions::PublicMcpOperationKind;
    use serde_json::json;
    use std::time::{Duration, Instant};

    fn public_request(arguments: serde_json::Value) -> PublicMcpApprovalRequest {
        PublicMcpApprovalRequest {
            operation: PublicMcpOperationKind::CallToolRuntimeTool,
            tool_name: "terminal.exec".into(),
            summary: "Call Execute command".into(),
            details: json!({
                "tool": "terminal.exec",
                "arguments": arguments,
            }),
        }
    }

    #[test]
    fn grant_consumes_one_matching_public_mcp_approval() {
        let now = Instant::now();
        let mut grants = PublicMcpApprovalGrantState::new(Duration::from_secs(10));
        let grant_id = grants
            .register(json!({"command": "ls"}), now)
            .expect("valid arguments should create a grant");

        assert!(grants.consume(&public_request(json!({"command": "ls"})), now));
        assert!(!grants.consume(&public_request(json!({"command": "ls"})), now));
        assert!(!grants.revoke(&grant_id));
    }

    #[test]
    fn grant_does_not_bypass_mismatched_or_expired_request() {
        let now = Instant::now();
        let mut grants = PublicMcpApprovalGrantState::new(Duration::from_secs(10));
        grants
            .register(json!({"command": "ls"}), now)
            .expect("valid arguments should create a grant");

        assert!(!grants.consume(&public_request(json!({"command": "pwd"})), now));
        assert!(!grants.consume(
            &public_request(json!({"command": "ls"})),
            now + Duration::from_secs(11),
        ));
    }

    #[test]
    fn grant_matches_original_redacted_arguments_before_normalized_arguments() {
        let now = Instant::now();
        let mut grants = PublicMcpApprovalGrantState::new(Duration::from_secs(10));
        grants
            .register(json!({"command": "ls", "password": "secret"}), now)
            .expect("valid arguments should create a grant");
        let mut request = public_request(json!({
            "command": "ls",
            "password": "<redacted>",
            "target": {"resource_id": "ssh-1"},
        }));
        request.details["requestArguments"] = json!({
            "command": "ls",
            "password": "<redacted>",
        });

        assert!(grants.consume(&request, now));
    }

    #[test]
    fn grant_rejects_non_object_arguments_and_non_runtime_approvals() {
        let now = Instant::now();
        let mut grants = PublicMcpApprovalGrantState::new(Duration::from_secs(10));
        assert!(grants.register(json!("ls"), now).is_none());
        grants
            .register(json!({"command": "ls"}), now)
            .expect("valid arguments should create a grant");

        let mut request = public_request(json!({"command": "ls"}));
        request.operation = PublicMcpOperationKind::ExecuteRemoteCommand;
        assert!(!grants.consume(&request, now));
    }
}
