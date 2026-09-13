//! Redis domain contracts and the embedded redis-rs backend without GPUI dependencies.

rust_i18n::i18n!("../redis_view/locales", fallback = "zh-CN");

pub mod connection;
pub mod pubsub;
pub mod types;

mod builtin;
mod builtin_pubsub;

#[doc(hidden)]
pub fn parse_command_args_for_test(command: &str) -> Vec<String> {
    builtin::parse_command_args_for_test(command)
}

pub use builtin::RedisConnectionImpl;
pub use connection::RedisConnection;
pub use pubsub::{PubSubMessage, PubSubMessageKind, RedisPubSubHandle, SubscriptionCommand};
pub use types::*;
