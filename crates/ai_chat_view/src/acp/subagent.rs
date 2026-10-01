//! 子代理（`task` 工具）子会话地址的提取。
//!
//! 外部 agent（实测 OpenCode 1.18.30）把子代理跑在一条**独立的子会话**里，工具调用
//! 的终态输出才带上这条子会话的地址：
//!
//! - 完成：`{"output": "<task id=\"ses_X\" state=\"completed\">…",
//!   "metadata": {"parentSessionId": "…", "sessionId": "ses_X", …}}`
//! - 失败：`{"error": "Task cancelled", "metadata": {"parentSessionId": "…",
//!   "sessionId": "ses_X", …}}`
//!
//! 运行态（ACP 的 `in_progress`）**不带** `rawOutput`，所以这条地址只在子代理结束时
//! 才可得。这是「子代理详情只能在跑完之后看」的协议级原因，不是实现偷懒。
//!
//! 拿到地址之后，客户端可以用 `session/load` 把子会话的整段历史（含 `reasoning`
//! 推理块）拉回来——这是本模块存在的前提。

use agent_runtime::{SessionId, TurnId};
use serde_json::Value;

/// 一次子代理调用的子会话地址。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SubagentLink {
    /// 子代理子会话的协议 id；`session/load` 的目标。
    pub session_id: String,
    /// 父会话协议 id（诊断用；缺席不影响使用）。
    pub parent_session_id: Option<String>,
}

/// 详情会话 id 的前缀。改它等于让已经开着的详情面板认不出自己的转录。
pub const DETAIL_SESSION_PREFIX: &str = "acp-sub:";

/// 详情会话的合成轮次前缀。
///
/// 与 [`DETAIL_SESSION_PREFIX`] 分开命名，是为了让日志与转录里一眼能分出
/// 「这一轮是子代理的详情回放」和「这一轮是主会话真跑出来的」。
pub const DETAIL_TURN_PREFIX: &str = "acp-sub-turn:";

/// 由子会话的**协议 id** 构造详情会话的内置 id。
pub fn detail_session_id_for(acp_session_id: &str) -> String {
    format!("{DETAIL_SESSION_PREFIX}{acp_session_id}")
}

/// 由子会话的协议 id 构造详情回放的合成轮次 id。
///
/// 回放不属于任何一轮：它没有 prompt、没有终态。给一个稳定（同一条子会话恒等）的
/// 合成轮次，翻译层才能把事件造出来，视图层也才能把它和主会话的轮次区分开。
pub fn detail_turn_id_for(acp_session_id: &str) -> TurnId {
    TurnId::from_string(format!("{DETAIL_TURN_PREFIX}{acp_session_id}"))
}

/// 由子会话的协议 id 构造详情会话的 `SessionId`。
pub fn detail_session_uid_for(acp_session_id: &str) -> SessionId {
    SessionId::from_string(detail_session_id_for(acp_session_id))
}

/// 这条内置会话 id 是不是子代理详情会话。
///
/// 详情会话的事件**不属于**当前对话：视图必须据此把它们分流出去，否则一次
/// `session/load` 回放会把整段子代理推理灌进主转录。
///
/// 光有前缀不算：`DETAIL_SESSION_PREFIX` 本身没有指向任何子会话，认了它等于凭空
/// 造出一条空详情会话。与 [`acp_session_id_from_detail`] 保持同一判据。
pub fn is_detail_session_id(session_id: &str) -> bool {
    acp_session_id_from_detail(session_id).is_some()
}

/// 从详情会话 id 还原子会话的协议 id；不是详情会话就返回 `None`。
pub fn acp_session_id_from_detail(detail_session_id: &str) -> Option<&str> {
    detail_session_id
        .strip_prefix(DETAIL_SESSION_PREFIX)
        .filter(|rest| !rest.is_empty())
}

/// 从工具观测文本里抽子会话地址；不是子代理就返回 `None`。
///
/// 三级回退，按可靠程度排序：
///
/// 1. `metadata.sessionId` + `metadata.parentSessionId` —— agent 给的结构化字段，
///    两个字段必须**同时**在场（见下）；
/// 2. 输出正文里的 `<task id="ses_X" …>` —— agent 渲染给模型看的标签，metadata
///    缺席时（取消 / 失败）还在；
/// 3. 都没有就放弃。宁可不给入口，也不要猜一个 id 去 `session/load`——load 一条
///    不属于子代理的会话会把主会话历史灌进详情面板。
///
/// # 为什么要求 `parentSessionId` 同时在场
///
/// 只有 `sessionId` 的工具元数据太常见（`bash` / `read` 之类都可能带），认了它
/// 就会给普通工具挂上一个永远点不出东西的「查看推理过程」。
pub fn subagent_link_from_observation(text: &str) -> Option<SubagentLink> {
    if let Ok(value) = serde_json::from_str::<Value>(text)
        && let Some(link) = link_from_structured_output(&value)
    {
        return Some(link);
    }
    link_from_task_tag(text)
}

/// 结构化输出里的子代理标记（`metadata` 优先，`output` 正文兜底）。
fn link_from_structured_output(value: &Value) -> Option<SubagentLink> {
    let object = value.as_object()?;
    if let Some(link) = link_from_metadata(object.get("metadata")) {
        return Some(link);
    }
    object
        .get("output")
        .and_then(Value::as_str)
        .and_then(link_from_task_tag)
}

/// `metadata.parentSessionId` + `metadata.sessionId` 同在才算子代理。
fn link_from_metadata(metadata: Option<&Value>) -> Option<SubagentLink> {
    let metadata = metadata?.as_object()?;
    let session_id = metadata.get("sessionId")?.as_str()?;
    let parent_session_id = metadata.get("parentSessionId")?.as_str()?;
    if session_id.is_empty() {
        return None;
    }
    Some(SubagentLink {
        session_id: session_id.to_string(),
        parent_session_id: Some(parent_session_id.to_string()),
    })
}

/// 正文里的 `<task id="ses_X" …>` 标签。
///
/// **两种引号形态都要认**：观测文本是 `rawOutput` 的 pretty JSON（见
/// `translate::tool_text`），所以正常解析成功时 `output` 里的引号已经被
/// `serde_json` 反转义成 `"`；而截断之后 JSON 解析失败、只能退回原始文本匹配，
/// 那时引号仍是转义形态 `\"`。只认一种，就会在「输出很长、注定被截断」的子代理上
/// 恰好失效——而子代理的输出几乎总是很长。
fn link_from_task_tag(text: &str) -> Option<SubagentLink> {
    let rest = text.split_once("<task id=")?.1;
    // 只吃掉「一个可选的转义符 + 一个可选的开引号」，绝不能用 trim 一次性剔除引号：
    // 空 id 的两个引号紧挨着，贪心剔除会把闭引号也吃掉，于是把后面的 `state=` 读成 id。
    let rest = rest.trim_start_matches([' ', '\t']);
    let rest = rest.strip_prefix('\\').unwrap_or(rest);
    let rest = rest.strip_prefix('"').unwrap_or(rest);
    // 会话 id 是 `ses_` + 字母数字，遇到分隔符即结束。
    let end = rest
        .find(|c: char| {
            matches!(
                c,
                '"' | '\\' | ' ' | '\t' | '\n' | '\r' | '>' | '<' | '/' | '}'
            )
        })
        .unwrap_or(rest.len());
    let session_id = rest[..end].trim();
    if session_id.is_empty() {
        return None;
    }
    Some(SubagentLink {
        session_id: session_id.to_string(),
        parent_session_id: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_task_output_yields_the_child_session() {
        // OpenCode `completedToolRawOutput` 的真实形状。
        let text = serde_json::json!({
            "output": "<task id=\"ses_fbccd3f66ffeKycb0SRdX5ee7K\" state=\"completed\">\n\
                       <task_result>\n**Findings**\n- …\n</task_result>\n</task>",
            "metadata": {
                "parentSessionId": "ses_fbcdcce6effeShGH4H4w6S1gvJ",
                "sessionId": "ses_fbccd3f66ffeKycb0SRdX5ee7K",
                "model": { "modelID": "gpt-5.6-sol", "providerID": "custom" },
                "truncated": false
            }
        })
        .to_string();

        let link = subagent_link_from_observation(&text).expect("子代理地址");

        assert_eq!(link.session_id, "ses_fbccd3f66ffeKycb0SRdX5ee7K");
        assert_eq!(
            link.parent_session_id.as_deref(),
            Some("ses_fbcdcce6effeShGH4H4w6S1gvJ")
        );
    }

    #[test]
    fn failed_task_output_yields_the_child_session() {
        // 截图里那种被取消的形态：`errorToolUpdate` 的 `{error, metadata}`。
        let text = serde_json::json!({
            "error": "Task cancelled",
            "metadata": {
                "parentSessionId": "ses_f100c4795ffeHpFUlz0CuHtR9V",
                "sessionId": "ses_f1008f4e3ffeW6TIL7MPPe7sP7",
                "model": { "providerID": "opencode-acp", "modelID": "9router/deepseek-v4-flash" }
            }
        })
        .to_string();

        let link = subagent_link_from_observation(&text).expect("子代理地址");

        assert_eq!(link.session_id, "ses_f1008f4e3ffeW6TIL7MPPe7sP7");
        assert_eq!(
            link.parent_session_id.as_deref(),
            Some("ses_f100c4795ffeHpFUlz0CuHtR9V")
        );
    }

    #[test]
    fn metadata_less_output_falls_back_to_the_task_tag() {
        // metadata 缺席时，正文标签还在（取消 / 老版本 agent）。
        let text = "<task id=\"ses_abc123\" state=\"error\">\nboom\n</task>";
        let link = subagent_link_from_observation(text).expect("子代理地址");
        assert_eq!(link.session_id, "ses_abc123");
        assert_eq!(link.parent_session_id, None);
    }

    #[test]
    fn a_session_id_without_a_parent_is_not_a_subagent() {
        // 只有 `sessionId` 的工具元数据太常见；认了它就会给普通工具挂死入口。
        let text = serde_json::json!({
            "output": "done",
            "metadata": { "sessionId": "ses_whatever", "exit_code": 0 }
        })
        .to_string();
        assert_eq!(subagent_link_from_observation(&text), None);
    }

    #[test]
    fn a_parent_without_a_session_id_is_not_a_subagent() {
        let text = serde_json::json!({
            "metadata": { "parentSessionId": "ses_parent" }
        })
        .to_string();
        assert_eq!(subagent_link_from_observation(&text), None);
    }

    #[test]
    fn ordinary_tool_output_yields_nothing() {
        assert_eq!(subagent_link_from_observation("hello world"), None);
        assert_eq!(subagent_link_from_observation(""), None);
        assert_eq!(
            subagent_link_from_observation(&serde_json::json!({ "output": "42" }).to_string()),
            None
        );
        assert_eq!(
            subagent_link_from_observation(&serde_json::json!(["a", "b"]).to_string()),
            None
        );
    }

    #[test]
    fn an_empty_task_id_is_rejected() {
        // 宁可没有入口，也不要拿空 id 去 load。
        assert_eq!(
            subagent_link_from_observation("<task id=\"\" state=\"completed\">"),
            None
        );
        assert_eq!(
            subagent_link_from_observation("<task id=\\\"\\\" state=\\\"completed\\\">"),
            None
        );
    }

    #[test]
    fn a_truncated_pretty_json_still_yields_the_task_id() {
        // 真实形态：观测文本是 rawOutput 的 pretty JSON，长输出被截断后 JSON 不再可解析，
        // 只能退回原文匹配 —— 此时引号仍是 JSON 转义形态 `\"`。
        let text = "{\n  \"output\": \"<task id=\\\"ses_fbccd3f66ffeKycb0SRdX5ee7K\\\" state=\\\"completed\\\">\\n<task_result>\\n…（已截断）";
        let link = subagent_link_from_observation(text).expect("截断后仍要认出子会话");
        assert_eq!(link.session_id, "ses_fbccd3f66ffeKycb0SRdX5ee7K");
    }

    #[test]
    fn a_failed_task_whose_summary_is_pretty_json_yields_the_task_id() {
        // 失败观测的 `summary`/`data` 都是 `{error, metadata}` 的同一份文本，
        // 两条路都要能认出来。
        let text = "{\n  \"error\": \"Task cancelled\",\n  \"metadata\": {\n    \"parentSessionId\": \"ses_p\",\n    \"sessionId\": \"ses_c\"\n  }\n}";
        let link = subagent_link_from_observation(text).expect("子代理地址");
        assert_eq!(link.session_id, "ses_c");
    }

    #[test]
    fn the_detail_session_id_is_namespaced_away_from_real_sessions() {
        let detail = detail_session_id_for("ses_abc");
        assert_eq!(detail, "acp-sub:ses_abc");
        // 真实 ACP 会话是 `acp:<uuid>`；两者不能撞 —— `acp-sub:` 的第 4 个字符
        // 不是 `:`，所以不会被 `acp:` 前缀吃掉。
        assert!(!detail.starts_with("acp:"));
        assert!(detail.starts_with(DETAIL_SESSION_PREFIX));
    }

    #[test]
    fn detail_session_ids_round_trip_and_stay_out_of_the_real_namespace() {
        let uid = detail_session_id_for("ses_child");
        assert!(is_detail_session_id(&uid));
        assert_eq!(acp_session_id_from_detail(&uid), Some("ses_child"));
        assert_eq!(
            detail_session_uid_for("ses_child").to_string(),
            uid,
            "事件上的会话 id 必须与详情面板认的 id 一致"
        );
        assert_eq!(
            detail_turn_id_for("ses_child").to_string(),
            "acp-sub-turn:ses_child"
        );

        // 真实会话与空后缀都不算详情会话。
        assert!(!is_detail_session_id("acp:3f1a"));
        assert!(!is_detail_session_id(DETAIL_SESSION_PREFIX));
        assert_eq!(acp_session_id_from_detail(DETAIL_SESSION_PREFIX), None);
    }

    #[test]
    fn the_detail_prefix_does_not_collide_with_a_real_acp_session() {
        // 真实会话 id 形如 `acp:<uuid>`；`acp-sub:` 不能被 `acp:` 前缀吃掉。
        let uid = detail_session_id_for("ses_x");
        assert!(
            !uid.starts_with("acp:"),
            "详情会话不能落进真实会话的命名空间"
        );
    }
}
