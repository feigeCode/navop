mod model;

pub use model::*;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn database_record_requires_database_payload() {
        let record = ImportRecord {
            id: "navicat:prod".to_string(),
            importer_id: "navicat".to_string(),
            source_label: "Navicat".to_string(),
            kind: ImportRecordKind::Database,
            display_name: "Prod MySQL".to_string(),
            database: None,
            ssh: Some(SshImportRecord {
                name: "Prod SSH".to_string(),
                host: "prod.example.com".to_string(),
                port: Some(22),
                username: "root".to_string(),
                auth_method: SshImportAuthMethod::Agent,
            }),
            password_status: PasswordImportStatus::Unsupported,
            warnings: Vec::new(),
        };

        assert!(matches!(
            record.validate_shape(),
            Err(ImportProtocolError::MismatchedRecordPayload { .. })
        ));
    }

    #[test]
    fn ssh_record_requires_ssh_payload() {
        let record = ImportRecord {
            id: "xshell:prod".to_string(),
            importer_id: "xshell".to_string(),
            source_label: "Xshell".to_string(),
            kind: ImportRecordKind::Ssh,
            display_name: "Prod SSH".to_string(),
            database: Some(DatabaseImportRecord {
                database_type: ImportDatabaseType::MySql,
                name: "Prod MySQL".to_string(),
                host: "10.2.4.55".to_string(),
                port: Some(3306),
                username: "root".to_string(),
                password: None,
                database: Some("app".to_string()),
                extra_params: BTreeMap::new(),
            }),
            ssh: None,
            password_status: PasswordImportStatus::Unsupported,
            warnings: Vec::new(),
        };

        assert!(matches!(
            record.validate_shape(),
            Err(ImportProtocolError::MismatchedRecordPayload { .. })
        ));
    }

    #[test]
    fn password_status_survives_json_roundtrip() {
        let status = PasswordImportStatus::PermissionDenied;

        let json = serde_json::to_string(&status).unwrap();
        let decoded: PasswordImportStatus = serde_json::from_str(&json).unwrap();

        assert_eq!(status, decoded);
    }
}
