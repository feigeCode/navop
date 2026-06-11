use db::DbNode;

/// 数据比较窗口
pub struct DataCompareWindow {
    pub source_node: DbNode,
}

impl DataCompareWindow {
    pub fn new(source_node: DbNode) -> Self {
        Self { source_node }
    }
}
