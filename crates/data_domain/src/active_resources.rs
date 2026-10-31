//! 数据领域的活动资源：把运行中的 Redis 连接交给公共 MCP 资源池。

use domain_api::{ActiveResourceIds, ActiveResourceProvider};
use gpui::App;

pub(crate) struct RedisActiveResourceProvider;

impl ActiveResourceProvider for RedisActiveResourceProvider {
    fn active_resources(&self, cx: &App) -> Vec<ActiveResourceIds> {
        let Some(state) = cx.try_global::<redis_view::GlobalRedisState>().cloned() else {
            return Vec::new();
        };
        vec![ActiveResourceIds {
            kind: "redis",
            connection_ids: state.connection_ids(),
        }]
    }
}
