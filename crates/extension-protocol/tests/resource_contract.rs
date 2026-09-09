use extension_protocol::{
    method,
    resource::{
        ResourceCloseParams, ResourceInvokeParams, ResourceInvokeResult, ResourceOpenParams,
        ResourceOpenResult, ResourcePingParams,
    },
    result_ref::ResultRef,
};

#[test]
fn resource_methods_are_stable_public_contracts() {
    assert_eq!("resource/open", method::RESOURCE_OPEN);
    assert_eq!("resource/close", method::RESOURCE_CLOSE);
    assert_eq!("resource/ping", method::RESOURCE_PING);
    assert_eq!("resource/invoke", method::RESOURCE_INVOKE);
    assert!(method::is_known(method::RESOURCE_OPEN));
    assert!(method::is_known(method::RESOURCE_INVOKE));
}

#[test]
fn resource_lifecycle_and_invoke_payloads_round_trip() {
    let open = ResourceOpenParams {
        resource_type: "kafka".into(),
        config: serde_json::json!({
            "brokers": ["localhost:9092"],
            "security_protocol": "plaintext"
        }),
        metadata: Some(serde_json::json!({"profile": "local"})),
    };
    let open_json = serde_json::to_value(&open).unwrap();
    assert_eq!("kafka", open_json["resource_type"]);
    assert_eq!(
        open,
        serde_json::from_value::<ResourceOpenParams>(open_json).unwrap()
    );

    let opened = ResourceOpenResult {
        resource_id: "resource-1".into(),
        capabilities: vec!["kafka/topic/list".into(), "kafka/topic/consume".into()],
        metadata: None,
    };
    assert_eq!(
        opened,
        serde_json::from_value(serde_json::to_value(&opened).unwrap()).unwrap()
    );

    let invoke = ResourceInvokeParams {
        resource_id: "resource-1".into(),
        method: "kafka/topic/list".into(),
        params: serde_json::json!({"include_internal": false}),
    };
    assert_eq!(
        invoke,
        serde_json::from_value(serde_json::to_value(&invoke).unwrap()).unwrap()
    );

    let result = ResourceInvokeResult {
        result: ResultRef::Inline {
            value: serde_json::json!({"topics": ["orders"]}),
        },
    };
    assert_eq!(
        "inline",
        serde_json::to_value(result).unwrap()["result"]["kind"]
    );

    for value in [
        serde_json::to_value(ResourcePingParams {
            resource_id: "resource-1".into(),
        })
        .unwrap(),
        serde_json::to_value(ResourceCloseParams {
            resource_id: "resource-1".into(),
        })
        .unwrap(),
    ] {
        assert_eq!("resource-1", value["resource_id"]);
    }
}
