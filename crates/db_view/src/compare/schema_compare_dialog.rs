use db::compare::{SchemaCompareResult, SyncPlan};

/// 结构比较对话框
pub struct SchemaCompareDialog {
    result: Option<SchemaCompareResult>,
    sync_plan: Option<SyncPlan>,
}

impl SchemaCompareDialog {
    pub fn new() -> Self {
        Self {
            result: None,
            sync_plan: None,
        }
    }

    pub fn set_result(&mut self, result: SchemaCompareResult) {
        self.result = Some(result);
    }

    pub fn set_sync_plan(&mut self, plan: SyncPlan) {
        self.sync_plan = Some(plan);
    }

    pub fn result(&self) -> Option<&SchemaCompareResult> {
        self.result.as_ref()
    }

    pub fn sync_plan(&self) -> Option<&SyncPlan> {
        self.sync_plan.as_ref()
    }
}
