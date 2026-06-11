use db::compare::{DataCompareResult, SyncPlan};

/// 数据比较对话框
pub struct DataCompareDialog {
    result: Option<DataCompareResult>,
    sync_plan: Option<SyncPlan>,
}

impl DataCompareDialog {
    pub fn new() -> Self {
        Self {
            result: None,
            sync_plan: None,
        }
    }

    pub fn set_result(&mut self, result: DataCompareResult) {
        self.result = Some(result);
    }

    pub fn set_sync_plan(&mut self, plan: SyncPlan) {
        self.sync_plan = Some(plan);
    }

    pub fn result(&self) -> Option<&DataCompareResult> {
        self.result.as_ref()
    }

    pub fn sync_plan(&self) -> Option<&SyncPlan> {
        self.sync_plan.as_ref()
    }
}
