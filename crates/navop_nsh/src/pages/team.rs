use nsh_base::theme;
use nsh_core::prelude::*;
use one_core::cloud_sync::{TeamKeyCacheStatus, TeamOption};
use one_core::storage::TeamMembershipState;

use crate::{services::NavopNshServices, ui};

pub(crate) fn team() -> impl IntoElement {
    let teams = NavopNshServices::current()
        .map(|services| services.teams.clone())
        .unwrap_or_default();
    let count = teams.len();

    view()
        .size_full()
        .child(ui::page_header(
            "Team",
            format!("{count} team membership(s) available in the shared cache"),
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .p_6()
                    .gap_3()
                    .child(ui::card().child(
                        text(
                            "Team data is read from the existing cloud-sync cache. Account, member \
                             administration and key rotation will use the same host service APIs in \
                             the next migration slice.",
                        )
                        .text_sm()
                        .text_color(move || theme().muted_foreground),
                    ))
                    .when(teams.is_empty(), |page| {
                        page.child(ui::card().child(
                            text("No cached teams. Sign in or refresh team data in the current desktop app.")
                                .text_sm()
                                .text_color(move || theme().muted_foreground),
                        ))
                    })
                    .children(teams.into_iter().map(team_row)),
            ),
        )
}

fn team_row(team: TeamOption) -> impl IntoElement {
    let role = team.role.unwrap_or_else(|| "member".to_owned());
    let membership = membership_label(team.membership_state);
    let key_status = key_status_label(&team.key_status);
    ui::card()
        .flex_row()
        .items_center()
        .gap_4()
        .child(
            view()
                .size(px(40.0))
                .flex_none()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(move || theme().accent)
                .child(
                    text(team.name.chars().next().unwrap_or('T').to_string())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                ),
        )
        .child(
            view()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    text(team.name)
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().foreground),
                )
                .child(
                    text(format!("{} · {membership} · {key_status}", team.id))
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            text(format!("{role} · key v{}", team.key_version))
                .text_xs()
                .text_color(move || theme().muted_foreground),
        )
}

fn membership_label(state: TeamMembershipState) -> &'static str {
    match state {
        TeamMembershipState::Active => "active",
        TeamMembershipState::Departed => "departed",
        TeamMembershipState::Unknown => "unknown",
    }
}

fn key_status_label(status: &TeamKeyCacheStatus) -> &'static str {
    match status {
        TeamKeyCacheStatus::Missing => "key missing",
        TeamKeyCacheStatus::Cached => "key cached",
        TeamKeyCacheStatus::VersionMismatch => "key version mismatch",
        TeamKeyCacheStatus::Invalid => "key invalid",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_status_labels_are_stable() {
        assert_eq!(membership_label(TeamMembershipState::Active), "active");
        assert_eq!(key_status_label(&TeamKeyCacheStatus::Cached), "key cached");
    }
}
