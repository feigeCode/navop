//! Redis connection compatibility exports for the UI crate.

#[cfg(feature = "builtin-redis")]
pub use redis_runtime::BuiltinRedisConnection as RedisConnectionImpl;
pub use redis_runtime::{RedisConnection, RedisPubSubHandle};
