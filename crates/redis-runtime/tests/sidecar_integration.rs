//! Real Redis sidecar integration. Set `NAVOP_REDIS_DRIVER_BIN` to the built
//! sidecar path; the test is skipped when no integration environment is set.

use extension_host::NativeDriverManifest;
use redis_runtime::{IpcRedisConnection, RedisConnectionConfig, RedisValue};

#[tokio::test]
async fn sidecar_round_trips_binary_values_and_pipeline() {
    let Some(binary) = std::env::var_os("NAVOP_REDIS_DRIVER_BIN") else {
        eprintln!("skipping Redis sidecar integration: NAVOP_REDIS_DRIVER_BIN is unset");
        return;
    };
    let mut manifest: NativeDriverManifest = serde_json::from_value(serde_json::json!({
        "id": "redis",
        "name": "Redis",
        "api": "redis",
        "protocol_version": "1.0",
        "entry": { "command": "redis-driver" },
        "transport": { "name": "redis.sock" },
        "methods": ["conn/open", "conn/close", "redis/command", "redis/pipeline"]
    }))
    .unwrap();
    manifest.entry.command = binary.to_string_lossy().into_owned();
    manifest.manifest_dir = std::env::temp_dir();

    let config = RedisConnectionConfig {
        host: "127.0.0.1".into(),
        port: 6380,
        timeout: 10,
        ..Default::default()
    };
    let connection = IpcRedisConnection::start(&manifest, config)
        .await
        .expect("Redis sidecar should connect to Redis 7");

    let binary_key = b"navop:binary".to_vec();
    let binary_value = vec![0, 1, 2, 0xff, 0xfe];
    connection
        .command_bytes(
            None,
            vec![b"SET".to_vec(), binary_key.clone(), binary_value.clone()],
        )
        .await
        .unwrap();
    let value = connection
        .command_bytes(None, vec![b"GET".to_vec(), binary_key])
        .await
        .unwrap();
    assert_eq!(RedisValue::Binary(binary_value), value);

    let values = connection
        .pipeline_bytes(
            None,
            vec![
                vec![b"SET".to_vec(), b"navop:pipeline".to_vec(), b"ok".to_vec()],
                vec![b"GET".to_vec(), b"navop:pipeline".to_vec()],
            ],
        )
        .await
        .unwrap();
    assert_eq!(2, values.len());
    assert_eq!(RedisValue::String("ok".into()), values[1]);
    connection.shutdown().await;
}
