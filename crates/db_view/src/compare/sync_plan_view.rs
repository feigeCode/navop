use db::compare::SyncPlan;
use rust_i18n::t;

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
        t!(
            "Compare.sync_plan_summary",
            insert = self.plan.summary.insert_count,
            update = self.plan.summary.update_count,
            delete = self.plan.summary.delete_count,
            ddl = self.plan.summary.ddl_count,
            total = self.plan.summary.total_count
        )
        .to_string()
    }

    pub fn sql_text(&self) -> &str {
        &self.plan.sql_text
    }
}
