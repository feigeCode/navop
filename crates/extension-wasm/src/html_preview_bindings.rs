wasmtime::component::bindgen!({
    path: "../extension-api/wit",
    world: "html-preview-transform",
    imports: { default: async | trappable },
    exports: { default: async },
});
