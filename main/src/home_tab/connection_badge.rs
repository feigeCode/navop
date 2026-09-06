use super::*;

#[derive(Clone)]
pub(crate) struct ConnectionTeamBadge {
    pub(crate) name: String,
    pub(crate) tooltip: String,
    pub(crate) active: bool,
}

pub(crate) fn connection_team_badge(
    team_id: Option<&str>,
    teams: &[TeamOption],
) -> Option<ConnectionTeamBadge> {
    let team_id = team_id?;
    teams.iter().find(|team| team.id == team_id).map(|team| {
        let (status, active) = match team.membership_state {
            TeamMembershipState::Active => (None, true),
            TeamMembershipState::Departed => {
                (Some(t!("TeamSync.membership_departed").to_string()), false)
            }
            TeamMembershipState::Unknown => {
                (Some(t!("TeamSync.membership_unknown").to_string()), false)
            }
        };
        let tooltip = status
            .map(|status| format!("{} · {status}", team.name))
            .unwrap_or_else(|| team.name.clone());
        ConnectionTeamBadge {
            name: team.name.clone(),
            tooltip,
            active,
        }
    })
}

/// 中性团队标签（redesign §5.4）：统一 muted 底色，不再用 primary 蓝底白字；
/// 固定在名称行右端与名称对齐，超宽时标签先截断。
pub(super) fn render_team_badge(
    id_prefix: &str,
    conn: &StoredConnection,
    badge: ConnectionTeamBadge,
    cx: &App,
) -> AnyElement {
    let tooltip_text: SharedString = badge.tooltip.into();
    let foreground = if badge.active {
        cx.theme().foreground
    } else {
        cx.theme().muted_foreground
    };
    div()
        .id(SharedString::from(format!(
            "{id_prefix}-team-{}",
            conn.id.unwrap_or(0)
        )))
        .flex_shrink_0()
        .max_w(px(112.0))
        .px_1p5()
        .py_0p5()
        .rounded(px(4.0))
        .bg(cx.theme().muted)
        .text_color(foreground)
        .text_xs()
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .child(badge.name)
        .into_any_element()
}
