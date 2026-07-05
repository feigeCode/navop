use std::path::Path;

use wasmtime::{
    Config, Engine, Store,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::{WasmError, WasmResult, html_preview_bindings};
use html_preview_bindings::onet::extension::html_preview as Wit;

pub struct HtmlPreviewTransformRuntime {
    id: String,
    engine: Engine,
    component: Component,
}

impl HtmlPreviewTransformRuntime {
    pub fn from_file(id: impl Into<String>, component_path: &Path) -> WasmResult<Self> {
        if !component_path.exists() {
            return Err(WasmError::ComponentNotFound(
                component_path.display().to_string(),
            ));
        }
        let engine = html_preview_engine()?;
        let component = Component::from_file(&engine, component_path)
            .map_err(|error| WasmError::ComponentLoad(format!("{error:?}")))?;
        Ok(Self {
            id: id.into(),
            engine,
            component,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    #[cfg(test)]
    pub fn from_wat_for_tests(id: impl Into<String>, wat: &str) -> WasmResult<Self> {
        let engine = html_preview_engine()?;
        let component = Component::new(&engine, wat)
            .map_err(|error| WasmError::ComponentLoad(format!("{error:?}")))?;
        Ok(Self {
            id: id.into(),
            engine,
            component,
        })
    }

    #[cfg(test)]
    pub fn from_bytes_for_tests(id: impl Into<String>, bytes: &[u8]) -> WasmResult<Self> {
        let engine = html_preview_engine()?;
        let component = Component::from_binary(&engine, bytes)
            .map_err(|error| WasmError::ComponentLoad(format!("{error:?}")))?;
        Ok(Self {
            id: id.into(),
            engine,
            component,
        })
    }

    pub async fn transform_html(
        &self,
        language: impl Into<String>,
        html: impl Into<String>,
    ) -> WasmResult<html_preview::HtmlPreviewTransformOutput> {
        let linker = Linker::new(&self.engine);
        let mut store = Store::new(&self.engine, HtmlPreviewHostState::new());
        let transform = html_preview_bindings::HtmlPreviewTransform::instantiate_async(
            &mut store,
            &self.component,
            &linker,
        )
        .await
        .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        let input = Wit::HtmlTransformInput {
            language: language.into(),
            html: html.into(),
        };
        let output = transform
            .call_transform_html(&mut store, &input)
            .await
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?
            .map_err(WasmError::ComponentLoad)?;
        Ok(host_transform_output(output))
    }
}

pub struct HtmlPreviewHostState {
    wasi_ctx: WasiCtx,
    table: ResourceTable,
}

impl HtmlPreviewHostState {
    fn new() -> Self {
        Self {
            wasi_ctx: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        }
    }
}

impl WasiView for HtmlPreviewHostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

fn host_transform_output(
    output: Wit::HtmlTransformOutput,
) -> html_preview::HtmlPreviewTransformOutput {
    html_preview::HtmlPreviewTransformOutput {
        html: output.html,
        assets: output
            .assets
            .into_iter()
            .map(|asset| html_preview::HtmlPreviewAsset {
                path: asset.path,
                url: asset.url,
            })
            .collect(),
    }
}

fn html_preview_engine() -> WasmResult<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(true);
    Engine::new(&config).map_err(|error| WasmError::ComponentLoad(error.to_string()))
}
