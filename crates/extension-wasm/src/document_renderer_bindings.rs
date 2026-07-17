wasmtime::component::bindgen!({
    path: "../extension-api/wit",
    world: "document-renderer",
    imports: { default: async | trappable },
    exports: { default: async },
});
