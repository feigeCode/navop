wasmtime::component::bindgen!({
    path: "../extension-api/wit",
    world: "extension",
    imports: { default: async | trappable },
    exports: { default: async },
    with: {
        "onet:extension/db/session": extension_component::DbSessionResource,
        "onet:extension/db/cursor": crate::component::ComponentCursorResource,
        "onet:extension/ui/progress": extension_component::UiProgressResource,
    },
});
