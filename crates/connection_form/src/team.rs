use gpui::{App, AppContext, Context, Entity, SharedString, Window};
use gpui_component::select::{SelectItem, SelectState};
use one_core::cloud_sync::{
    GlobalCloudUser, TeamKeyCacheStatus, TeamKeyError, TeamOption, ensure_team_key_ready_for_save,
    get_cached_team_options,
};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event, get_notifier};
use one_core::license::{Feature, is_feature_enabled};
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

pub fn team_management_enabled(cx: &App) -> bool {
    is_feature_enabled(Feature::TeamManagement, cx)
}

pub fn create_team_select<T: 'static>(
    teams: &[TeamOption],
    selected_team_id: Option<&str>,
    window: &mut Window,
    cx: &mut Context<T>,
) -> Entity<SelectState<Vec<TeamSelectItem>>> {
    let items = team_select_items(teams);
    let selected = selected_team_id.map(str::to_string);
    let select = cx.new(|cx| {
        let mut state = SelectState::new(items, Some(Default::default()), window, cx);
        if selected.is_some() {
            state.set_selected_value(&selected, window, cx);
        }
        state
    });
    if let Some(notifier) = get_notifier(cx) {
        let select_for_update = select.clone();
        cx.subscribe_in(
            &notifier,
            window,
            move |_form, _, event: &ConnectionDataEvent, window, cx| {
                if matches!(event, ConnectionDataEvent::TeamCacheUpdated) {
                    replace_team_options(
                        &select_for_update,
                        &get_cached_team_options(cx),
                        window,
                        cx,
                    );
                    cx.notify();
                }
            },
        )
        .detach();
    }
    select
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
    _select: &Entity<SelectState<Vec<TeamSelectItem>>>,
    _window: &mut Window,
    cx: &mut Context<T>,
) {
    emit_connection_event(ConnectionDataEvent::CloudSyncRequested, cx);
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
    use gpui::{AppContext, Context, Entity, IntoElement, Render, TestAppContext, Window, div};
    use gpui_component::{Root, Theme, select::SelectState};
    use one_core::cloud_sync::{TeamKeyCacheStatus, TeamOption};
    use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};

    use super::{
        TeamAssignment, TeamSelectItem, apply_team_assignment, create_team_select,
        team_select_items,
    };

    struct TeamSelectTestRoot {
        select: Entity<SelectState<Vec<TeamSelectItem>>>,
    }

    impl Render for TeamSelectTestRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }

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

    #[test]
    fn refresh_waits_for_team_cache_updated_before_replacing_options() {
        let source = include_str!("team.rs");
        let refresh = source
            .split("pub fn refresh_team_options")
            .nth(1)
            .expect("refresh_team_options exists")
            .split("pub fn selected_team_id")
            .next()
            .expect("refresh_team_options has an end marker");
        let create = source
            .split("pub fn create_team_select")
            .nth(1)
            .expect("create_team_select exists")
            .split("pub fn replace_team_options")
            .next()
            .expect("create_team_select has an end marker");

        assert!(refresh.contains("ConnectionDataEvent::CloudSyncRequested"));
        assert!(!refresh.contains("replace_team_options("));
        assert!(!refresh.contains("get_cached_team_options("));
        assert!(create.contains("subscribe_in("));
        assert!(create.contains("ConnectionDataEvent::TeamCacheUpdated"));
        assert!(create.contains("get_cached_team_options(cx)"));
    }

    #[gpui::test]
    fn team_cache_updated_reloads_open_team_select(cx: &mut TestAppContext) {
        let initial_team = team("Alpha", TeamKeyCacheStatus::Missing);
        let (window, form) = cx.update(|cx| {
            cx.set_global(Theme::default());
            one_core::connection_notifier::init(cx);
            let mut form = None;
            let window = cx
                .open_window(Default::default(), |window, cx| {
                    let entity = cx.new(|cx| TeamSelectTestRoot {
                        select: create_team_select(
                            std::slice::from_ref(&initial_team),
                            Some(initial_team.id.as_str()),
                            window,
                            cx,
                        ),
                    });
                    form = Some(entity.clone());
                    cx.new(|cx| Root::new(entity, window, cx))
                })
                .expect("test window opens");
            (window, form.expect("test form created"))
        });

        cx.update(|cx| {
            let notifier = get_notifier(cx).expect("connection notifier initialized");
            notifier.update(cx, |_, cx| {
                cx.emit(ConnectionDataEvent::TeamCacheUpdated);
            });
        });

        cx.update(|cx| {
            window
                .update(cx, |_, _, cx| {
                    assert_eq!(None, form.read(cx).select.read(cx).selected_value());
                })
                .expect("test window updates");
        });
    }

    #[test]
    fn every_team_field_uses_the_shared_feature_gate() {
        for (name, source) in [
            (
                "database",
                include_str!("../../db_view/src/common/db_connection_form.rs"),
            ),
            (
                "mongodb",
                include_str!("../../mongodb_view/src/mongo_form_window.rs"),
            ),
            (
                "redis",
                include_str!("../../redis_view/src/redis_form_window.rs"),
            ),
            (
                "ssh",
                include_str!("../../terminal_view/src/ssh_form_window.rs"),
            ),
            (
                "serial",
                include_str!("../../terminal_view/src/serial_form_window.rs"),
            ),
            (
                "port forwarding",
                include_str!("../../port_forwarding_view/src/view.rs"),
            ),
            (
                "remote desktop",
                include_str!("../../remote_desktop_view/src/remote_desktop_form/view.rs"),
            ),
        ] {
            assert!(
                source.contains("team_management_enabled(cx)"),
                "{name} team field must use the shared feature gate"
            );
        }
    }
}
