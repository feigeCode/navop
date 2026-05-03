use db::ipc::IpcDriverRegistry;
use gpui::{Styled, px};
use gpui_component::{Icon, IconName, Sizable};
use one_core::storage::DatabaseType;
use rust_i18n::t;

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
                .copied()
                .map(Self::Database),
        );
        items.extend(
            IpcDriverRegistry::load_default()
                .drivers()
                .iter()
                .map(|driver| Self::ExternalDatabase {
                    driver_id: driver.id.clone(),
                    name: driver.name.clone(),
                    description: driver.description.clone(),
                }),
        );
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
