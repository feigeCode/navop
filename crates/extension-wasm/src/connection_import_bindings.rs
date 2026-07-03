wasmtime::component::bindgen!({
    path: "../extension-api/wit",
    world: "connection-importer",
    imports: { default: async | trappable },
    exports: { default: async },
});
