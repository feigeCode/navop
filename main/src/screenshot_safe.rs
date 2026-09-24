//! `screenshot-safe` feature 下用于截图的脱敏展示数据。
//!
//! 首页连接卡片和左侧连接列表展示的名称与地址会被替换成这里生成的假数据：
//! 取值由连接 ID（工作区则用列表位置）稳定决定，所以同一个连接每次渲染结果
//! 一致、不同连接之间又各不相同——不是所有卡片都显示同一个占位符。
//!
//! 只替换展示字符串，不去改动 `HomePage.connections` / `HomePage.workspaces`，
//! 因此存储、加密、云同步和实际连接拿到的仍然是真实数据。
//!
//! 地址一律使用文档保留域名（`example.com`）和文档专用网段
//! （`192.0.2.0/24`、`198.51.100.0/24`、`203.0.113.0/24`），保证不会指向真实主机。

use one_core::storage::ConnectionType;

const ENVS: &[&str] = &["prod", "staging", "dev", "test", "uat", "sandbox"];

const CN_LABELS: &[&str] = &[
    "订单系统",
    "用户中心",
    "支付网关",
    "风控平台",
    "数据分析",
    "内网测试机",
    "日志中心",
    "消息队列",
    "监控节点",
    "跳板机",
];

const USERS: &[&str] = &["app", "deploy", "readonly", "ops", "dev", "report"];

const HOSTS: &[&str] = &[
    "db-01.example.com",
    "db-02.example.com",
    "app-03.example.com",
    "cache-01.example.com",
    "192.0.2.11",
    "192.0.2.24",
    "198.51.100.7",
    "203.0.113.5",
];

const DATABASES: &[&str] = &["appdb", "orders", "analytics", "warehouse", "reporting"];

const DATABASE_PORTS: &[&str] = &["3306", "5432", "8123", "1433", "1521"];

const SERIAL_INFO: &str = "COM1 (115200, 8N1)";

const WORKSPACES: &[&str] = &[
    "生产环境",
    "预发环境",
    "测试环境",
    "云主机",
    "内部工具",
    "客户接入",
    "归档连接",
    "个人项目",
    "Production",
    "Staging",
    "Internal Tools",
    "Customer Demos",
];

/// splitmix64：把标识符和一个固定盐值混成稳定的伪随机数。
fn mix(seed: u64, salt: u64) -> u64 {
    let mut z = (seed ^ salt).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn index(seed: u64, salt: u64, modulus: usize) -> usize {
    (mix(seed, salt) % modulus.max(1) as u64) as usize
}

fn pick<'a>(pool: &'a [&'a str], seed: u64, salt: u64) -> &'a str {
    pool[index(seed, salt, pool.len())]
}

/// 按连接类型给一组像模像样的主机名主干，让卡片看起来不像随手编的。
fn kind_pool(connection_type: ConnectionType) -> &'static [&'static str] {
    match connection_type {
        ConnectionType::Database => &[
            "mysql", "postgres", "clickhouse", "sqlserver", "oracle", "sqlite",
        ],
        ConnectionType::SshSftp => &["web", "app", "api", "bastion", "worker", "gateway"],
        ConnectionType::Ftp => &["ftp", "ftps"],
        ConnectionType::Redis => &["redis", "cache", "sentinel"],
        ConnectionType::MongoDB => &["mongo", "docdb"],
        ConnectionType::Mqtt => &["mqtt", "emqx"],
        ConnectionType::Serial => &["serial", "tty"],
        ConnectionType::Telnet => &["console", "switch", "router"],
        ConnectionType::Rdp => &["rdp", "win"],
        ConnectionType::Vnc => &["vnc", "kvm"],
        ConnectionType::PortForwarding => &["tunnel", "forward"],
        ConnectionType::Extension => &["extension", "plugin"],
        ConnectionType::All => &["connection"],
    }
}

/// 脱敏后的连接名：约三分之一用中文名，其余用 `环境-用途-序号` 形式。
/// 序号取真实连接 ID，因此同一份连接列表里不会出现重名。
pub(crate) fn connection_name(connection_type: ConnectionType, id: Option<i64>) -> String {
    let serial = id.unwrap_or_default().unsigned_abs();
    let seed = serial;

    if index(seed, 0x11, 3) == 0 {
        let label = pick(CN_LABELS, seed, 0x21);
        return format!("{label}-{serial:02}");
    }

    let env = pick(ENVS, seed, 0x31);
    let kind = pick(kind_pool(connection_type), seed, 0x32);
    format!("{env}-{kind}-{serial:02}")
}

/// 脱敏后的连接地址（卡片副标题）。
pub(crate) fn connection_info(connection_type: ConnectionType, id: Option<i64>) -> Option<String> {
    let seed = id.unwrap_or_default().unsigned_abs();
    let host = pick(HOSTS, seed, 0x41);
    let user = pick(USERS, seed, 0x42);

    Some(match connection_type {
        ConnectionType::Database => {
            let database = pick(DATABASES, seed, 0x43);
            let port = pick(DATABASE_PORTS, seed, 0x44);
            format!("{user}@{host}:{port}/{database}")
        }
        ConnectionType::SshSftp => format!("{user}@{host}:22"),
        ConnectionType::Ftp => format!("{user}@{host}:21"),
        ConnectionType::Redis => format!("{host}:6379/{}", index(seed, 0x45, 16)),
        ConnectionType::MongoDB => format!("{host}:27017"),
        ConnectionType::Mqtt => format!("{host}:1883"),
        ConnectionType::Serial => SERIAL_INFO.to_string(),
        ConnectionType::Telnet => format!("{host}:23"),
        ConnectionType::Rdp => format!("{user}@{host}:3389"),
        ConnectionType::Vnc => format!("{user}@{host}:5900"),
        ConnectionType::PortForwarding => {
            format!("localhost:{} -> {host}:80", 8000 + index(seed, 0x46, 500))
        }
        ConnectionType::Extension => "Local Extension".to_string(),
        ConnectionType::All => return None,
    })
}

/// 脱敏后的工作区名。按列表位置取，避免默认名重名（工作区没有稳定的唯一序号）。
pub(crate) fn workspace_name(position: usize) -> String {
    let pool = WORKSPACES;
    let name = pool[position % pool.len()];
    match position / pool.len() {
        0 => name.to_string(),
        round => format!("{name} {}", round + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TYPES: &[ConnectionType] = &[
        ConnectionType::Database,
        ConnectionType::SshSftp,
        ConnectionType::Ftp,
        ConnectionType::Redis,
        ConnectionType::MongoDB,
        ConnectionType::Mqtt,
        ConnectionType::Serial,
        ConnectionType::Telnet,
        ConnectionType::Rdp,
        ConnectionType::Vnc,
        ConnectionType::PortForwarding,
        ConnectionType::Extension,
    ];

    #[test]
    fn every_connection_type_has_a_name_and_an_address() {
        for (offset, connection_type) in TYPES.iter().enumerate() {
            let id = Some(offset as i64 + 1);
            let name = connection_name(*connection_type, id);
            let info = connection_info(*connection_type, id);

            assert!(!name.is_empty(), "empty name for {connection_type:?}");
            assert!(info.is_some_and(|info| !info.is_empty()));
        }
        assert_eq!(None, connection_info(ConnectionType::All, Some(1)));
    }

    #[test]
    fn names_are_unique_per_connection_id() {
        let mut names = std::collections::HashSet::new();
        for id in 1..=200 {
            let name = connection_name(ConnectionType::Database, Some(id));
            assert!(names.insert(name.clone()), "duplicate name: {name}");
        }
    }

    #[test]
    fn results_are_stable_for_the_same_connection() {
        for connection_type in TYPES {
            let id = Some(42);
            assert_eq!(
                connection_name(*connection_type, id),
                connection_name(*connection_type, id)
            );
            assert_eq!(
                connection_info(*connection_type, id),
                connection_info(*connection_type, id)
            );
        }
    }

    #[test]
    fn addresses_never_point_at_real_hosts() {
        for connection_type in TYPES {
            for id in 1..=200 {
                let info = connection_info(*connection_type, Some(id)).unwrap();
                if matches!(
                    connection_type,
                    ConnectionType::Serial | ConnectionType::Extension
                ) {
                    continue;
                }
                assert!(
                    info.contains("example.com") || info.contains("192.0.2.")
                        || info.contains("198.51.100.") || info.contains("203.0.113."),
                    "unexpected host in {info}"
                );
            }
        }
    }

    #[test]
    fn workspace_names_do_not_repeat() {
        let mut names = std::collections::HashSet::new();
        for position in 0..WORKSPACES.len() {
            assert!(names.insert(workspace_name(position)));
        }
        assert_eq!(workspace_name(WORKSPACES.len()), format!("生产环境 2"));
    }
}
