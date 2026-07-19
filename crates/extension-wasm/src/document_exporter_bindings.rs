wasmtime::component::bindgen!({
    path: "../extension-api/wit",
    world: "document-exporter",
    imports: { default: async | trappable },
    exports: { default: async },
});
