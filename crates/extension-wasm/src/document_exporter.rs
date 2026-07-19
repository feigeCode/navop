use crate::{WasmError, WasmResult, WasmRuntimeConfig, document_exporter_bindings};
use document_exporter_bindings::onet::extension::document_export as Wit;
use std::path::Path;
use std::time::Duration;
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

#[derive(Clone, Debug, PartialEq)]
pub struct DocumentExportTheme {
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
pub struct DocumentExportRequest {
    pub exporter: String,
    pub format: String,
    pub title: String,
    pub source: String,
    pub theme: DocumentExportTheme,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DocumentExportArtifact {
    pub media_type: String,
    pub extension: String,
    pub bytes: Vec<u8>,
}

pub struct DocumentExporterRuntime {
    id: String,
    engine: Engine,
    component: Component,
    config: WasmRuntimeConfig,
}

impl std::fmt::Debug for DocumentExporterRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DocumentExporterRuntime")
            .field("id", &self.id)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl DocumentExporterRuntime {
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
            .map_err(|error| WasmError::ComponentLoad(format!("{error:?}")))?;
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

    pub async fn export(
        &self,
        request: DocumentExportRequest,
    ) -> WasmResult<DocumentExportArtifact> {
        let mut linker = Linker::new(&self.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        let mut store = Store::new(&self.engine, HostState::new(self.config.max_memory_mb));
        store.limiter(|state| &mut state.limits);
        store.set_epoch_deadline(1);
        store
            .set_fuel(self.config.fuel_per_call)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        let timeout_engine = self.engine.clone();
        let timeout = Duration::from_millis(self.config.timeout_ms.max(1));
        std::thread::spawn(move || {
            std::thread::sleep(timeout);
            timeout_engine.increment_epoch();
        });
        let exporter = document_exporter_bindings::DocumentExporter::instantiate_async(
            &mut store,
            &self.component,
            &linker,
        )
        .await
        .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        let input = Wit::Request {
            exporter: request.exporter,
            format: request.format,
            title: request.title,
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
        };
        let output = exporter
            .call_export_document(&mut store, &input)
            .await
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?
            .map_err(WasmError::ComponentLoad)?;
        Ok(DocumentExportArtifact {
            media_type: output.media_type,
            extension: output.extension,
            bytes: output.bytes,
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
    config.epoch_interruption(true);
    if let Ok(cache) = wasmtime::Cache::from_file(None) {
        config.cache(Some(cache));
    }
    Engine::new(&config).map_err(|error| WasmError::ComponentLoad(error.to_string()))
}
