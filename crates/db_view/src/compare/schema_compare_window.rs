use db::DbNode;

/// 结构比较窗口
pub struct SchemaCompareWindow {
    pub source_node: DbNode,
}

impl SchemaCompareWindow {
    pub fn new(source_node: DbNode) -> Self {
        Self { source_node }
    }
}
