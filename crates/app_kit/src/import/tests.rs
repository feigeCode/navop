//! `import` 模块的纯逻辑单测（不依赖 gpui App 上下文）。

use super::*;

#[test]
fn every_product_has_supported_kinds() {
    let kinds = supported_connection_types();
    assert!(!kinds.is_empty());
    assert!(kinds.iter().all(|kind| {
        let canonical = ConnectionType::from_str(kind).to_string();
        &canonical == kind
    }));
}

#[test]
fn future_bundle_version_is_rejected() {
    let mut bundle = ConnectionBundle {
        version: BUNDLE_VERSION + 1,
        exported_at: 0,
        app_id: None,
        workspaces: vec![],
        connections: vec![],
        encrypted_credentials: None,
    };
    let error = ensure_supported_version(&bundle).unwrap_err();
    assert!(error.to_string().contains("高于当前支持"), "{error}");
    bundle.version = BUNDLE_VERSION;
    assert!(ensure_supported_version(&bundle).is_ok());
}

#[test]
fn broken_params_json_is_rejected_before_writing() {
    let connection = BundleConnection {
        name: "坏包".to_string(),
        connection_type: "Database".to_string(),
        params: "{not json".to_string(),
        workspace: None,
        remark: None,
        selected_databases: None,
        sync_enabled: true,
    };
    let error = validate_params(&connection).unwrap_err();
    assert!(error.to_string().contains("params 不是合法 JSON"));
    let ok = BundleConnection {
        params: r#"{"host":"127.0.0.1"}"#.to_string(),
        ..connection
    };
    assert!(validate_params(&ok).is_ok());
}

#[test]
fn v3_credentials_are_restored_only_with_the_export_password() {
    let secret_params = r#"{"host":"db.internal","password":"p@ss","private_key":"key"}"#;
    let mut bundle = protected_bundle(secret_params);
    apply_bundle_credentials(&mut bundle, Some("export-pass")).expect("正确密码应能解出凭据");
    assert_eq!(bundle.connections[0].params, secret_params);
    assert!(
        bundle.encrypted_credentials.is_none(),
        "导入后不应再保留密文"
    );

    let mut bundle = protected_bundle(secret_params);
    let error = apply_bundle_credentials(&mut bundle, Some("wrong-pass")).unwrap_err();
    assert!(error.to_string().contains("密码错误"), "{error}");
    assert!(
        bundle.encrypted_credentials.is_some(),
        "解密失败时不应丢弃密文"
    );

    let mut bundle = protected_bundle(secret_params);
    let error = apply_bundle_credentials(&mut bundle, None).unwrap_err();
    assert!(
        error.to_string().contains("请输入导出时设置的密码"),
        "{error}"
    );
}

#[test]
fn imported_bundle_with_credentials_rejects_dangling_references() {
    let connection = BundleConnection {
        name: "主库".to_string(),
        connection_type: "Database".to_string(),
        params: r#"{"credential_reference":{"credential_id":9,"credential_cloud_id":null,"username":true,"password":true,"private_key":false,"passphrase":false}}"#.to_string(),
        workspace: None,
        remark: None,
        selected_databases: None,
        sync_enabled: true,
    };
    let credential = BundleCredential {
        id: 9,
        name: "生产库".to_string(),
        username: None,
        password: Some("secret".to_string()),
        private_key_path: None,
        private_key_content: None,
        passphrase: None,
        ssh_expect: Default::default(),
        sync_enabled: false,
        forward_to_agent: false,
        cloud_id: None,
    };
    validate_bundle_credential_references(&[connection.clone()], &[credential.clone()])
        .expect("引用命中应当通过");

    let missing = BundleCredential {
        id: 10,
        ..credential
    };
    let error = validate_bundle_credential_references(&[connection], &[missing])
        .expect_err("悬空引用应失败");
    assert!(error.to_string().contains("不存在的钥匙串条目"), "{error}");
}

#[test]
fn incomplete_credentials_payload_is_rejected() {
    let mut bundle = protected_bundle(r#"{"host":"db.internal","password":"p@ss"}"#);
    let payload = CredentialsPayload {
        version: CREDENTIALS_PAYLOAD_VERSION,
        connections: vec![],
        credentials: vec![],
    };
    let ciphertext =
        crypto::encrypt_with_passphrase(&serde_json::to_string(&payload).unwrap(), "export-pass")
            .unwrap();
    bundle.encrypted_credentials = Some(EncryptedCredentials {
        format: CREDENTIALS_FORMAT.to_string(),
        ciphertext,
    });
    let error = apply_bundle_credentials(&mut bundle, Some("export-pass")).unwrap_err();
    assert!(error.to_string().contains("缺少连接"), "{error}");
}

#[test]
fn legacy_bundle_without_credentials_needs_no_password() {
    let mut bundle = ConnectionBundle {
        version: BUNDLE_VERSION - 1,
        exported_at: 0,
        app_id: None,
        workspaces: vec![],
        connections: vec![BundleConnection {
            name: "旧包".to_string(),
            connection_type: "Database".to_string(),
            params: r#"{"host":"127.0.0.1"}"#.to_string(),
            workspace: None,
            remark: None,
            selected_databases: None,
            sync_enabled: true,
        }],
        encrypted_credentials: None,
    };
    apply_bundle_credentials(&mut bundle, None).expect("旧包不要求密码");
    assert_eq!(bundle.connections[0].params, r#"{"host":"127.0.0.1"}"#);
}

/// 构造一个 v3 迁移包：可见参数已脱敏，完整参数放在加密凭据里。
fn protected_bundle(secret_params: &str) -> ConnectionBundle {
    let payload = CredentialsPayload {
        version: CREDENTIALS_PAYLOAD_VERSION,
        connections: vec![ConnectionCredentials {
            index: 0,
            params: secret_params.to_string(),
        }],
        credentials: vec![],
    };
    let ciphertext =
        crypto::encrypt_with_passphrase(&serde_json::to_string(&payload).unwrap(), "export-pass")
            .unwrap();
    ConnectionBundle {
        version: BUNDLE_VERSION,
        exported_at: 0,
        app_id: None,
        workspaces: vec![],
        connections: vec![BundleConnection {
            name: "主库".to_string(),
            connection_type: "Database".to_string(),
            params: r#"{"host":"db.internal","password":"","private_key":""}"#.to_string(),
            workspace: None,
            remark: None,
            selected_databases: None,
            sync_enabled: true,
        }],
        encrypted_credentials: Some(EncryptedCredentials {
            format: CREDENTIALS_FORMAT.to_string(),
            ciphertext,
        }),
    }
}

#[test]
fn expands_tilde_quotes_and_scheme_in_source_path() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    assert_eq!(
        expand_source_path("~/Desktop/one-hub.db"),
        home.join("Desktop/one-hub.db")
    );
    assert_eq!(
        expand_source_path("  ~/Desktop/one-hub.db  "),
        home.join("Desktop/one-hub.db")
    );
    assert_eq!(
        expand_source_path("\"~/Desktop/one-hub.db\""),
        home.join("Desktop/one-hub.db")
    );
    assert_eq!(
        expand_source_path("file:///tmp/navop-connections.json"),
        PathBuf::from("/tmp/navop-connections.json")
    );
    assert_eq!(expand_source_path("~"), home);
}

#[test]
fn keeps_absolute_and_unknown_variable_paths() {
    let absolute = PathBuf::from("/tmp/navop-connections.json");
    assert_eq!(expand_source_path("/tmp/navop-connections.json"), absolute);
    // 未定义的变量原样保留，避免把 `%VAR%` 吞成空路径。
    assert_eq!(
        expand_source_path("%NAVOP_TESTS_UNSET_VAR%\\one-hub.db"),
        PathBuf::from("%NAVOP_TESTS_UNSET_VAR%\\one-hub.db")
    );
    // 只有前缀是 `~` 才算 home，`~user` 之类不动。
    assert_eq!(
        expand_source_path("~someone/db"),
        PathBuf::from("~someone/db")
    );
}

#[test]
fn export_encrypts_credentials_and_redacts_visible_params() {
    let secret = r#"{"host":"db.internal","port":5432,"password":"p@ss","private_key":"k"}"#;
    let plain = r#"{"host":"cache.internal","port":6379}"#;
    let connections = [
        stored_connection("主库", secret),
        stored_connection("缓存", plain),
    ];

    let bundle = build_bundle(
        &connections,
        &[],
        &[],
        Some("navop".to_string()),
        "export-pass",
    )
    .expect("导出应成功");
    let json = serde_json::to_string_pretty(&bundle).unwrap();
    assert!(!json.contains("p@ss"), "导出 JSON 不应包含明文密码");
    assert!(!json.contains("\"k\""), "导出 JSON 不应包含明文私钥");
    assert!(json.contains("encrypted_credentials"), "应包含加密凭据字段");
    let visible = &bundle.connections[0].params;
    assert!(visible.contains("\"host\":\"db.internal\""), "{visible}");
    assert!(visible.contains("\"password\":\"\""), "{visible}");

    let mut bundle = bundle;
    apply_bundle_credentials(&mut bundle, Some("export-pass")).expect("正确密码应能还原凭据");
    assert_eq!(bundle.connections[0].params, secret);
    assert_eq!(bundle.connections[1].params, plain);
}

#[test]
fn export_rejects_empty_password_and_undecryptable_credentials() {
    let connections = [stored_connection("主库", r#"{"host":"db"}"#)];
    let error = build_bundle(&connections, &[], &[], None, "").unwrap_err();
    assert!(error.to_string().contains("导出密码不能为空"), "{error}");

    let locked = [stored_connection("锁定的库", r#"{"password":"ENC:AAAA"}"#)];
    let error = build_bundle(&locked, &[], &[], None, "export-pass").unwrap_err();
    assert!(error.to_string().contains("无法解密"), "{error}");

    let broken = [stored_connection("坏配置", "password=plain-text")];
    let error = build_bundle(&broken, &[], &[], None, "export-pass").unwrap_err();
    assert!(error.to_string().contains("无法安全脱敏"), "{error}");
}

#[test]
fn export_carries_only_referenced_vault_credentials() {
    let params = r#"{"host":"db.internal","credential_reference":{"credential_id":7,"username":true,"password":true,"private_key":false,"passphrase":false}}"#;
    let connections = [stored_connection("主库", params)];
    let credentials = [
        credential_entry(7, "生产库", "vault-secret"),
        credential_entry(8, "无关条目", "unrelated-secret"),
    ];

    let bundle = build_bundle(
        &connections,
        &[],
        &credentials,
        Some("navop".to_string()),
        "export-pass",
    )
    .expect("导出应成功");
    let json = serde_json::to_string(&bundle).unwrap();
    assert!(!json.contains("vault-secret"), "可见 JSON 不应泄漏条目密码");
    assert!(
        !json.contains("unrelated-secret"),
        "可见 JSON 不应泄漏条目密码"
    );

    let ciphertext = bundle
        .encrypted_credentials
        .as_ref()
        .expect("应包含加密凭据")
        .ciphertext
        .clone();
    let payload = crypto::decrypt_with_passphrase(&ciphertext, "export-pass").unwrap();
    let payload: CredentialsPayload = serde_json::from_str(&payload).unwrap();
    assert_eq!(payload.credentials.len(), 1, "只应带被引用的条目");
    assert_eq!(payload.credentials[0].id, 7);
    assert_eq!(payload.credentials[0].name, "生产库");
    assert_eq!(
        payload.credentials[0].password.as_deref(),
        Some("vault-secret")
    );
}

#[test]
fn export_rejects_dangling_vault_references() {
    let params = r#"{"host":"db","credential_reference":{"credential_id":99,"password":true}}"#;
    let connections = [stored_connection("悬空引用", params)];
    let error = build_bundle(&connections, &[], &[], None, "export-pass").unwrap_err();
    assert!(error.to_string().contains("不存在的钥匙串条目"), "{error}");
}

#[test]
fn credential_references_are_remapped_recursively() {
    let params = r#"{
        "credential_reference": {"credential_id": 7, "credential_cloud_id": "cloud-7", "username": true},
        "jump_server": {"credential_reference": {"credential_id": 9, "credential_cloud_id": "cloud-9"}},
        "cloud_only": {"credential_reference": {"credential_id": 999, "credential_cloud_id": "cloud-7"}},
        "config": {"mysql": {"credential_reference": {"credential_id": 7}}},
        "plain": {"credential_id": 7}
    }"#;
    let mapping = CredentialMapping {
        by_id: HashMap::from([(7, (42, Some("cloud-42".to_string()))), (9, (43, None))]),
        by_cloud_id: HashMap::from([
            ("cloud-7".to_string(), (42, Some("cloud-42".to_string()))),
            ("cloud-9".to_string(), (43, None)),
        ]),
    };

    let remapped = remap_credential_references(params, &mapping).unwrap();
    let value: serde_json::Value = serde_json::from_str(&remapped).unwrap();
    assert_eq!(value["credential_reference"]["credential_id"], 42);
    assert_eq!(
        value["credential_reference"]["credential_cloud_id"],
        "cloud-42"
    );
    assert_eq!(
        value["jump_server"]["credential_reference"]["credential_id"],
        43
    );
    assert!(
        value["jump_server"]["credential_reference"]
            .get("credential_cloud_id")
            .is_none(),
        "目标条目没有 cloud id 时应删除源 cloud id，避免解析到错误条目"
    );
    assert_eq!(
        value["cloud_only"]["credential_reference"]["credential_id"], 42,
        "只有 cloud id 的引用也必须按 cloud id 重映射"
    );
    assert_eq!(
        value["config"]["mysql"]["credential_reference"]["credential_id"],
        42
    );
    assert_eq!(value["plain"]["credential_id"], 7, "非引用字段不得改动");
}

#[test]
fn credentials_payload_without_credentials_is_backward_compatible() {
    let payload: CredentialsPayload =
        serde_json::from_str(r#"{"version":1,"connections":[]}"#).unwrap();
    assert!(payload.credentials.is_empty());
}

fn credential_entry(id: i64, name: &str, password: &str) -> CredentialEntry {
    CredentialEntry {
        id: Some(id),
        name: name.to_string(),
        username: Some("root".to_string()),
        password: Some(password.to_string()),
        private_key_path: None,
        private_key_content: None,
        passphrase: None,
        ssh_expect: Default::default(),
        sync_enabled: false,
        forward_to_agent: false,
        cloud_id: None,
        last_synced_at: None,
        team_id: None,
        owner_id: None,
        created_at: None,
        updated_at: None,
    }
}

fn stored_connection(name: &str, params: &str) -> StoredConnection {
    StoredConnection {
        id: None,
        credential_revision: None,
        name: name.to_string(),
        connection_type: ConnectionType::Database,
        params: params.to_string(),
        workspace_id: None,
        selected_databases: None,
        remark: None,
        sync_enabled: true,
        cloud_id: None,
        last_synced_at: None,
        last_used_at: None,
        sort_order: None,
        created_at: None,
        updated_at: None,
        team_id: None,
        owner_id: None,
        preferred_open_mode: None,
        ai_access_level: Default::default(),
    }
}
