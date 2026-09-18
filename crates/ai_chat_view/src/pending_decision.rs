//! 待决策项的统一投影：把消息流里「等待用户决定」的卡片收敛成一份清单。
//!
//! 时间线里的审批卡与输入区上方的决策栏读**同一份投影**，于是：
//! - 两处永远是同一个决策（同一个 `id`），不会出现「显示了两份所以要点两次」；
//! - 决策落地只走一个入口（派发内联卡片用的同一个 action），重复点击天然幂等。
//!
//! **三个来源保持独立授权域**（本地工具 / ACP permission / Public MCP）：
//! 投影只做关联展示与来源标注，绝不把三个域合并成一个授权。
//!
//! 本模块不依赖 GPUI，投影规则用普通单元测试锁住；渲染见 `agent_view`。

use crate::ChatMessageUI;
use crate::agent_cards::{
    ACP_PERMISSION_CARD, AcpPermissionCardData, TOOL_CONFIRM_CARD, ToolConfirmCardData,
};

/// 待决策项来自哪一类卡片。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionCardSource {
    /// 本地工具确认卡（`agent.confirm`）；Public MCP 审批同样落在这张卡上，靠授权域区分。
    ToolConfirm,
    /// ACP 协议权限请求卡（`acp.permission`）。
    AcpPermission,
}

/// 授权域。三者**不合并**：一个按钮只作用于一个域。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionAuthority {
    /// 内置 agent 的本地工具。
    LocalTool,
    /// ACP 后端下发的 permission 请求。
    AcpPermission,
    /// Public MCP 审批。
    PublicMcp,
}

impl DecisionAuthority {
    /// 来源标注的 i18n key。
    pub fn label_key(self) -> &'static str {
        match self {
            Self::LocalTool => "AgentUi.decision_authority_local",
            Self::AcpPermission => "AgentUi.decision_authority_acp",
            Self::PublicMcp => "AgentUi.decision_authority_public_mcp",
        }
    }

    /// 是否为 ACP 域（决定点下去派发哪一类 action）。
    pub fn is_acp(self) -> bool {
        matches!(self, Self::AcpPermission)
    }
}

/// 决策选项的语义分类。
///
/// 只看 provider 下发的 `kind` 前缀，**不猜**：ACP 的 `allow_once` / `allow_always`
/// 都归 `Allow`，`reject_once` / `reject_always` 都归 `Reject`；本地工具由固定两项给出。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecisionOptionKind {
    Allow,
    Reject,
}

/// 一个决策选项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecisionOption {
    /// 回传给后端的稳定 id（本地工具用固定常量，ACP 原样透传）。
    pub option_id: String,
    /// 展示名。
    pub name: String,
    pub kind: DecisionOptionKind,
}

impl DecisionOption {
    pub fn is_reject(&self) -> bool {
        self.kind == DecisionOptionKind::Reject
    }
}

/// 一个待决策项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingDecision {
    /// 稳定身份：本地工具与 Public MCP 用 `call_id`，ACP 用 `request_id`。
    pub id: String,
    pub source: DecisionCardSource,
    pub authority: DecisionAuthority,
    pub tool_name: String,
    /// 已脱敏的入参摘要（写入卡片时已经脱敏，这里不再加工）。
    pub summary: String,
    /// 已脱敏的入参 JSON；可能为空。
    pub input_json: String,
    /// 面向用户的提问 / 摘要文案。
    pub question: String,
    /// 选项列表。本地工具固定「拒绝 / 允许一次」，顺序即展示顺序。
    pub options: Vec<DecisionOption>,
}

impl PendingDecision {
    /// 默认聚焦的选项：优先「允许」，没有允许项时退化为第一项。
    pub fn primary_option(&self) -> Option<&DecisionOption> {
        self.options
            .iter()
            .find(|option| option.kind == DecisionOptionKind::Allow)
            .or_else(|| self.options.first())
    }

    /// 供标题展示的目标：优先脱敏摘要，其次工具名。
    pub fn target_label(&self) -> &str {
        if !self.summary.trim().is_empty() {
            &self.summary
        } else {
            &self.tool_name
        }
    }
}

/// 本地工具确认卡的固定选项 id。与内联卡片按钮一一对应。
pub const LOCAL_TOOL_ALLOW_OPTION: &str = "allow";
pub const LOCAL_TOOL_DENY_OPTION: &str = "deny";

/// 从消息流按出现顺序收集未决决策。
///
/// `is_public_mcp` 由调用方注入：只有视图知道某个 `call_id` 是否来自 Public MCP 审批。
/// 这样投影本身保持纯净，同时授权域标注依然准确。
pub fn pending_decisions(
    messages: &[ChatMessageUI],
    is_public_mcp: impl Fn(&str) -> bool,
) -> Vec<PendingDecision> {
    let mut out: Vec<PendingDecision> = Vec::new();

    for message in messages {
        match message.variant.card_kind() {
            Some(TOOL_CONFIRM_CARD) => {
                let Some(data) = ToolConfirmCardData::from_json(&message.content) else {
                    continue;
                };
                if data.status != "pending" {
                    continue;
                }
                if out.iter().any(|item| item.id == data.call_id) {
                    continue;
                }
                let authority = if is_public_mcp(&data.call_id) {
                    DecisionAuthority::PublicMcp
                } else {
                    DecisionAuthority::LocalTool
                };
                out.push(PendingDecision {
                    id: data.call_id.clone(),
                    source: DecisionCardSource::ToolConfirm,
                    authority,
                    tool_name: data.tool_name.clone(),
                    summary: data.input_summary.clone(),
                    input_json: data.input_json.clone(),
                    question: data.question.clone(),
                    options: local_tool_options(),
                });
            }
            Some(ACP_PERMISSION_CARD) => {
                let Some(data) = AcpPermissionCardData::from_json(&message.content) else {
                    continue;
                };
                if data.status != "pending" {
                    continue;
                }
                if out.iter().any(|item| item.id == data.request_id) {
                    continue;
                }
                out.push(PendingDecision {
                    id: data.request_id.clone(),
                    source: DecisionCardSource::AcpPermission,
                    authority: DecisionAuthority::AcpPermission,
                    tool_name: data.tool_name.clone(),
                    summary: data.summary.clone(),
                    input_json: data.details_json.clone(),
                    question: data.summary.clone(),
                    options: acp_options(&data),
                });
            }
            _ => {}
        }
    }

    out
}

/// 本地工具的固定选项：拒绝在前、允许在后（危险动作不做默认聚焦的诱因，
/// 展示顺序上先给「拒绝」）。
fn local_tool_options() -> Vec<DecisionOption> {
    vec![
        DecisionOption {
            option_id: LOCAL_TOOL_DENY_OPTION.to_string(),
            name: rust_i18n::t!("AgentUi.reject").to_string(),
            kind: DecisionOptionKind::Reject,
        },
        DecisionOption {
            option_id: LOCAL_TOOL_ALLOW_OPTION.to_string(),
            name: rust_i18n::t!("AgentUi.execute").to_string(),
            kind: DecisionOptionKind::Allow,
        },
    ]
}

/// ACP 选项原样透传 provider 下发的顺序与文案；只把 `kind` 归类。
///
/// provider 没给任何选项时**不虚构按钮**（宁可不显示，也不显示一个点了没用的控件）。
fn acp_options(data: &AcpPermissionCardData) -> Vec<DecisionOption> {
    data.options
        .iter()
        .map(|option| DecisionOption {
            option_id: option.option_id.clone(),
            name: option.name.clone(),
            kind: if option.kind.starts_with("reject") {
                DecisionOptionKind::Reject
            } else {
                DecisionOptionKind::Allow
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_cards::{AcpPermissionOptionData, TOOL_CONFIRM_CARD};
    use crate::{ChatMessageUI, MessageVariant};

    fn confirm_card(call_id: &str, status: &str) -> ChatMessageUI {
        ChatMessageUI::card(
            TOOL_CONFIRM_CARD,
            ToolConfirmCardData {
                call_id: call_id.to_string(),
                tool_name: "fs.write".into(),
                items: Vec::new(),
                input_summary: "navop/.env · 1 行变更".into(),
                input_json: r#"{"path":".env"}"#.into(),
                question: "需要确认：写入文件".into(),
                status: status.into(),
            }
            .to_json(),
        )
    }

    fn acp_card(request_id: &str, status: &str) -> ChatMessageUI {
        ChatMessageUI::card(
            ACP_PERMISSION_CARD,
            AcpPermissionCardData {
                request_id: request_id.to_string(),
                session_id: "s1".into(),
                tool_call_id: "c1".into(),
                tool_name: "terminal.exec".into(),
                summary: "执行 rm -rf build".into(),
                details_json: r#"{"command":"rm -rf build"}"#.into(),
                options: vec![
                    AcpPermissionOptionData {
                        option_id: "once".into(),
                        name: "允许一次".into(),
                        kind: "allow_once".into(),
                    },
                    AcpPermissionOptionData {
                        option_id: "never".into(),
                        name: "拒绝".into(),
                        kind: "reject_once".into(),
                    },
                ],
                status: status.into(),
                selected_option_name: String::new(),
            }
            .to_json(),
        )
    }

    fn never_public(_call_id: &str) -> bool {
        false
    }

    #[test]
    fn pending_cards_surface_in_message_order() {
        let messages = vec![
            confirm_card("call_a", "pending"),
            ChatMessageUI::assistant("中间的解释"),
            acp_card("req_b", "pending"),
        ];

        let decisions = pending_decisions(&messages, never_public);

        assert_eq!(2, decisions.len());
        assert_eq!("call_a", decisions[0].id);
        assert_eq!(DecisionCardSource::ToolConfirm, decisions[0].source);
        assert_eq!("req_b", decisions[1].id);
        assert_eq!(DecisionCardSource::AcpPermission, decisions[1].source);
    }

    #[test]
    fn resolved_cards_are_not_pending() {
        let messages = vec![
            confirm_card("call_a", "approved"),
            confirm_card("call_b", "rejected"),
            acp_card("req_c", "cancelled"),
        ];

        assert!(pending_decisions(&messages, never_public).is_empty());
    }

    #[test]
    fn a_repeated_card_for_the_same_call_appears_once() {
        // 卡片会被就地替换，但防御重复写入时不能出现两个决策。
        let messages = vec![
            confirm_card("call_a", "pending"),
            confirm_card("call_a", "pending"),
        ];

        assert_eq!(1, pending_decisions(&messages, never_public).len());
    }

    #[test]
    fn public_mcp_keeps_its_own_authority_and_never_merges_into_local_tool() {
        let messages = vec![
            confirm_card("call_local", "pending"),
            confirm_card("call_public", "pending"),
        ];

        let decisions = pending_decisions(&messages, |call_id| call_id == "call_public");

        assert_eq!(DecisionAuthority::LocalTool, decisions[0].authority);
        assert_eq!(DecisionAuthority::PublicMcp, decisions[1].authority);
        assert_ne!(
            decisions[0].authority.label_key(),
            decisions[1].authority.label_key()
        );
    }

    #[test]
    fn local_tool_offers_reject_then_allow_and_never_invents_session_scope() {
        let decisions = pending_decisions(&[confirm_card("call_a", "pending")], never_public);

        let options = &decisions[0].options;
        assert_eq!(2, options.len());
        assert_eq!(LOCAL_TOOL_DENY_OPTION, options[0].option_id);
        assert!(options[0].is_reject());
        assert_eq!(LOCAL_TOOL_ALLOW_OPTION, options[1].option_id);
        assert!(!options[1].is_reject());
        // 「本会话允许 / 始终允许」只在后端明确支持后才会出现。
        assert!(!options.iter().any(|option| option.option_id.contains("always")));
    }

    #[test]
    fn acp_options_keep_provider_order_and_only_classify_kind() {
        let decisions = pending_decisions(&[acp_card("req_b", "pending")], never_public);

        let options = &decisions[0].options;
        assert_eq!(vec!["once", "never"], option_ids(options));
        assert_eq!(DecisionOptionKind::Allow, options[0].kind);
        assert_eq!(DecisionOptionKind::Reject, options[1].kind);
        // 文案原样透传，不做本地化改写。
        assert_eq!("允许一次", options[0].name);
    }

    #[test]
    fn acp_card_without_provider_options_yields_no_buttons() {
        let mut card = acp_card("req_b", "pending");
        if let MessageVariant::Card { .. } = card.variant {
            let mut data = AcpPermissionCardData::from_json(&card.content).unwrap();
            data.options.clear();
            card.content = data.to_json();
        }

        let decisions = pending_decisions(&[card], never_public);

        assert_eq!(1, decisions.len());
        assert!(decisions[0].options.is_empty());
        assert!(decisions[0].primary_option().is_none());
    }

    #[test]
    fn primary_option_prefers_allow_over_reject() {
        let decisions = pending_decisions(&[acp_card("req_b", "pending")], never_public);

        let primary = decisions[0].primary_option().expect("primary option");
        assert_eq!("once", primary.option_id);
    }

    #[test]
    fn target_label_falls_back_to_tool_name_when_summary_is_blank() {
        let mut card = confirm_card("call_a", "pending");
        let mut data = ToolConfirmCardData::from_json(&card.content).unwrap();
        data.input_summary = "   ".into();
        card.content = data.to_json();

        let decisions = pending_decisions(&[card], never_public);

        assert_eq!("fs.write", decisions[0].target_label());
    }

    #[test]
    fn malformed_cards_are_skipped_instead_of_panicking() {
        let messages = vec![
            ChatMessageUI::card(TOOL_CONFIRM_CARD, "{ not json"),
            ChatMessageUI::card(ACP_PERMISSION_CARD, ""),
            confirm_card("call_a", "pending"),
        ];

        let decisions = pending_decisions(&messages, never_public);

        assert_eq!(1, decisions.len());
        assert_eq!("call_a", decisions[0].id);
    }

    fn option_ids(options: &[DecisionOption]) -> Vec<&str> {
        options
            .iter()
            .map(|option| option.option_id.as_str())
            .collect()
    }
}
