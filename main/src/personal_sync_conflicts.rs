use gpui::{App, SharedString, Window};
use gpui_component::WindowExt;
use one_core::cloud_sync::ConflictResolution;
use one_core::cloud_sync::personal::{PersonalConflictType, PersonalSyncConflict};
use rust_i18n::t;

use crate::personal_sync_runtime::{PersonalSyncConflictDisplayInfo, PersonalSyncRecordDisplay};
use crate::sync_conflict_dialog::{
    SyncConflictDialogItem, SyncConflictResolutionOption, show_sync_conflict_dialog,
};

pub(crate) fn current_personal_conflict_count(cx: &App) -> usize {
    crate::personal_sync_runtime::list_personal_conflicts(cx)
        .map(|conflicts| personal_conflict_count(&conflicts))
        .unwrap_or(0)
}

pub(crate) fn show_personal_conflict_dialog(window: &mut Window, cx: &mut App) {
    crate::personal_sync_runtime::refresh_personal_sync_identity(cx);
    let conflicts = match crate::personal_sync_runtime::list_personal_conflicts(cx) {
        Ok(conflicts) => conflicts,
        Err(error) => {
            window.push_notification(error.to_string(), cx);
            return;
        }
    };
    if conflicts.is_empty() {
        return;
    }

    let count = conflicts.len();
    let items = conflicts
        .iter()
        .map(|conflict| {
            let display =
                crate::personal_sync_runtime::personal_conflict_display_info(conflict, cx);
            personal_conflict_dialog_item(conflict, &display)
        })
        .collect();
    show_sync_conflict_dialog(
        window,
        cx,
        t!("Home.personal_sync_conflict_dialog_title", count = count).to_string(),
        t!("Home.personal_sync_conflict_apply").to_string(),
        items,
        |selected, _window, cx| {
            crate::personal_sync_runtime::resolve_personal_conflicts(selected, cx);
        },
    );
}

pub(crate) fn personal_conflict_count(conflicts: &[PersonalSyncConflict]) -> usize {
    conflicts.len()
}

pub(crate) fn default_personal_conflict_strategy(
    conflict_type: PersonalConflictType,
) -> ConflictResolution {
    match conflict_type {
        PersonalConflictType::BothModified | PersonalConflictType::LocalDeletedRemoteModified => {
            ConflictResolution::UseCloud
        }
        PersonalConflictType::LocalModifiedRemoteDeleted => ConflictResolution::UseLocal,
    }
}

fn personal_conflict_dialog_item(
    conflict: &PersonalSyncConflict,
    display: &PersonalSyncConflictDisplayInfo,
) -> SyncConflictDialogItem {
    SyncConflictDialogItem {
        id: conflict.record_id.clone(),
        title: personal_conflict_title(conflict, display),
        detail: personal_conflict_detail(conflict, display),
        default_strategy: default_personal_conflict_strategy(conflict.conflict_type),
        options: personal_resolution_options(),
    }
}

fn personal_conflict_title(
    conflict: &PersonalSyncConflict,
    display: &PersonalSyncConflictDisplayInfo,
) -> String {
    let name = display
        .remote
        .as_ref()
        .or(display.local.as_ref())
        .map(|record| record.name.as_str())
        .unwrap_or(conflict.record_id.as_str());
    format!("{} {}", data_type_label(conflict.data_type.as_str()), name)
}

fn personal_conflict_detail(
    conflict: &PersonalSyncConflict,
    display: &PersonalSyncConflictDisplayInfo,
) -> String {
    let mut parts = Vec::new();
    if let Some(local) = &display.local {
        parts.push(format!("本地: {}", record_display_text(local)));
    }
    if let Some(remote) = &display.remote {
        parts.push(format!("远端: {}", record_display_text(remote)));
    }
    parts.push(
        t!(
            "Home.personal_sync_conflict_type",
            conflict_type = conflict_type_label(conflict.conflict_type)
        )
        .to_string(),
    );
    parts.join(" | ")
}

fn record_display_text(record: &PersonalSyncRecordDisplay) -> String {
    match record.info.as_deref() {
        Some(info) if !info.is_empty() => format!("{} ({info})", record.name),
        _ => record.name.clone(),
    }
}

fn personal_resolution_options() -> Vec<SyncConflictResolutionOption> {
    vec![
        SyncConflictResolutionOption {
            strategy: ConflictResolution::UseCloud,
            label: SharedString::from(t!("Home.personal_sync_conflict_use_remote").to_string()),
        },
        SyncConflictResolutionOption {
            strategy: ConflictResolution::UseLocal,
            label: SharedString::from(t!("Home.personal_sync_conflict_use_local").to_string()),
        },
    ]
}

fn data_type_label(data_type: &str) -> String {
    match data_type {
        one_core::cloud_sync::data_type::WORKSPACE => {
            t!("Home.personal_sync_conflict_workspace").to_string()
        }
        _ => t!("Home.personal_sync_conflict_connection").to_string(),
    }
}

fn conflict_type_label(conflict_type: PersonalConflictType) -> String {
    match conflict_type {
        PersonalConflictType::BothModified => {
            t!("Home.personal_sync_conflict_both_modified").to_string()
        }
        PersonalConflictType::LocalDeletedRemoteModified => {
            t!("Home.personal_sync_conflict_local_deleted_remote_modified").to_string()
        }
        PersonalConflictType::LocalModifiedRemoteDeleted => {
            t!("Home.personal_sync_conflict_local_modified_remote_deleted").to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use one_core::cloud_sync::ConflictResolution;
    use one_core::cloud_sync::personal::{PersonalConflictType, PersonalSyncConflict};

    use crate::personal_sync_runtime::{
        PersonalSyncConflictDisplayInfo, PersonalSyncRecordDisplay,
    };

    use super::{
        default_personal_conflict_strategy, personal_conflict_count, personal_conflict_dialog_item,
    };

    #[test]
    fn personal_conflict_defaults_only_use_supported_resolution_strategies() {
        assert_eq!(
            ConflictResolution::UseCloud,
            default_personal_conflict_strategy(PersonalConflictType::BothModified)
        );
        assert_eq!(
            ConflictResolution::UseCloud,
            default_personal_conflict_strategy(PersonalConflictType::LocalDeletedRemoteModified)
        );
        assert_eq!(
            ConflictResolution::UseLocal,
            default_personal_conflict_strategy(PersonalConflictType::LocalModifiedRemoteDeleted)
        );
    }

    #[test]
    fn personal_conflict_count_matches_list_length() {
        let conflicts = vec![
            personal_conflict("cloud-2", PersonalConflictType::BothModified),
            personal_conflict("cloud-1", PersonalConflictType::LocalModifiedRemoteDeleted),
        ];

        assert_eq!(2, personal_conflict_count(&conflicts));
    }

    #[test]
    fn personal_conflict_dialog_item_prefers_readable_name_and_endpoint() {
        let conflict = personal_conflict("cloud-1", PersonalConflictType::BothModified);
        let display = PersonalSyncConflictDisplayInfo {
            local: Some(record("Local MySQL", "Database root@127.0.0.1:3306/ai_app")),
            remote: Some(record("Remote MySQL", "Database root@10.0.0.8:3306/ai_app")),
        };

        let item = personal_conflict_dialog_item(&conflict, &display);

        assert!(item.title.contains("Remote MySQL"));
        assert!(item.detail.contains("本地: Local MySQL"));
        assert!(item.detail.contains("127.0.0.1:3306"));
        assert!(item.detail.contains("远端: Remote MySQL"));
        assert!(item.detail.contains("10.0.0.8:3306"));
    }

    #[test]
    fn personal_conflict_dialog_item_falls_back_to_record_id() {
        let conflict = personal_conflict("cloud-1", PersonalConflictType::BothModified);
        let display = PersonalSyncConflictDisplayInfo {
            local: None,
            remote: None,
        };

        let item = personal_conflict_dialog_item(&conflict, &display);

        assert!(item.title.contains("cloud-1"));
        assert!(item.detail.contains("冲突类型"));
    }

    fn personal_conflict(
        record_id: &str,
        conflict_type: PersonalConflictType,
    ) -> PersonalSyncConflict {
        PersonalSyncConflict {
            backend_profile_id: "personal".to_string(),
            record_id: record_id.to_string(),
            data_type: one_core::cloud_sync::data_type::CONNECTION.to_string(),
            conflict_type,
            local_snapshot: None,
            remote_snapshot: None,
            detected_at: 100,
        }
    }

    fn record(name: &str, info: &str) -> PersonalSyncRecordDisplay {
        PersonalSyncRecordDisplay {
            name: name.to_string(),
            info: Some(info.to_string()),
        }
    }
}
