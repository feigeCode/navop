use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HomeSyncRoute {
    OnetCloud,
    Personal,
}

pub(super) fn refreshed_pending_conflicts(
    previous: Vec<SyncConflict>,
    current: Vec<SyncConflict>,
    errors: &[String],
) -> Vec<SyncConflict> {
    if errors.is_empty() || !current.is_empty() {
        current
    } else {
        previous
    }
}

pub(super) fn sync_route_for_provider(provider: SyncProvider) -> HomeSyncRoute {
    match provider {
        SyncProvider::OnetCloud => HomeSyncRoute::OnetCloud,
        SyncProvider::Personal => HomeSyncRoute::Personal,
    }
}

pub(super) fn sync_route(cx: &App) -> HomeSyncRoute {
    sync_route_for_provider(AppSettings::global(cx).sync_provider)
}

pub(super) fn should_auto_onet_cloud_sync(cx: &App, current_user_present: bool) -> bool {
    sync_route(cx) == HomeSyncRoute::OnetCloud && current_user_present && crypto::has_master_key()
}

pub(super) fn should_show_team_key_menu_item(
    route: HomeSyncRoute,
    cached_team_count: usize,
) -> bool {
    route == HomeSyncRoute::OnetCloud && cached_team_count > 0
}

pub(super) fn should_show_team_management_entry(team_management_enabled: bool) -> bool {
    team_management_enabled
}
