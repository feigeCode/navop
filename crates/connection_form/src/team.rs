use gpui::{App, AppContext, Context, Entity, SharedString, Window};
use gpui_component::select::{SelectItem, SelectState};
use one_core::cloud_sync::{
    GlobalCloudUser, TeamKeyCacheStatus, TeamKeyError, TeamOption, ensure_team_key_ready_for_save,
    get_cached_team_options,
};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event};
use rust_i18n::t;

#[derive(Clone, Default, PartialEq)]
pub struct TeamSelectItem {
    id: Option<String>,
    name: SharedString,
}

impl TeamSelectItem {
    pub fn personal() -> Self {
        Self {
            id: None,
            name: t!("TeamSync.personal").to_string().into(),
        }
    }

    pub fn from_team(team: &TeamOption) -> Self {
        let status = match team.key_status {
            TeamKeyCacheStatus::Missing
            | TeamKeyCacheStatus::VersionMismatch
            | TeamKeyCacheStatus::Invalid => {
                t!("TeamSync.key_missing_short")
            }
            TeamKeyCacheStatus::Cached => t!("TeamSync.key_cached_short"),
        };
        Self {
            id: Some(team.id.clone()),
            name: format!("{} ({status})", team.name).into(),
        }
    }

    pub fn team_id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn label(&self) -> &str {
        self.name.as_ref()
    }
}

impl SelectItem for TeamSelectItem {
    type Value = Option<String>;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

pub fn team_select_items(teams: &[TeamOption]) -> Vec<TeamSelectItem> {
    std::iter::once(TeamSelectItem::personal())
        .chain(teams.iter().map(TeamSelectItem::from_team))
        .collect()
}

pub fn create_team_select<T: 'static>(
    teams: &[TeamOption],
    selected_team_id: Option<&str>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<SelectState<Vec<TeamSelectItem>>> {
    let items = team_select_items(teams);
    let selected = selected_team_id.map(str::to_string);
    cx.new(|cx| {
        let mut state = SelectState::new(items, Some(Default::default()), window, cx);
        if selected.is_some() {
            state.set_selected_value(&selected, window, cx);
        }
        state
    })
}

pub fn replace_team_options<T: 'static>(
    select: &Entity<SelectState<Vec<TeamSelectItem>>>,
    teams: &[TeamOption],
    window: &mut Window,
    cx: &mut Context<T>,
) {
    let selected = selected_team_id(select, cx);
    let items = team_select_items(teams);
    select.update(cx, |state, cx| {
        state.set_items(items, window, cx);
        state.set_selected_value(&selected, window, cx);
    });
}

pub fn refresh_team_options<T: 'static>(
    select: &Entity<SelectState<Vec<TeamSelectItem>>>,
    window: &mut Window,
    cx: &mut Context<T>,
) {
    emit_connection_event(ConnectionDataEvent::CloudSyncRequested, cx);
    replace_team_options(select, &get_cached_team_options(cx), window, cx);
}

pub fn selected_team_id(
    select: &Entity<SelectState<Vec<TeamSelectItem>>>,
    cx: &App,
) -> Option<String> {
    select.read(cx).selected_value().cloned().flatten()
}

pub fn validate_selected_team(
    select: &Entity<SelectState<Vec<TeamSelectItem>>>,
    cx: &App,
) -> Result<Option<String>, TeamKeyError> {
    let team_id = selected_team_id(select, cx);
    ensure_team_key_ready_for_save(team_id.as_deref(), cx)?;
    Ok(team_id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TeamAssignment {
    New { current_user_id: Option<String> },
    Existing { owner_id: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppliedTeamAssignment {
    pub team_id: Option<String>,
    pub owner_id: Option<String>,
}

pub fn apply_team_assignment(
    team_id: Option<String>,
    assignment: TeamAssignment,
) -> AppliedTeamAssignment {
    let owner_id = match assignment {
        TeamAssignment::New { current_user_id } => current_user_id,
        TeamAssignment::Existing { owner_id } => owner_id,
    };
    AppliedTeamAssignment { team_id, owner_id }
}

pub fn resolve_team_assignment(
    team_id: Option<String>,
    is_editing: bool,
    existing_owner_id: Option<String>,
    cx: &App,
) -> Result<AppliedTeamAssignment, TeamKeyError> {
    ensure_team_key_ready_for_save(team_id.as_deref(), cx)?;
    let assignment = if is_editing {
        TeamAssignment::Existing {
            owner_id: existing_owner_id,
        }
    } else {
        TeamAssignment::New {
            current_user_id: GlobalCloudUser::get_user(cx).map(|user| user.id),
        }
    };
    Ok(apply_team_assignment(team_id, assignment))
}

pub fn team_label() -> String {
    t!("TeamSync.team_label").to_string()
}

pub fn refresh_teams_tooltip() -> String {
    t!("TeamSync.refresh_tooltip").to_string()
}

#[cfg(test)]
mod tests {
    use one_core::cloud_sync::{TeamKeyCacheStatus, TeamOption};

    use super::{TeamAssignment, TeamSelectItem, apply_team_assignment, team_select_items};

    fn team(name: &str, key_status: TeamKeyCacheStatus) -> TeamOption {
        TeamOption {
            id: format!("{name}-id"),
            name: name.to_string(),
            key_status,
            key_version: 1,
            key_verification: None,
            last_verified_at: None,
            role: None,
        }
    }

    #[test]
    fn team_options_start_with_personal_and_preserve_team_order() {
        let teams = vec![
            team("Alpha", TeamKeyCacheStatus::Missing),
            team("Beta", TeamKeyCacheStatus::Cached),
        ];

        let items = team_select_items(&teams);

        assert_eq!(None, items[0].team_id());
        assert_eq!(Some("Alpha-id"), items[1].team_id());
        assert_eq!(Some("Beta-id"), items[2].team_id());
    }

    #[test]
    fn team_option_label_describes_key_readiness() {
        let missing =
            TeamSelectItem::from_team(&team("Alpha", TeamKeyCacheStatus::VersionMismatch));
        let invalid = TeamSelectItem::from_team(&team("Gamma", TeamKeyCacheStatus::Invalid));
        let ready = TeamSelectItem::from_team(&team("Beta", TeamKeyCacheStatus::Cached));

        assert_eq!("Alpha (Needs key)", missing.label());
        assert_eq!("Gamma (Needs key)", invalid.label());
        assert_eq!("Beta (Key saved)", ready.label());
    }

    #[test]
    fn new_assignment_uses_current_user_as_owner() {
        let assignment = apply_team_assignment(
            Some("team-1".to_string()),
            TeamAssignment::New {
                current_user_id: Some("user-1".to_string()),
            },
        );

        assert_eq!(Some("team-1"), assignment.team_id.as_deref());
        assert_eq!(Some("user-1"), assignment.owner_id.as_deref());
    }

    #[test]
    fn edited_assignment_preserves_existing_owner() {
        let assignment = apply_team_assignment(
            None,
            TeamAssignment::Existing {
                owner_id: Some("owner-1".to_string()),
            },
        );

        assert_eq!(None, assignment.team_id);
        assert_eq!(Some("owner-1"), assignment.owner_id.as_deref());
    }
}
