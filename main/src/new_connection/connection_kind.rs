use db::ipc::IpcDriverRegistry;
use gpui::{Styled, px};
use gpui_component::{Icon, IconName, Sizable};
use one_core::storage::DatabaseType;
use rust_i18n::t;

const BUILTIN_EXTERNAL_DRIVER_IDS: &[&str] = &["duckdb"];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum NewConnectionCategory {
    All,
    Database,
    NoSql,
    Terminal,
}

impl NewConnectionCategory {
    pub(super) fn all() -> [Self; 4] {
        [Self::All, Self::Database, Self::NoSql, Self::Terminal]
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::Database => "数据库",
            Self::NoSql => "NoSQL",
            Self::Terminal => "终端",
        }
    }

    pub(super) fn icon(self) -> IconName {
        match self {
            Self::All => IconName::AppsColor,
            Self::Database => IconName::Database,
            Self::NoSql => IconName::Server,
            Self::Terminal => IconName::Terminal,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(super) enum NewConnectionKind {
    Ssh,
    Terminal,
    Redis,
    MongoDB,
    Serial,
    Database(DatabaseType),
    ExternalDatabase {
        driver_id: String,
        name: String,
        description: String,
    },
}

impl NewConnectionKind {
    pub(super) fn all() -> Vec<Self> {
        let mut items = vec![
            Self::Ssh,
            Self::Terminal,
            Self::Redis,
            Self::MongoDB,
            Self::Serial,
        ];
        items.extend(
            DatabaseType::builtin_all()
                .iter()
                .cloned()
                .map(Self::Database),
        );
        items.extend(external_database_kinds(&IpcDriverRegistry::load_default()));
        items
    }

    pub(super) fn label(&self) -> String {
        match self {
            Self::Ssh => "SSH / SFTP".to_string(),
            Self::Terminal => "Terminal".to_string(),
            Self::Redis => "Redis".to_string(),
            Self::MongoDB => "MongoDB".to_string(),
            Self::Serial => t!("Serial.new").to_string(),
            Self::Database(db_type) => db_type.as_str().to_string(),
            Self::ExternalDatabase { name, .. } => name.clone(),
        }
    }

    pub(super) fn description(&self) -> String {
        match self {
            Self::Ssh => "远程服务器终端与文件连接".to_string(),
            Self::Terminal => "打开一个本地终端标签页".to_string(),
            Self::Redis => "Redis 单机、哨兵或集群连接".to_string(),
            Self::MongoDB => "MongoDB 数据库连接".to_string(),
            Self::Serial => "串口设备连接".to_string(),
            Self::Database(_) => "关系型数据库连接".to_string(),
            Self::ExternalDatabase { description, .. } => description.clone(),
        }
    }

    pub(super) fn category(&self) -> NewConnectionCategory {
        match self {
            Self::Ssh | Self::Terminal | Self::Serial => NewConnectionCategory::Terminal,
            Self::Redis | Self::MongoDB => NewConnectionCategory::NoSql,
            Self::Database(_) | Self::ExternalDatabase { .. } => NewConnectionCategory::Database,
        }
    }

    pub(super) fn icon(&self) -> Icon {
        match self {
            Self::Ssh => IconName::TerminalColor.color().with_size(px(40.0)),
            Self::Terminal => IconName::Terminal
                .mono()
                .text_color(gpui::rgb(0x8b5cf6))
                .with_size(px(40.0)),
            Self::Redis => IconName::Redis.color().with_size(px(40.0)),
            Self::MongoDB => IconName::MongoDB.color().with_size(px(40.0)),
            Self::Serial => IconName::SerialPort.color().with_size(px(40.0)),
            Self::Database(db_type) => db_type.as_icon().with_size(px(40.0)),
            Self::ExternalDatabase { .. } => IconName::Database.color().with_size(px(40.0)),
        }
    }
}

fn external_database_kinds(registry: &IpcDriverRegistry) -> Vec<NewConnectionKind> {
    registry
        .drivers()
        .iter()
        .filter(|driver| !is_builtin_external_driver(&driver.id))
        .map(|driver| NewConnectionKind::ExternalDatabase {
            driver_id: driver.id.clone(),
            name: driver.name.clone(),
            description: driver.description.clone(),
        })
        .collect()
}

fn is_builtin_external_driver(driver_id: &str) -> bool {
    BUILTIN_EXTERNAL_DRIVER_IDS.contains(&driver_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::ipc::{IpcDriverEntry, IpcDriverManifest, IpcDriverRegistry, IpcDriverTransport};
    use std::path::PathBuf;

    #[test]
    fn external_database_kinds_skip_builtin_duckdb_driver() {
        let registry = IpcDriverRegistry::from_drivers(vec![
            manifest("duckdb", "DuckDB"),
            manifest("custom", "Custom"),
        ]);

        let ids: Vec<String> = external_database_kinds(&registry)
            .into_iter()
            .filter_map(|kind| match kind {
                NewConnectionKind::ExternalDatabase { driver_id, .. } => Some(driver_id),
                _ => None,
            })
            .collect();

        assert_eq!(ids, vec!["custom"]);
    }

    fn manifest(id: &str, name: &str) -> IpcDriverManifest {
        IpcDriverManifest {
            id: id.to_string(),
            name: name.to_string(),
            description: String::new(),
            version: String::new(),
            entry: IpcDriverEntry {
                command: "./driver".to_string(),
                args: Vec::new(),
                working_dir: None,
            },
            transport: IpcDriverTransport::local_socket(format!("{id}.sock")),
            dialect: Default::default(),
            capabilities: None,
            connection: Default::default(),
            methods: Vec::new(),
            ui: Default::default(),
            manifest_dir: PathBuf::from("."),
        }
    }
}
