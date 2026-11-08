//! 团队管理错误文案
//!
//! 团队管理接口（RPC `create_team_with_owner` / `add_team_member_by_email` /
//! `update_team_member_role` / `remove_team_member` 等）在权限不足或参数不合法
//! 时用 `RAISE EXCEPTION` 抛出英文原文，UI 需要把它们映射成人话。

use crate::cloud_sync::client::CloudApiError;

/// 登录态失效的统一文案
const NOT_LOGGED_IN_MESSAGE: &str = "登录状态已失效，请重新登录";

/// 服务端抛出的原文 → 中文文案
///
/// 原文来自线上函数体（`RAISE EXCEPTION '<原文>'`），必须逐字匹配；
/// 历史措辞（如 "website console"）也一并映射成桌面端能读懂的表述。
const TEAM_ADMIN_ERROR_MESSAGES: &[(&str, &str)] = &[
    (
        "User does not exist. Ask them to sign in first.",
        "该邮箱还没有注册账号，请让对方先在 Navop 登录一次",
    ),
    ("Only owners can add owners", "只有团队所有者可以添加所有者"),
    (
        "Only owners can manage owners",
        "只有团队所有者可以管理所有者",
    ),
    (
        "Only owners can assign owners",
        "只有团队所有者可以指派所有者",
    ),
    (
        "Only owners can remove admins",
        "只有团队所有者可以移除管理员",
    ),
    (
        "Owners cannot be removed from the website console",
        "不能直接移除团队所有者，请先把所有者降级",
    ),
    (
        "Owners cannot change their own role",
        "拥有者不能移除自己的拥有者角色，请先转让所有权",
    ),
    ("Insufficient team permissions", "当前角色没有该操作权限"),
    ("Team membership required", "你不是该团队成员"),
    ("Authentication required", NOT_LOGGED_IN_MESSAGE),
    ("Invalid team role", "角色不合法"),
    ("Email is required", "邮箱不能为空"),
];

/// 把团队管理接口的错误转换成用户可读的中文文案
///
/// 命中映射表时返回中文；未命中时返回服务端原文（`CloudApiError::Conflict`
/// 等变体里已经是中文，直接透出）。
pub fn team_admin_error_message(error: &CloudApiError) -> String {
    if matches!(error, CloudApiError::NotAuthenticated) {
        return NOT_LOGGED_IN_MESSAGE.to_string();
    }

    let raw = error_message(error);
    let message = raw.trim();
    if message.is_empty() {
        return error.to_string();
    }

    // 先按整串匹配，避免自由文本里恰好带出关键词时误判
    if let Some((_, translated)) = TEAM_ADMIN_ERROR_MESSAGES
        .iter()
        .find(|(raw_message, _)| message.eq_ignore_ascii_case(raw_message))
    {
        return (*translated).to_string();
    }

    // 兼容被上层包装过的文案，如 "添加团队成员失败: Only owners can add owners"
    let lowered = message.to_lowercase();
    if let Some((_, translated)) = TEAM_ADMIN_ERROR_MESSAGES.iter().find(|(raw_message, _)| {
        let needle = raw_message.to_lowercase();
        lowered.contains(&needle)
    }) {
        return (*translated).to_string();
    }

    message.to_string()
}

/// 取出错误里携带的服务端原文
fn error_message(error: &CloudApiError) -> String {
    match error {
        CloudApiError::NotAuthenticated => String::new(),
        CloudApiError::AuthenticationFailed(message)
        | CloudApiError::EmailConfirmationRequired(message)
        | CloudApiError::NetworkError(message)
        | CloudApiError::ServerError(message)
        | CloudApiError::ParseError(message)
        | CloudApiError::NotFound(message)
        | CloudApiError::Conflict(message)
        | CloudApiError::Unknown(message) => message.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn translated(raw: &str) -> String {
        team_admin_error_message(&CloudApiError::ServerError(raw.to_string()))
    }

    #[test]
    fn maps_every_server_side_message() {
        for (raw, expected) in TEAM_ADMIN_ERROR_MESSAGES {
            assert_eq!(*expected, translated(raw), "原文 {} 未正确映射", raw);
        }
    }

    #[test]
    fn maps_owner_permission_messages() {
        assert_eq!(
            "只有团队所有者可以管理所有者",
            translated("Only owners can manage owners")
        );
        assert_eq!(
            "只有团队所有者可以指派所有者",
            translated("Only owners can assign owners")
        );
        assert_eq!(
            "只有团队所有者可以移除管理员",
            translated("Only owners can remove admins")
        );
    }

    #[test]
    fn maps_owner_removal_with_legacy_website_wording() {
        assert_eq!(
            "不能直接移除团队所有者，请先把所有者降级",
            translated("Owners cannot be removed from the website console")
        );
    }

    #[test]
    fn maps_wrapped_server_message() {
        assert_eq!(
            "只有团队所有者可以添加所有者",
            translated("添加团队成员失败: Only owners can add owners")
        );
    }

    #[test]
    fn maps_not_authenticated_variant() {
        assert_eq!(
            NOT_LOGGED_IN_MESSAGE,
            team_admin_error_message(&CloudApiError::NotAuthenticated)
        );
    }

    #[test]
    fn keeps_conflict_message_untouched() {
        let error = CloudApiError::Conflict("当前角色没有移除成员的权限".to_string());
        assert_eq!(
            "当前角色没有移除成员的权限",
            team_admin_error_message(&error)
        );
    }

    #[test]
    fn keeps_unknown_server_message() {
        assert_eq!(
            "some unexpected failure",
            translated("some unexpected failure")
        );
    }

    #[test]
    fn falls_back_to_display_when_message_is_empty() {
        let error = CloudApiError::ServerError("   ".to_string());
        assert_eq!(error.to_string(), team_admin_error_message(&error));
    }
}
