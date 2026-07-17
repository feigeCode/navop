use extension_protocol::blob::WireBytes;
use extension_protocol::method;
use extension_protocol::redis::{
    RedisCommandParams, RedisCommandResult, RedisPipelineParams, RedisRespValue,
};

#[test]
fn redis_binary_command_and_response_round_trip_without_utf8_assumptions() {
    let params = RedisCommandParams {
        conn_id: 7,
        database: Some(2),
        args: vec![
            WireBytes::Utf8("SET".into()),
            WireBytes::Base64("AP8=".into()),
            WireBytes::Base64("gAE=".into()),
        ],
    };
    let encoded = serde_json::to_value(&params).unwrap();
    let decoded: RedisCommandParams = serde_json::from_value(encoded).unwrap();
    assert_eq!(params.args, decoded.args);

    let result = RedisCommandResult {
        value: RedisRespValue::Array(vec![
            RedisRespValue::Integer(1),
            RedisRespValue::Bytes(WireBytes::Base64("AP8=".into())),
            RedisRespValue::Nil,
        ]),
    };
    let encoded = serde_json::to_value(&result).unwrap();
    let decoded: RedisCommandResult = serde_json::from_value(encoded).unwrap();
    assert_eq!(result.value, decoded.value);
}

#[test]
fn redis_pipeline_is_a_bounded_list_of_binary_commands() {
    let pipeline = RedisPipelineParams {
        conn_id: 9,
        database: None,
        commands: vec![vec![WireBytes::Utf8("PING".into())]],
    };
    assert_eq!(1, pipeline.commands.len());
    assert_eq!("redis/command", method::REDIS_COMMAND);
    assert_eq!("redis/pipeline", method::REDIS_PIPELINE);
    assert!(method::is_known(method::REDIS_PUBSUB_OPEN));
}
