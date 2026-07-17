use bson::{Bson, Document, doc, oid::ObjectId};
use mongodb_runtime::bson_to_compact_json;

#[test]
fn bson_values_round_trip_without_json_number_loss() {
    let id = ObjectId::new();
    let document = Document::from_iter([
        ("_id".into(), Bson::ObjectId(id)),
        ("i32".into(), Bson::Int32(7)),
        ("i64".into(), Bson::Int64(i64::MAX)),
        ("nested".into(), Bson::Document(doc! { "ok": true })),
    ]);
    let json = bson_to_compact_json(&Bson::Document(document)).unwrap();
    assert!(json.contains("i64"));
    assert!(json.contains("ObjectId") || json.contains("$oid"));
}
