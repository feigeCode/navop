//! 当前产品支持的连接类型。
//!
//! `registry` 里领域注册的 kind id（如 `ssh` / `db:postgres`）与
//! `one_core::storage::ConnectionType` 不同构，无法反查，因此按产品身份静态列出。

/// 全量产品支持全部连接类型。
///
/// 不含 `All`：它只是「全部」筛选用的伪类型，`ConnectionType::from_str` 无法
/// 往返（会回退成 `Database`）。
const ALL: [&str; 12] = [
    "Database",
    "SshSftp",
    "Ftp",
    "Redis",
    "MongoDB",
    "Mqtt",
    "Serial",
    "Telnet",
    "PortForwarding",
    "Rdp",
    "Vnc",
    "Extension",
];

/// shell 产品只装了终端与远程桌面类连接。
const SHELL: [&str; 7] = [
    "Ftp",
    "Serial",
    "Telnet",
    "SshSftp",
    "PortForwarding",
    "Rdp",
    "Vnc",
];

/// data 产品只装了数据库类连接。
const DATA: [&str; 3] = ["Database", "Redis", "MongoDB"];

/// 当前产品支持的连接类型名（与 `ConnectionType` 的 `Display` 一致）。
pub(crate) fn supported_connection_types() -> Vec<String> {
    let kinds: &[&str] = match crate::identity().map(|identity| identity.app_id) {
        Some("navop-shell") => &SHELL,
        Some("navop-data") => &DATA,
        _ => &ALL,
    };
    kinds.iter().map(|kind| (*kind).to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::ConnectionType;

    #[test]
    fn kinds_round_trip_through_connection_type() {
        for kind in ALL.iter().chain(SHELL.iter()).chain(DATA.iter()) {
            let canonical = ConnectionType::from_str(kind).to_string();
            assert_eq!(&canonical, kind, "类型名必须能被 ConnectionType 原样解析");
        }
    }

    #[test]
    fn shell_and_data_sets_are_subset_of_all() {
        for kind in SHELL.iter().chain(DATA.iter()) {
            assert!(ALL.contains(kind), "{kind} 不在全量类型集合中");
        }
    }
}
