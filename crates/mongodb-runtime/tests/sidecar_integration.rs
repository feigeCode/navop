//! Real MongoDB sidecar integration. Set `NAVOP_MONGO_DRIVER_BIN` to run.

use bson::doc;
use extension_host::NativeDriverManifest;
use mongodb_runtime::{
    IpcMongoConnection, MongoConnection, MongoConnectionConfig, MongoFindOptions,
};

#[tokio::test]
async fn sidecar_round_trips_bson_commands_and_find() {
    let Some(binary) = std::env::var_os("NAVOP_MONGO_DRIVER_BIN") else {
        eprintln!("skipping MongoDB sidecar integration: NAVOP_MONGO_DRIVER_BIN is unset");
        return;
    };
    let mut manifest: NativeDriverManifest = serde_json::from_str(include_str!(
        "../../../drivers/mongodb-driver/packages/mongodb-modern/driver.json"
    ))
    .unwrap();
    manifest.entry.command = binary.to_string_lossy().into_owned();
    manifest.manifest_dir = std::env::temp_dir();

    let config = MongoConnectionConfig {
        id: "it".into(),
        name: "MongoDB 7 integration".into(),
        connection_string: "mongodb://127.0.0.1:27018".into(),
        direct_host: "127.0.0.1".into(),
        direct_port: 27018,
        ssh_tunnel: None,
    };
    let mut connection = IpcMongoConnection::with_manifest(manifest, config);
    connection.connect().await.unwrap();
    connection.ping().await.unwrap();
    connection
        .create_collection("navop_it", "documents")
        .await
        .unwrap();
    connection
        .insert_document("navop_it", "documents", doc! { "kind": "ipc", "n": 7 })
        .await
        .unwrap();
    let documents = connection
        .find_documents(
            "navop_it",
            "documents",
            Some(doc! { "kind": "ipc" }),
            MongoFindOptions::default(),
        )
        .await
        .unwrap();
    assert!(
        documents
            .iter()
            .any(|document| document.get_i32("n") == Ok(7))
    );
    connection.drop_database("navop_it").await.unwrap();
    connection.disconnect().await.unwrap();
}
