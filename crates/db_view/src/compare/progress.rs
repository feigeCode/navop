use gpui::SharedString;

/// 比较任务的阶段性进度
///
/// 由执行器(executor)通过通道发送,窗口据此渲染进度条与阶段文本。
#[derive(Debug, Clone)]
pub struct CompareProgress {
    /// 当前阶段的中文描述,例如「正在读取源表结构」
    pub phase: SharedString,
    /// 已完成步骤数(确定型进度);`total` 为 0 时忽略
    pub current: usize,
    /// 总步骤数;为 0 表示不确定进度(只展示阶段文本)
    pub total: usize,
}

impl CompareProgress {
    /// 确定型进度:已知总步骤数
    pub fn steps(phase: impl Into<SharedString>, current: usize, total: usize) -> Self {
        Self {
            phase: phase.into(),
            current,
            total,
        }
    }

    /// 不确定进度:仅展示阶段文本
    pub fn phase(phase: impl Into<SharedString>) -> Self {
        Self {
            phase: phase.into(),
            current: 0,
            total: 0,
        }
    }

    /// 阶段文本(确定型时附带「(已完成/总数)」计数)
    pub fn label(&self) -> String {
        if self.total > 0 {
            format!(
                "{} ({}/{})",
                self.phase,
                self.current.min(self.total),
                self.total
            )
        } else {
            self.phase.to_string()
        }
    }

    /// 百分比 0..=100;不确定进度返回 `None`
    pub fn percentage(&self) -> Option<f32> {
        if self.total == 0 {
            return None;
        }
        let value = (self.current as f32 / self.total as f32) * 100.0;
        Some(value.clamp(0.0, 100.0))
    }
}
