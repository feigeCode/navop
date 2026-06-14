pub type WasmResult<T> = Result<T, WasmError>;

#[derive(Debug, thiserror::Error)]
pub enum WasmError {
    #[error("wasm module not found: {0}")]
    ModuleNotFound(String),
    #[error("wasm component not found: {0}")]
    ComponentNotFound(String),
    #[error("load wasm component failed: {0}")]
    ComponentLoad(String),
    #[error("wasm function not found: {0}")]
    FunctionNotFound(String),
}
