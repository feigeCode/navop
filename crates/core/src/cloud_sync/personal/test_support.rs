use crate::cloud_sync::models::CloudSyncData;

pub fn test_record(id: &str, data_type: &str, version: u32, checksum: &str) -> CloudSyncData {
    CloudSyncData {
        id: id.to_string(),
        owner_id: "personal-test-user".to_string(),
        team_id: None,
        data_type: data_type.to_string(),
        encrypted_data: "encrypted".to_string(),
        key_version: 1,
        checksum: checksum.to_string(),
        version,
        updated_at: 1_000,
        deleted_at: None,
    }
}
