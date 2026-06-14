use crate::{
    ComponentHostState, ComponentRuntime, WasmRuntimeConfig, bindings,
    test_support::{MINIMAL_EXTENSION_COMPONENT, NoopDbHost, action_context},
};

#[test]
fn component_runtime_rejects_missing_component_path() {
    let err = match ComponentRuntime::from_file(
        "test",
        std::path::Path::new("/tmp/missing-component.wasm"),
        WasmRuntimeConfig::default(),
    ) {
        Ok(_) => panic!("missing wasm component should fail"),
        Err(err) => err,
    };

    assert!(err.to_string().contains("wasm component not found"));
}

#[test]
fn component_bindings_expose_extension_world() {
    let type_name = std::any::type_name::<bindings::Extension>();

    assert!(type_name.contains("Extension"));
}

#[test]
fn component_runtime_builds_db_linker() {
    let runtime = ComponentRuntime::for_tests("component").unwrap();
    let _linker = runtime.db_linker::<NoopDbHost>().unwrap();
}

#[test]
fn component_runtime_instantiates_extension_with_db_host() {
    let runtime =
        ComponentRuntime::from_wat_for_tests("component", MINIMAL_EXTENSION_COMPONENT).unwrap();
    let state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );

    let (mut store, extension) =
        futures::executor::block_on(runtime.instantiate_with_db(state)).unwrap();

    futures::executor::block_on(extension.call_activate(&mut store)).unwrap();
    futures::executor::block_on(extension.call_run_action(&mut store)).unwrap();
    futures::executor::block_on(extension.call_deactivate(&mut store)).unwrap();
}

#[test]
fn component_runtime_runs_action_with_action_context() {
    let runtime =
        ComponentRuntime::from_wat_for_tests("component", MINIMAL_EXTENSION_COMPONENT).unwrap();
    let state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );

    let views =
        futures::executor::block_on(runtime.run_action_with_db(state, action_context())).unwrap();

    assert!(views.is_empty());
}

#[test]
fn component_runtime_handles_view_action_with_form_values() {
    let runtime =
        ComponentRuntime::from_wat_for_tests("component", MINIMAL_EXTENSION_COMPONENT).unwrap();
    let state = ComponentHostState::new(
        "ext",
        NoopDbHost {
            connections: Vec::new(),
        },
    );
    let event = extension_component::ViewActionEvent {
        view_id: "full-search".to_string(),
        action_id: "run".to_string(),
        fields: vec![extension_component::FieldValue {
            id: "database".to_string(),
            value: "app".to_string(),
        }],
    };

    futures::executor::block_on(runtime.handle_view_action_with_db(state, action_context(), event))
        .unwrap();
}
