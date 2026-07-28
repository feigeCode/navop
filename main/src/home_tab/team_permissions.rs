use one_core::cloud_sync::{TeamOption, can_edit_connection_with_cached_teams};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RestoreFailureResolution {
    Unchanged,
    IdentityChanged,
    Stale,
}

#[derive(Debug, Default)]
pub(crate) struct TeamPermissionSnapshot {
    user_id: Option<String>,
    teams: Vec<TeamOption>,
}

impl TeamPermissionSnapshot {
    pub(crate) fn from_persisted_user_id(user_id: Option<String>) -> Self {
        Self {
            user_id,
            teams: Vec::new(),
        }
    }

    pub(crate) fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }

    pub(crate) fn teams(&self) -> &[TeamOption] {
        &self.teams
    }

    pub(crate) fn set_user_id(&mut self, user_id: Option<String>) {
        if self.user_id == user_id {
            return;
        }
        self.user_id = user_id;
        self.teams.clear();
    }

    pub(crate) fn clear(&mut self) {
        self.user_id = None;
        self.teams.clear();
    }

    pub(crate) fn replace_teams_for(
        &mut self,
        requested_user_id: &str,
        teams: Vec<TeamOption>,
    ) -> bool {
        if self.user_id() != Some(requested_user_id) {
            return false;
        }
        self.teams = teams;
        true
    }

    pub(crate) fn reconcile_restore_failure(
        &mut self,
        requested_user_id: &str,
        persisted_user_id: Option<String>,
    ) -> RestoreFailureResolution {
        if self.user_id() != Some(requested_user_id) {
            return RestoreFailureResolution::Stale;
        }
        if self.user_id == persisted_user_id {
            return RestoreFailureResolution::Unchanged;
        }
        self.set_user_id(persisted_user_id);
        RestoreFailureResolution::IdentityChanged
    }

    pub(crate) fn can_edit_connection(&self, team_id: Option<&str>) -> bool {
        can_edit_connection_with_cached_teams(team_id, &self.teams, self.user_id.is_some())
    }
}

#[cfg(test)]
mod tests {
    use one_core::cloud_sync::{TeamKeyCacheStatus, TeamOption};
    use one_core::storage::TeamMembershipState;

    use super::{RestoreFailureResolution, TeamPermissionSnapshot};

    fn team(id: &str, role: &str, membership_state: TeamMembershipState) -> TeamOption {
        TeamOption {
            id: id.to_string(),
            name: id.to_string(),
            key_status: TeamKeyCacheStatus::Missing,
            key_version: 0,
            key_verification: None,
            last_verified_at: None,
            role: Some(role.to_string()),
            membership_state,
        }
    }

    #[test]
    fn persisted_identity_drives_cached_permissions_without_an_online_user() {
        let mut snapshot =
            TeamPermissionSnapshot::from_persisted_user_id(Some("user-1".to_string()));
        assert!(snapshot.replace_teams_for(
            "user-1",
            vec![
                team("owner-team", "owner", TeamMembershipState::Active),
                team("admin-team", "admin", TeamMembershipState::Active),
                team("member-team", "member", TeamMembershipState::Active),
                team("departed-team", "owner", TeamMembershipState::Departed),
                team("unknown-team", "admin", TeamMembershipState::Unknown),
            ],
        ));

        assert!(snapshot.can_edit_connection(None));
        assert!(snapshot.can_edit_connection(Some("owner-team")));
        assert!(snapshot.can_edit_connection(Some("admin-team")));
        assert!(!snapshot.can_edit_connection(Some("member-team")));
        assert!(!snapshot.can_edit_connection(Some("departed-team")));
        assert!(!snapshot.can_edit_connection(Some("unknown-team")));
        assert!(!snapshot.can_edit_connection(Some("missing-team")));
    }

    #[test]
    fn restore_failure_preserves_or_revokes_permissions_with_persisted_auth() {
        let mut snapshot =
            TeamPermissionSnapshot::from_persisted_user_id(Some("user-1".to_string()));
        snapshot.replace_teams_for(
            "user-1",
            vec![team("team-1", "owner", TeamMembershipState::Active)],
        );

        assert_eq!(
            RestoreFailureResolution::Unchanged,
            snapshot.reconcile_restore_failure("user-1", Some("user-1".to_string())),
        );
        assert!(snapshot.can_edit_connection(Some("team-1")));

        assert_eq!(
            RestoreFailureResolution::IdentityChanged,
            snapshot.reconcile_restore_failure("user-1", None),
        );
        assert_eq!(None, snapshot.user_id());
        assert!(snapshot.teams().is_empty());
        assert!(!snapshot.can_edit_connection(Some("team-1")));
    }

    #[test]
    fn stale_restore_and_team_load_results_cannot_overwrite_a_new_user() {
        let mut snapshot =
            TeamPermissionSnapshot::from_persisted_user_id(Some("user-1".to_string()));
        snapshot.set_user_id(Some("user-2".to_string()));

        assert!(!snapshot.replace_teams_for(
            "user-1",
            vec![team("old-team", "owner", TeamMembershipState::Active)],
        ));
        assert_eq!(
            RestoreFailureResolution::Stale,
            snapshot.reconcile_restore_failure("user-1", None),
        );
        assert_eq!(Some("user-2"), snapshot.user_id());
        assert!(snapshot.teams().is_empty());
    }
}
