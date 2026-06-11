use db::compare::SyncPlan;

/// 同步计划预览视图
pub struct SyncPlanView {
    plan: SyncPlan,
}

impl SyncPlanView {
    pub fn new(plan: SyncPlan) -> Self {
        Self { plan }
    }

    pub fn plan(&self) -> &SyncPlan {
        &self.plan
    }

    pub fn summary_text(&self) -> String {
        format!(
            "INSERT: {} | UPDATE: {} | DELETE: {} | DDL: {} | 总计: {}",
            self.plan.summary.insert_count,
            self.plan.summary.update_count,
            self.plan.summary.delete_count,
            self.plan.summary.ddl_count,
            self.plan.summary.total_count
        )
    }

    pub fn sql_text(&self) -> &str {
        &self.plan.sql_text
    }
}
