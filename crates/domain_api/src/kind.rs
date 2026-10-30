use gpui::{App, SharedString};

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

/// 一种可新建的连接类型。由领域注册，独立产品只展示已装配领域提供的种类。
#[derive(Clone)]
pub struct ConnectionKindDescriptor {
    pub id: SharedString,
    pub label: fn(&App) -> SharedString,
    pub group: ConnectionKindGroup,
    pub order: i32,
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
        }
    }

    pub fn display_label(&self, cx: &App) -> SharedString {
        (self.label)(cx)
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
}
