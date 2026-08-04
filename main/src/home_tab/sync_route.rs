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

pub(super) fn should_auto_onet_cloud_sync_for_settings(
    settings: &AppSettings,
    current_user_present: bool,
    has_master_key: bool,
) -> bool {
    settings.sync_enabled
        && sync_route_for_provider(settings.sync_provider) == HomeSyncRoute::OnetCloud
        && current_user_present
        && has_master_key
}

pub(super) fn should_auto_onet_cloud_sync(cx: &App, current_user_present: bool) -> bool {
    should_auto_onet_cloud_sync_for_settings(
        AppSettings::global(cx),
        current_user_present,
        crypto::has_master_key(),
    )
}

pub(super) fn should_show_team_key_menu_item(
    route: HomeSyncRoute,
    cached_team_count: usize,
) -> bool {
    route == HomeSyncRoute::OnetCloud && cached_team_count > 0
}

pub(crate) fn should_show_team_management_entry(team_management_enabled: bool) -> bool {
    team_management_enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn onet_cloud_auto_sync_requires_global_sync_to_be_enabled() {
        let mut settings = AppSettings::default();
        settings.sync_provider = SyncProvider::OnetCloud;
        settings.sync_enabled = false;

        assert!(!should_auto_onet_cloud_sync_for_settings(
            &settings, true, true
        ));

        settings.sync_enabled = true;
        assert!(should_auto_onet_cloud_sync_for_settings(
            &settings, true, true
        ));
    }

    #[test]
    fn onet_cloud_auto_sync_still_requires_provider_user_and_master_key() {
        let mut settings = AppSettings::default();
        settings.sync_enabled = true;
        settings.sync_provider = SyncProvider::Personal;

        assert!(!should_auto_onet_cloud_sync_for_settings(
            &settings, true, true
        ));

        settings.sync_provider = SyncProvider::OnetCloud;
        assert!(!should_auto_onet_cloud_sync_for_settings(
            &settings, false, true
        ));
        assert!(!should_auto_onet_cloud_sync_for_settings(
            &settings, true, false
        ));
    }
}
