use super::*;

#[test]
fn external_driver_id_for_connection_form_uses_editing_connection() {
    let connection = stored_external_connection("dm");

    assert_eq!(
        Some("dm".to_string()),
        external_driver_id_for_connection_form(&DatabaseType::MySQL, Some(&connection))
    );
}

#[test]
fn connection_title_prefers_external_driver_name() {
    assert_eq!(
        HomePage::connection_title_for_locale(
            "zh-CN",
            false,
            &DatabaseType::MySQL,
            None,
            Some("DM"),
        ),
        "新建 DM 连接"
    );
    assert_eq!(
        HomePage::connection_title_for_locale(
            "zh-CN",
            true,
            &DatabaseType::MySQL,
            None,
            Some("DM"),
        ),
        "编辑 DM 连接"
    );
}

#[test]
fn connection_title_prefers_connection_name_when_editing() {
    assert_eq!(
        HomePage::connection_title_for_locale(
            "zh-CN",
            true,
            &DatabaseType::MySQL,
            Some("生产库"),
            Some("DM"),
        ),
        "编辑 生产库 连接"
    );
    assert_eq!(
        HomePage::connection_title_for_locale(
            "zh-CN",
            false,
            &DatabaseType::MySQL,
            Some("生产库"),
            Some("DM"),
        ),
        "新建 DM 连接"
    );
}

#[test]
fn editing_title_or_default_prefers_connection_name() {
    let mut connection = stored_external_connection("dm");
    connection.name = "生产库".to_string();

    assert_eq!(
        HomePage::editing_title_or_default("zh-CN", Some(&connection), "编辑 SSH 连接".to_string(),),
        "编辑 生产库 连接"
    );

    connection.name = "  ".to_string();
    assert_eq!(
        HomePage::editing_title_or_default("zh-CN", Some(&connection), "编辑 SSH 连接".to_string(),),
        "编辑 SSH 连接"
    );
}

#[test]
fn typed_connection_title_uses_connection_name_only_when_editing() {
    let mut connection = stored_external_connection("dm");
    connection.name = "缓存集群".to_string();

    assert_eq!(
        HomePage::typed_connection_title_for_locale("zh-CN", true, "Redis", Some(&connection),),
        "编辑 缓存集群 连接"
    );
    assert_eq!(
        HomePage::typed_connection_title_for_locale("zh-CN", false, "Redis", Some(&connection),),
        "新建 Redis 连接"
    );
}

#[test]
fn remote_desktop_connection_info_uses_remote_desktop_params() {
    let params = RemoteDesktopParams {
        protocol: StoredRemoteDesktopProtocol::Rdp,
        host: "10.0.0.8".to_string(),
        port: 3389,
        username: Some("administrator".to_string()),
        password: None,
        credential_reference: None,
        domain: None,
        read_only: false,
        audio_playback: false,
        proxy: None,
        backend_preference: Default::default(),
        rdp: None,
    };

    assert_eq!(
        "administrator@10.0.0.8:3389",
        remote_desktop_connection_info(&params)
    );
}

#[test]
fn remote_desktop_connection_info_omits_missing_username() {
    let params = RemoteDesktopParams {
        protocol: StoredRemoteDesktopProtocol::Vnc,
        host: "10.0.0.9".to_string(),
        port: 5900,
        username: None,
        password: None,
        credential_reference: None,
        domain: None,
        read_only: false,
        audio_playback: false,
        proxy: None,
        backend_preference: Default::default(),
        rdp: None,
    };

    assert_eq!("10.0.0.9:5900", remote_desktop_connection_info(&params));
}

#[cfg(not(feature = "screenshot-safe"))]
#[test]
fn default_feature_set_preserves_connection_name() {
    let mut connection = stored_external_connection("dm");
    connection.name = "Production database".to_string();

    assert_eq!("Production database", connection_display_name(&connection));
}

#[cfg(feature = "screenshot-safe")]
#[test]
fn screenshot_safe_feature_replaces_all_home_connection_info() {
    let cases = [
        ConnectionType::Database,
        ConnectionType::SshSftp,
        ConnectionType::Redis,
        ConnectionType::MongoDB,
        ConnectionType::Serial,
        ConnectionType::PortForwarding,
        ConnectionType::Rdp,
        ConnectionType::Vnc,
    ];

    let mut names = std::collections::HashSet::new();
    for (offset, connection_type) in cases.into_iter().enumerate() {
        let connection = StoredConnection {
            id: Some(offset as i64 + 1),
            credential_revision: None,
            name: "Sensitive connection name".to_string(),
            connection_type,
            params: r#"{"host":"production.internal","username":"administrator"}"#.to_string(),
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
        };

        let name = connection_display_name(&connection);
        assert!(
            !name.contains("Sensitive"),
            "connection name was not replaced: {name}",
        );
        // 同一连接重复渲染结果稳定，不同连接之间不重名。
        assert_eq!(name, connection_display_name(&connection));
        assert!(names.insert(name), "duplicate display name");

        let info = card_connection_info(&connection)
            .unwrap_or_else(|| panic!("missing screenshot-safe info for {connection_type:?}"));
        assert!(
            !info.contains("production.internal") && !info.contains("administrator"),
            "connection info was not replaced: {info}",
        );
    }
}
