use gpui::App;
use public_mcp::tools::{
    RedisConnectionSnapshot, RedisConnectionSnapshotProvider, RedisToolProvider,
};
use redis_view::GlobalRedisState;
use std::sync::Arc;

pub(super) fn redis_tool_provider(cx: &App) -> RedisToolProvider {
    match cx.try_global::<GlobalRedisState>().cloned() {
        Some(state) => RedisToolProvider::new(Arc::new(RedisRuntimeSnapshots { state })),
        None => {
            tracing::warn!("Public MCP Redis toolset enabled before Redis state is initialized");
            RedisToolProvider::empty()
        }
    }
}

struct RedisRuntimeSnapshots {
    state: GlobalRedisState,
}

impl RedisConnectionSnapshotProvider for RedisRuntimeSnapshots {
    fn list_connections(&self) -> Vec<RedisConnectionSnapshot> {
        self.state
            .connection_ids()
            .into_iter()
            .map(|connection_id| RedisConnectionSnapshot { connection_id })
            .collect()
    }
}
