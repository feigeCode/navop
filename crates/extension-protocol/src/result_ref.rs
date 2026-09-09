//! 通用调用结果引用。
//!
//! 小结果直接内联；大结果与持续事件分别复用 blob 和有界 event stream，
//! 避免宿主通过扫描任意 JSON 字段猜测资源句柄。

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{blob::BlobId, event_stream::EventStreamId};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResultRef {
    Inline { value: Value },
    Blob { id: BlobId },
    EventStream { id: EventStreamId },
}
