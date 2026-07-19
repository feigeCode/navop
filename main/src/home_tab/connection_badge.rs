use super::*;

#[derive(Clone)]
pub(super) struct ConnectionTeamBadge {
    pub(super) name: String,
    pub(super) tooltip: String,
    pub(super) active: bool,
}

pub(super) fn connection_team_badge(
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
