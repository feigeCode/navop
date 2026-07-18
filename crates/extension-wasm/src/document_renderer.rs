use crate::{WasmError, WasmResult, WasmRuntimeConfig, document_renderer_bindings};
use document_renderer_bindings::onet::extension::document_render as Wit;
use std::path::Path;
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

#[derive(Clone, Debug, PartialEq)]
pub struct DocumentRenderTheme {
    pub dark: bool,
    pub background: u32,
    pub foreground: u32,
    pub border: u32,
    pub muted: u32,
    pub accent: u32,
    pub danger: u32,
    pub font_family: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentRenderRequest {
    pub renderer: String,
    pub source: String,
    pub theme: DocumentRenderTheme,
    pub available_width: f32,
    pub scale_factor: f32,
}
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentRenderArtifact {
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub intrinsic_width: Option<f32>,
    pub intrinsic_height: Option<f32>,
}

pub struct DocumentRendererRuntime {
    id: String,
    engine: Engine,
    component: Component,
    config: WasmRuntimeConfig,
}
impl DocumentRendererRuntime {
    pub fn from_file(id: impl Into<String>, path: &Path) -> WasmResult<Self> {
        Self::from_file_with_config(id, path, WasmRuntimeConfig::default())
    }

    pub fn from_file_with_config(
        id: impl Into<String>,
        path: &Path,
        config: WasmRuntimeConfig,
    ) -> WasmResult<Self> {
        if !path.exists() {
            return Err(WasmError::ComponentNotFound(path.display().to_string()));
        }
        let engine = engine()?;
        let component = Component::from_file(&engine, path)
            .map_err(|e| WasmError::ComponentLoad(format!("{e:?}")))?;
        Ok(Self {
            id: id.into(),
            engine,
            component,
            config,
        })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
    pub async fn render(
        &self,
        request: DocumentRenderRequest,
    ) -> WasmResult<DocumentRenderArtifact> {
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        let mut store = Store::new(&self.engine, HostState::new(self.config.max_memory_mb));
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(self.config.fuel_per_call)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        let renderer = document_renderer_bindings::DocumentRenderer::instantiate_async(
            &mut store,
            &self.component,
            &linker,
        )
        .await
        .map_err(|e| WasmError::ComponentLoad(e.to_string()))?;
        let input = Wit::Request {
            renderer: request.renderer,
            source: request.source,
            theme: Wit::Theme {
                dark: request.theme.dark,
                background: request.theme.background,
                foreground: request.theme.foreground,
                border: request.theme.border,
                muted: request.theme.muted,
                accent: request.theme.accent,
                danger: request.theme.danger,
                font_family: request.theme.font_family,
            },
            available_width: request.available_width,
            scale_factor: request.scale_factor,
        };
        let output = renderer
            .call_render_document(&mut store, &input)
            .await
            .map_err(|e| WasmError::ComponentLoad(e.to_string()))?
            .map_err(WasmError::ComponentLoad)?;
        Ok(DocumentRenderArtifact {
            media_type: output.media_type,
            bytes: output.bytes,
            intrinsic_width: output.intrinsic_width,
            intrinsic_height: output.intrinsic_height,
        })
    }
}
struct HostState {
    wasi_ctx: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
}
impl HostState {
    fn new(max_memory_mb: u32) -> Self {
        Self {
            wasi_ctx: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
            limits: StoreLimitsBuilder::new()
                .memory_size(max_memory_mb as usize * 1024 * 1024)
                .instances(8)
                .tables(8)
                .memories(8)
                .build(),
        }
    }
}
impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}
fn engine() -> WasmResult<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(true);
    config.consume_fuel(true);
    Engine::new(&config).map_err(|e| WasmError::ComponentLoad(e.to_string()))
}
