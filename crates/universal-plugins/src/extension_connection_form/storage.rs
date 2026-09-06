use std::collections::{HashMap, HashSet};

use one_core::{
    connection_notifier::ConnectionDataEvent,
    storage::{
        ConnectionRepository, ExtensionConnectionParams, StoredConnection, traits::Repository,
    },
};

pub(super) struct ExtensionConnectionDraft {
    pub name: String,
    pub config: serde_json::Map<String, serde_json::Value>,
    pub secret_updates: HashMap<String, String>,
    pub visible_secrets: HashSet<String>,
    pub cleared_secrets: HashSet<String>,
    pub workspace_id: Option<i64>,
    pub team_id: Option<String>,
    pub owner_id: Option<String>,
    pub remark: Option<String>,
    pub sync_enabled: bool,
}

pub(super) fn build_connection(
    existing: Option<&StoredConnection>,
    contribution: &extension_runtime::RegisteredResourceConnectionContribution,
    draft: ExtensionConnectionDraft,
) -> anyhow::Result<StoredConnection> {
    let mut secrets = existing
        .and_then(|connection| connection.to_extension_params().ok())
        .map(|params| params.secrets)
        .unwrap_or_default();
    secrets.retain(|field, _| {
        draft.visible_secrets.contains(field) && !draft.cleared_secrets.contains(field)
    });
    secrets.extend(draft.secret_updates);
    let params = ExtensionConnectionParams::new(
        contribution.extension_id.clone(),
        contribution.id.clone(),
        draft.config,
        secrets,
    )?;
    let mut connection = StoredConnection::new_extension(draft.name, params, draft.workspace_id);
    if let Some(existing) = existing {
        copy_metadata(existing, &mut connection);
    }
    connection.team_id = draft.team_id;
    connection.owner_id = draft.owner_id;
    connection.remark = draft.remark;
    connection.sync_enabled = draft.sync_enabled;
    Ok(connection)
}

fn copy_metadata(existing: &StoredConnection, connection: &mut StoredConnection) {
    connection.id = existing.id;
    connection.credential_revision = existing.credential_revision;
    connection.remark = existing.remark.clone();
    connection.sync_enabled = existing.sync_enabled;
    connection.cloud_id = existing.cloud_id.clone();
    connection.last_synced_at = existing.last_synced_at;
    connection.sort_order = existing.sort_order;
    connection.created_at = existing.created_at;
    connection.updated_at = existing.updated_at;
    connection.team_id = existing.team_id.clone();
    connection.owner_id = existing.owner_id.clone();
}

pub(super) fn persist_connection(
    repository: &ConnectionRepository,
    connection: &mut StoredConnection,
) -> anyhow::Result<ConnectionDataEvent> {
    if connection.id.is_some() {
        repository.update(connection)?;
        Ok(ConnectionDataEvent::ConnectionUpdated {
            connection: connection.clone(),
        })
    } else {
        repository.insert(connection)?;
        Ok(ConnectionDataEvent::ConnectionCreated {
            connection: connection.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, HashMap, HashSet};

    use extension_runtime::extension::manifest::ResourceConnectionForm;
    use one_core::storage::ExtensionConnectionParams;

    use super::*;

    #[test]
    fn build_connection_preserves_only_visible_declared_secrets() {
        let params = ExtensionConnectionParams::new(
            "com.example.search",
            "search",
            serde_json::Map::new(),
            BTreeMap::from([
                ("api_key".into(), "old-key".into()),
                ("password".into(), "old-password".into()),
            ]),
        )
        .unwrap();
        let existing = StoredConnection::new_extension("Search".into(), params, None);
        let contribution = contribution();

        let connection = build_connection(
            Some(&existing),
            &contribution,
            ExtensionConnectionDraft {
                name: "Search".into(),
                config: serde_json::Map::new(),
                secret_updates: HashMap::from([("password".into(), "new-password".into())]),
                visible_secrets: HashSet::from(["password".into()]),
                cleared_secrets: HashSet::new(),
                workspace_id: Some(7),
                team_id: Some("team-1".into()),
                owner_id: Some("owner-1".into()),
                remark: Some("Production".into()),
                sync_enabled: false,
            },
        )
        .unwrap();

        let params = connection.to_extension_params().unwrap();
        assert_eq!(
            BTreeMap::from([("password".into(), "new-password".into())]),
            params.secrets
        );
        assert_eq!(Some(7), connection.workspace_id);
        assert_eq!(Some("team-1"), connection.team_id.as_deref());
        assert_eq!(Some("owner-1"), connection.owner_id.as_deref());
        assert_eq!(Some("Production"), connection.remark.as_deref());
        assert!(!connection.sync_enabled);
    }

    #[test]
    fn build_connection_removes_explicitly_cleared_secret() {
        let params = ExtensionConnectionParams::new(
            "com.example.search",
            "search",
            serde_json::Map::new(),
            BTreeMap::from([("api_key".into(), "old-key".into())]),
        )
        .unwrap();
        let existing = StoredConnection::new_extension("Search".into(), params, None);
        let connection = build_connection(
            Some(&existing),
            &contribution(),
            ExtensionConnectionDraft {
                name: "Search".into(),
                config: serde_json::Map::new(),
                secret_updates: HashMap::new(),
                visible_secrets: HashSet::from(["api_key".into()]),
                cleared_secrets: HashSet::from(["api_key".into()]),
                workspace_id: None,
                team_id: None,
                owner_id: None,
                remark: None,
                sync_enabled: true,
            },
        )
        .unwrap();

        assert!(connection.to_extension_params().unwrap().secrets.is_empty());
    }

    fn contribution() -> extension_runtime::RegisteredResourceConnectionContribution {
        extension_runtime::RegisteredResourceConnectionContribution {
            extension_id: "com.example.search".into(),
            extension_root: "/tmp/com.example.search".into(),
            id: "search".into(),
            label: "Search".into(),
            description: None,
            icon_path: None,
            runtime_id: "com.example.search::main".into(),
            resource_type: "search".into(),
            shell_view_id: None,
            form: ResourceConnectionForm::default(),
        }
    }
}
