//! 输入框上下文视图模型(纯展示)。
//!
//! 这些类型刻意**只用 `SharedString`、不依赖 `agent_runtime` / 任何业务 crate**,
//! 让 [`AgentInput`](super::AgentInput) 保持"哑组件":它只接收一份
//! [`AgentComposerContext`] 用于渲染顶部 Context Bar 与底部执行参数,具体数据
//! 由上层(集成视图)按连接类型映射并注入。
//!
//! 设计要点:
//! - `scopes` 是**开放 `Vec`**,数量随目标类型动态变化,不写死 Database/Schema;
//! - 模型下拉项用 [`ComposerModelOption`] 表达,避免从展示文案反解析 provider/model;
//! - 工具模式 / 任务模式的下拉项用 [`ComposerMenuOption`] 表达。

use gpui::SharedString;

/// 顶部主「目标」chip 的展示数据。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposerTarget {
    /// 目标标识(emit 选择事件时回传,通常对应 connection_id)。
    pub id: SharedString,
    /// 主标题(如 `prod-mysql`)。
    pub label: SharedString,
    /// 图标缩写(如 `DB`/`SH`/`RD`),纯展示。
    pub icon: SharedString,
    /// 类型文案(如 `database`),同时进入能力标签。
    pub kind: SharedString,
    /// 副标题(如 `MySQL · 10.0.0.12:3306`),用于选择器列表。
    pub subtitle: SharedString,
}

impl ComposerTarget {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        icon: impl Into<SharedString>,
        kind: impl Into<SharedString>,
        subtitle: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: icon.into(),
            kind: kind.into(),
            subtitle: subtitle.into(),
        }
    }
}

/// 派生上下文 chip(数量随目标类型动态变化)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerScope {
    /// emit [`PickScope`](super::AgentInputEvent::PickScope) 时回传的标识。
    pub key: SharedString,
    /// 维度名(如 `Database`/`Schema`/`目录`)。
    pub label: SharedString,
    /// 当前取值(如 `ai_app`/`public`)。
    pub value: SharedString,
}

impl ComposerScope {
    pub fn new(
        key: impl Into<SharedString>,
        label: impl Into<SharedString>,
        value: impl Into<SharedString>,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value: value.into(),
        }
    }
}

/// 底部模型选择器的当前值(本轮执行参数)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ComposerModel {
    pub provider: SharedString,
    pub model: SharedString,
}

impl ComposerModel {
    pub fn new(provider: impl Into<SharedString>, model: impl Into<SharedString>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}

/// 底部模型选择器的结构化选项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerModelOption {
    /// 选项标识(emit 选择事件时回传)。
    pub id: SharedString,
    /// provider 的稳定标识,用于重建运行时。
    pub provider_id: SharedString,
    /// provider 展示名。
    pub provider_label: SharedString,
    /// 模型名,原样传给模型客户端。
    pub model: SharedString,
    /// 次要说明(可选,显示在标签下方)。
    pub hint: Option<SharedString>,
}

impl ComposerModelOption {
    pub fn new(
        id: impl Into<SharedString>,
        provider_id: impl Into<SharedString>,
        provider_label: impl Into<SharedString>,
        model: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            provider_id: provider_id.into(),
            provider_label: provider_label.into(),
            model: model.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn display_label(&self) -> SharedString {
        SharedString::from(format!("{} / {}", self.provider_label, self.model))
    }

    pub fn to_composer_model(&self) -> ComposerModel {
        ComposerModel::new(self.provider_label.clone(), self.model.clone())
    }
}

/// 内置下拉菜单的一个选项(模型 / 工具模式 / 任务模式共用)。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerMenuOption {
    /// 选项标识(emit 选择事件时回传)。
    pub id: SharedString,
    /// 展示文案。
    pub label: SharedString,
    /// 次要说明(可选,显示在标签下方)。
    pub hint: Option<SharedString>,
}

impl ComposerMenuOption {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: None,
        }
    }

    pub fn with_hint(mut self, hint: impl Into<SharedString>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}

/// 顶部「计划」面板中的一步。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerPlanItem {
    pub title: SharedString,
    pub status: SharedString,
    pub description: SharedString,
    pub risk: SharedString,
    pub tool: Option<SharedString>,
}

impl ComposerPlanItem {
    pub fn new(title: impl Into<SharedString>, status: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            status: status.into(),
            description: SharedString::default(),
            risk: SharedString::default(),
            tool: None,
        }
    }

    pub fn with_details(
        mut self,
        description: impl Into<SharedString>,
        risk: impl Into<SharedString>,
        tool: Option<SharedString>,
    ) -> Self {
        self.description = description.into();
        self.risk = risk.into();
        self.tool = tool;
        self
    }

    pub fn has_details(&self) -> bool {
        !self.description.is_empty() || !self.risk.is_empty() || self.tool.is_some()
    }
}

/// 顶部「子代理」面板中的最近子代理。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerSubAgentItem {
    pub id: SharedString,
    pub name: SharedString,
    pub task: SharedString,
    pub summary: SharedString,
    pub status: SharedString,
}

impl ComposerSubAgentItem {
    pub fn new(
        id: impl Into<SharedString>,
        name: impl Into<SharedString>,
        task: impl Into<SharedString>,
        status: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            task: task.into(),
            summary: SharedString::default(),
            status: status.into(),
        }
    }

    pub fn with_summary(mut self, summary: impl Into<SharedString>) -> Self {
        self.summary = summary.into();
        self
    }
}

/// 顶部「Agent」面板中的可选执行后端。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComposerAgentOption {
    /// `None` 表示内置 Agent;`Some(id)` 表示外部 ACP Agent。
    pub id: Option<SharedString>,
    pub label: SharedString,
    pub subtitle: SharedString,
    pub selected: bool,
    pub connecting: bool,
}

impl ComposerAgentOption {
    pub fn local(label: impl Into<SharedString>, selected: bool, connecting: bool) -> Self {
        Self {
            id: None,
            label: label.into(),
            subtitle: SharedString::from("内置 Agent"),
            selected,
            connecting,
        }
    }

    pub fn acp(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        selected: bool,
        connecting: bool,
    ) -> Self {
        Self {
            id: Some(id.into()),
            label: label.into(),
            subtitle: SharedString::from("ACP Agent"),
            selected,
            connecting,
        }
    }

    pub fn element_id(&self) -> SharedString {
        match &self.id {
            Some(id) => SharedString::from(format!("agent-option-acp-{id}")),
            None => SharedString::from("agent-option-local"),
        }
    }
}

/// 注入给输入框的整体上下文(只读展示)。
///
/// 由上层构造并通过 [`AgentInput::set_context`](super::AgentInput::set_context) 注入。
#[derive(Clone, Debug, Default)]
pub struct AgentComposerContext {
    /// 当前目标;`None` 时顶部显示「选择目标」占位 chip。
    pub target: Option<ComposerTarget>,
    /// 派生上下文 chip(动态数量)。
    pub scopes: Vec<ComposerScope>,
    /// 右侧能力标签。
    pub capabilities: Vec<SharedString>,
    /// 当前计划的 todo 列表,显示在顶部「计划」面板。
    pub plan_items: Vec<ComposerPlanItem>,
    /// 当前会话最近的子代理列表,显示在顶部「子代理」面板。
    pub subagent_items: Vec<ComposerSubAgentItem>,
    /// 内置 Agent 与 ACP Agent 切换项,显示在顶部「Agent」面板。
    pub agent_options: Vec<ComposerAgentOption>,
    /// 当前模型(底部高亮 chip);`None` 时显示「选择模型」。
    pub model: Option<ComposerModel>,
    /// 工具模式当前文案(如 `自动`)。
    pub tool_label: SharedString,
    /// 任务模式当前文案(如 `诊断`)。
    pub task_label: SharedString,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_option_hint_is_optional() {
        let plain = ComposerMenuOption::new("a", "Alpha");
        assert!(plain.hint.is_none());
        let hinted = ComposerMenuOption::new("b", "Beta").with_hint("快");
        assert_eq!(hinted.hint, Some(SharedString::from("快")));
    }

    #[test]
    fn plan_item_details_are_optional() {
        let plain = ComposerPlanItem::new("检查连接", "pending");
        assert!(!plain.has_details());

        let detailed = ComposerPlanItem::new("执行测试", "running").with_details(
            "运行相关验证",
            "只读",
            Some(SharedString::from("cargo test")),
        );

        assert!(detailed.has_details());
        assert_eq!(detailed.description.as_ref(), "运行相关验证");
        assert_eq!(detailed.risk.as_ref(), "只读");
        assert_eq!(
            detailed.tool.as_ref().map(|s| s.as_ref()),
            Some("cargo test")
        );
    }

    #[test]
    fn default_context_is_empty() {
        let ctx = AgentComposerContext::default();
        assert!(ctx.target.is_none());
        assert!(ctx.scopes.is_empty());
        assert!(ctx.model.is_none());
    }

    #[test]
    fn model_option_keeps_provider_and_model_structured() {
        let option = ComposerModelOption::new("openai:gpt-4.1", "openai", "OpenAI", "gpt-4.1")
            .with_hint("默认");

        assert_eq!(option.id.as_ref(), "openai:gpt-4.1");
        assert_eq!(option.provider_id.as_ref(), "openai");
        assert_eq!(option.provider_label.as_ref(), "OpenAI");
        assert_eq!(option.model.as_ref(), "gpt-4.1");
        assert_eq!(option.display_label().as_ref(), "OpenAI / gpt-4.1");
        assert_eq!(
            option.to_composer_model(),
            ComposerModel::new("OpenAI", "gpt-4.1")
        );
    }
}
