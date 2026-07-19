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
        domain: None,
        read_only: false,
        proxy: None,
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
        domain: None,
        read_only: false,
        proxy: None,
    };

    assert_eq!("10.0.0.9:5900", remote_desktop_connection_info(&params));
}
