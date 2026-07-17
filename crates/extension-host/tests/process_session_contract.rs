use extension_host::{ProcessRpcSession, ProcessRpcSessionConfig};

#[test]
fn process_rpc_session_is_exported_as_a_generic_host_primitive() {
    assert!(std::any::type_name::<ProcessRpcSession>().contains("ProcessRpcSession"));
    assert!(std::any::type_name::<ProcessRpcSessionConfig>().contains("ProcessRpcSessionConfig"));
}
