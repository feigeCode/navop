use extension_driver::{AsyncDriverConnection, AsyncNativeDriver, AsyncOpenedConnection};

fn assert_send_sync<T: Send + Sync>() {}

#[test]
fn async_driver_contract_is_exported_for_tokio_first_sidecars() {
    assert_send_sync::<AsyncOpenedConnection>();
    let _driver_trait: Option<&dyn AsyncNativeDriver> = None;
    let _connection_trait: Option<&dyn AsyncDriverConnection> = None;
}
