//! 云同步「数据范围」扩展
//!
//! 把「用户可自选同步哪些数据类型」落到两条同步通道上。
//! 设计文档见 `docs/design/sync-scope-selection.md`。

pub mod cloud;
pub mod notes;
pub mod payloads;
pub mod personal;
pub mod settings_payload;
pub mod settings_store;
pub mod state;

pub use cloud::{ScopeSyncItem, ScopeSyncTypeHandler, synthetic_id};
pub use notes::{NOTE_MAX_CONTENT_BYTES, NoteSyncAdapter, NoteSyncItem, SharedNoteAdapter};
pub use payloads::*;
pub use personal::*;
pub use settings_payload::{
    SettingsFieldConflict, SettingsGroup, SettingsMerge, apply_payload, extract_payload,
    merge_three_way,
};
pub use settings_store::FileSettingsStore;
pub use state::{SyncItemState, SyncItemStateRepository, SyncTypeStatus, SyncTypeStatusRepository};
