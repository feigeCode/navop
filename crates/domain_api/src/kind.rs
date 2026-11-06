use gpui::{App, SharedString};
use std::path::PathBuf;

/// 新建连接分类，决定向导左栏的分组顺序。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConnectionKindGroup {
    Database,
    Terminal,
    Remote,
    Extension,
}

impl ConnectionKindGroup {
    pub fn order(self) -> i32 {
        match self {
            Self::Database => 0,
            Self::Terminal => 1,
            Self::Remote => 2,
            Self::Extension => 3,
        }
    }
}

/// 连接种类的图标来源。
///
/// 领域负责把驱动清单里的图标解析成可渲染的来源，壳只按路径画图：
/// 随包资产交给应用资产源，磁盘文件走本地图标命名空间。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionKindIcon {
    /// 无 scheme 的资产路径（例如 `driver-icons/{driver}/{resource}.svg`）。
    Asset(SharedString),
    /// 磁盘上的图标文件。
    File(PathBuf),
}

/// 一种可新建的连接类型。由领域注册，独立产品只展示已装配领域提供的种类。
#[derive(Clone)]
pub struct ConnectionKindDescriptor {
    pub id: SharedString,
    pub label: fn(&App) -> SharedString,
    pub group: ConnectionKindGroup,
    pub order: i32,
    /// 标签覆盖：名字来自运行时清单的种类（如外部数据库驱动）用它，
    /// 优先于 `label`。
    label_override: Option<SharedString>,
    /// 卡片描述；`None` 时由壳按种类 ID / 分组给默认文案。
    description: Option<SharedString>,
    /// 旧版类目覆盖（如 `domestic_database`）；`None` 时壳按分组推导。
    category: Option<SharedString>,
    /// 图标来源；`None` 时由壳按种类 ID / 分组给默认图标。
    icon: Option<ConnectionKindIcon>,
}

impl ConnectionKindDescriptor {
    pub fn new(
        id: impl Into<SharedString>,
        label: fn(&App) -> SharedString,
        group: ConnectionKindGroup,
        order: i32,
    ) -> Self {
        Self {
            id: id.into(),
            label,
            group,
            order,
            label_override: None,
            description: None,
            category: None,
            icon: None,
        }
    }

    /// 覆盖标签（用于运行时清单提供的名字）。
    pub fn with_label(mut self, label: impl Into<SharedString>) -> Self {
        self.label_override = Some(label.into());
        self
    }

    /// 覆盖卡片描述。
    pub fn with_description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// 覆盖旧版类目（`domestic_database` / `time_series` / `middleware` 等）。
    pub fn with_category(mut self, category: impl Into<SharedString>) -> Self {
        self.category = Some(category.into());
        self
    }

    /// 覆盖图标。
    pub fn with_icon(mut self, icon: ConnectionKindIcon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn display_label(&self, cx: &App) -> SharedString {
        self.label_override
            .clone()
            .unwrap_or_else(|| (self.label)(cx))
    }

    /// 运行时覆盖的标签。
    pub fn label_override(&self) -> Option<&SharedString> {
        self.label_override.as_ref()
    }

    /// 运行时可选的卡片描述。
    pub fn description(&self) -> Option<&SharedString> {
        self.description.as_ref()
    }

    /// 运行时声明的旧版类目。
    pub fn category(&self) -> Option<&SharedString> {
        self.category.as_ref()
    }

    /// 运行时可选的图标来源。
    pub fn icon(&self) -> Option<&ConnectionKindIcon> {
        self.icon.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn label(cx: &App) -> SharedString {
        let _ = cx;
        "x".into()
    }

    #[test]
    fn group_order_is_stable() {
        assert_eq!(0, ConnectionKindGroup::Database.order());
        assert_eq!(3, ConnectionKindGroup::Extension.order());
    }

    #[test]
    fn descriptor_keeps_label_fn() {
        let kind = ConnectionKindDescriptor::new("ssh", label, ConnectionKindGroup::Terminal, 1);
        assert_eq!("ssh", kind.id.as_ref());
        assert_eq!(ConnectionKindGroup::Terminal, kind.group);
    }

    #[test]
    fn runtime_metadata_overrides_defaults() {
        let kind = ConnectionKindDescriptor::new(
            "db:external:kingbase",
            label,
            ConnectionKindGroup::Database,
            200,
        )
        .with_label("KingbaseES")
        .with_description("国产数据库")
        .with_category("domestic_database")
        .with_icon(ConnectionKindIcon::File(PathBuf::from("/tmp/kingbase.svg")));
        assert_eq!(
            Some("KingbaseES"),
            kind.label_override().map(|value| value.as_ref())
        );
        assert_eq!(
            Some("domestic_database"),
            kind.category().map(|value| value.as_ref())
        );
        assert_eq!(
            Some("国产数据库"),
            kind.description().map(|value| value.as_ref())
        );
        assert!(matches!(kind.icon(), Some(ConnectionKindIcon::File(_))));
    }
}
