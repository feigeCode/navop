use extension_protocol::blob::WireBytes;
use extension_protocol::method;
use extension_protocol::mongodb::{
    MongoBsonDocument, MongoFindOptions, MongoFindParams, MongoFindResult,
};

#[test]
fn mongodb_contract_keeps_bson_as_binary_and_preserves_find_options() {
    let params = MongoFindParams {
        conn_id: 7,
        database: "app".into(),
        collection: "items".into(),
        filter: Some(MongoBsonDocument {
            bson: WireBytes::Base64("AAE=".into()),
        }),
        options: MongoFindOptions {
            limit: Some(50),
            skip: Some(10),
            sort: None,
            projection: None,
        },
    };
    let decoded: MongoFindParams =
        serde_json::from_value(serde_json::to_value(&params).unwrap()).unwrap();
    assert_eq!(params, decoded);

    let result = MongoFindResult {
        documents: vec![MongoBsonDocument {
            bson: WireBytes::Base64("AAEC".into()),
        }],
        documents_blob_id: None,
        document_count: 1,
        cursor_id: Some("cursor-1".into()),
    };
    assert_eq!(
        result,
        serde_json::from_value(serde_json::to_value(result.clone()).unwrap()).unwrap()
    );
    assert_eq!("mongodb/find", method::MONGODB_FIND);
}
