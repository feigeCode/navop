use std::collections::{HashMap, HashSet};

use crate::cloud_sync::models::CloudSyncData;

use super::PersonalConflictType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalSyncItemSnapshot {
    pub local_id: String,
    pub cloud_id: Option<String>,
    pub data_type: String,
    pub updated_at: i64,
    pub last_synced_at: Option<i64>,
    pub checksum: String,
    pub team_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonalSyncRecordConflict {
    pub local_id: String,
    pub cloud_id: String,
    pub conflict_type: PersonalConflictType,
}

#[derive(Debug, Default, Clone)]
pub struct PersonalSyncPlan {
    pub to_upload: Vec<PersonalSyncItemSnapshot>,
    pub to_update_cloud: Vec<(PersonalSyncItemSnapshot, CloudSyncData)>,
    pub to_update_local: Vec<(CloudSyncData, PersonalSyncItemSnapshot)>,
    pub to_download: Vec<CloudSyncData>,
    pub to_mark_synced: Vec<String>,
    pub conflicts: Vec<PersonalSyncRecordConflict>,
}

impl PersonalSyncPlan {
    pub fn is_empty(&self) -> bool {
        self.to_upload.is_empty()
            && self.to_update_cloud.is_empty()
            && self.to_update_local.is_empty()
            && self.to_download.is_empty()
            && self.to_mark_synced.is_empty()
            && self.conflicts.is_empty()
    }

    pub fn to_upload_local_ids(&self) -> Vec<&str> {
        self.to_upload
            .iter()
            .map(|item| item.local_id.as_str())
            .collect()
    }

    pub fn to_mark_synced_cloud_ids(&self) -> Vec<&str> {
        self.to_mark_synced.iter().map(String::as_str).collect()
    }
}

#[derive(Debug, Default)]
pub struct PersonalSyncPlanner;

impl PersonalSyncPlanner {
    pub fn new() -> Self {
        Self
    }

    pub fn plan(
        &self,
        local_items: &[PersonalSyncItemSnapshot],
        remote_records: &[CloudSyncData],
        paused_record_ids: &HashSet<String>,
    ) -> PersonalSyncPlan {
        let remote_by_cloud_id = remote_records
            .iter()
            .map(|record| (record.id.as_str(), record))
            .collect::<HashMap<_, _>>();
        let mut plan = PersonalSyncPlan::default();
        let mut local_cloud_ids = HashSet::new();

        for item in local_items.iter().filter(|item| item.team_id.is_none()) {
            self.plan_local_item(item, &remote_by_cloud_id, paused_record_ids, &mut plan);
            if let Some(cloud_id) = &item.cloud_id {
                local_cloud_ids.insert(cloud_id.as_str());
            }
        }

        for record in remote_records {
            if !local_cloud_ids.contains(record.id.as_str())
                && !paused_record_ids.contains(record.id.as_str())
            {
                plan.to_download.push(record.clone());
            }
        }

        plan
    }

    fn plan_local_item(
        &self,
        item: &PersonalSyncItemSnapshot,
        remote_by_cloud_id: &HashMap<&str, &CloudSyncData>,
        paused_record_ids: &HashSet<String>,
        plan: &mut PersonalSyncPlan,
    ) {
        let Some(cloud_id) = &item.cloud_id else {
            plan.to_upload.push(item.clone());
            return;
        };
        if paused_record_ids.contains(cloud_id) {
            return;
        }

        match remote_by_cloud_id.get(cloud_id.as_str()) {
            Some(remote) => plan_existing_item(item, remote, plan),
            None => plan.to_upload.push(item.clone()),
        }
    }
}

fn plan_existing_item(
    item: &PersonalSyncItemSnapshot,
    remote: &CloudSyncData,
    plan: &mut PersonalSyncPlan,
) {
    if item.checksum == remote.checksum {
        plan.to_mark_synced.push(remote.id.clone());
        return;
    }

    let last_synced = item.last_synced_at.unwrap_or(0);
    let remote_updated = remote.updated_at / 1000;
    let local_changed = item.updated_at > last_synced;
    let remote_changed = remote_updated > last_synced;

    match (local_changed, remote_changed) {
        (true, true) => plan.conflicts.push(PersonalSyncRecordConflict {
            local_id: item.local_id.clone(),
            cloud_id: remote.id.clone(),
            conflict_type: PersonalConflictType::BothModified,
        }),
        (true, false) => plan.to_update_cloud.push((item.clone(), remote.clone())),
        (false, true) => plan.to_update_local.push((remote.clone(), item.clone())),
        (false, false) => {}
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::cloud_sync::models::data_type;
    use crate::cloud_sync::personal::test_support::test_record;
    use crate::cloud_sync::personal::{
        PersonalConflictType, PersonalSyncItemSnapshot, PersonalSyncPlanner,
    };

    #[test]
    fn planner_uploads_new_local_personal_record() {
        let local = vec![local_item("local-1", None, 200, None)];
        let remote = Vec::new();

        let plan = PersonalSyncPlanner::new().plan(&local, &remote, &HashSet::new());

        assert_eq!(vec!["local-1"], plan.to_upload_local_ids());
        assert!(plan.conflicts.is_empty());
    }

    #[test]
    fn planner_skips_team_records() {
        let local = vec![local_item("local-1", None, 200, Some("team-1"))];
        let remote = Vec::new();

        let plan = PersonalSyncPlanner::new().plan(&local, &remote, &HashSet::new());

        assert!(plan.is_empty());
    }

    #[test]
    fn planner_conflicts_when_both_sides_modified_with_different_checksum() {
        let local = vec![local_item_with_sync(
            "local-1",
            "cloud-1",
            300,
            100,
            "local-checksum",
        )];
        let mut remote = test_record("cloud-1", data_type::CONNECTION, 2, "remote-checksum");
        remote.updated_at = 300_000;

        let plan = PersonalSyncPlanner::new().plan(&local, &[remote], &HashSet::new());

        assert_eq!(
            PersonalConflictType::BothModified,
            plan.conflicts[0].conflict_type
        );
    }

    #[test]
    fn planner_treats_matching_checksum_as_synced_even_with_different_timestamps() {
        let local = vec![local_item_with_sync("local-1", "cloud-1", 300, 100, "same")];
        let mut remote = test_record("cloud-1", data_type::CONNECTION, 2, "same");
        remote.updated_at = 500_000;

        let plan = PersonalSyncPlanner::new().plan(&local, &[remote], &HashSet::new());

        assert!(plan.conflicts.is_empty());
        assert_eq!(vec!["cloud-1"], plan.to_mark_synced_cloud_ids());
    }

    fn local_item(
        local_id: &str,
        cloud_id: Option<&str>,
        updated_at: i64,
        team_id: Option<&str>,
    ) -> PersonalSyncItemSnapshot {
        PersonalSyncItemSnapshot {
            local_id: local_id.to_string(),
            cloud_id: cloud_id.map(str::to_string),
            data_type: data_type::CONNECTION.to_string(),
            updated_at,
            last_synced_at: None,
            checksum: String::new(),
            team_id: team_id.map(str::to_string),
        }
    }

    fn local_item_with_sync(
        local_id: &str,
        cloud_id: &str,
        updated_at: i64,
        last_synced_at: i64,
        checksum: &str,
    ) -> PersonalSyncItemSnapshot {
        PersonalSyncItemSnapshot {
            local_id: local_id.to_string(),
            cloud_id: Some(cloud_id.to_string()),
            data_type: data_type::CONNECTION.to_string(),
            updated_at,
            last_synced_at: Some(last_synced_at),
            checksum: checksum.to_string(),
            team_id: None,
        }
    }
}
