use std::path::Path;

use extension_component::{
    ActionContext, DbSessionResource, ExtensionDbHost, FieldValue, ViewActionEvent, ViewSpec,
};
use wasmtime::{
    Config, Engine, Store,
    component::{Component, HasSelf, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

use crate::{WasmError, WasmResult, WasmRuntimeConfig, bindings};
use bindings::onet::extension::db as Db;
use bindings::onet::extension::task as Task;
use bindings::onet::extension::ui as Ui;

pub struct ComponentRuntime {
    id: String,
    engine: Engine,
    component: Component,
    config: WasmRuntimeConfig,
}

impl ComponentRuntime {
    pub fn from_file(
        id: impl Into<String>,
        component_path: &Path,
        config: WasmRuntimeConfig,
    ) -> WasmResult<Self> {
        if !component_path.exists() {
            return Err(WasmError::ComponentNotFound(
                component_path.display().to_string(),
            ));
        }

        let engine = component_engine()?;
        let component = Component::from_file(&engine, component_path)
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

    pub fn component(&self) -> &Component {
        &self.component
    }

    pub fn config(&self) -> &WasmRuntimeConfig {
        &self.config
    }

    pub fn linker<T>(&self) -> Linker<T> {
        Linker::new(&self.engine)
    }

    #[cfg(test)]
    pub fn for_tests(id: impl Into<String>) -> WasmResult<Self> {
        Self::from_wat_for_tests(id, "(component)")
    }

    #[cfg(test)]
    pub fn from_wat_for_tests(id: impl Into<String>, wat: &str) -> WasmResult<Self> {
        let engine = component_engine()?;
        let component = Component::new(&engine, wat)
            .map_err(|error| WasmError::ComponentLoad(format!("{error:?}")))?;
        Ok(Self {
            id: id.into(),
            engine,
            component,
            config: WasmRuntimeConfig::default(),
        })
    }

    pub fn db_linker<H>(&self) -> WasmResult<Linker<ComponentHostState<H>>>
    where
        H: ExtensionDbHost + Send + Sync + 'static,
    {
        let mut linker = Linker::new(&self.engine);
        Db::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        Ui::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        Task::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        Ok(linker)
    }

    pub async fn instantiate_with_db<H>(
        &self,
        state: ComponentHostState<H>,
    ) -> WasmResult<(Store<ComponentHostState<H>>, bindings::Extension)>
    where
        H: ExtensionDbHost + Send + Sync + 'static,
    {
        let linker = self.db_linker()?;
        let mut store = Store::new(&self.engine, state);
        store
            .set_fuel(self.config.fuel_per_call)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        let extension =
            bindings::Extension::instantiate_async(&mut store, &self.component, &linker)
                .await
                .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        Ok((store, extension))
    }

    pub async fn run_action_with_db<H>(
        &self,
        mut state: ComponentHostState<H>,
        context: ActionContext,
    ) -> WasmResult<Vec<ViewSpec>>
    where
        H: ExtensionDbHost + Send + Sync + 'static,
    {
        state.set_action_context(context);
        let (mut store, extension) = self.instantiate_with_db(state).await?;
        extension
            .call_run_action(&mut store)
            .await
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        Ok(store.data().opened_views().to_vec())
    }

    pub async fn handle_view_action_with_db<H>(
        &self,
        mut state: ComponentHostState<H>,
        context: ActionContext,
        event: ViewActionEvent,
    ) -> WasmResult<()>
    where
        H: ExtensionDbHost + Send + Sync + 'static,
    {
        state.set_action_context(context);
        let (mut store, extension) = self.instantiate_with_db(state).await?;
        let event = wit_view_action_event(event);
        extension
            .call_handle_view_action(&mut store, &event)
            .await
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        Ok(())
    }
}

pub struct ComponentCursorResource;

pub struct ComponentHostState<H>
where
    H: ExtensionDbHost,
{
    extension_id: String,
    pub(crate) db_host: H,
    action_context: Option<ActionContext>,
    opened_views: Vec<ViewSpec>,
    wasi_ctx: WasiCtx,
    pub(crate) table: ResourceTable,
}

impl<H> ComponentHostState<H>
where
    H: ExtensionDbHost,
{
    pub fn new(extension_id: impl Into<String>, db_host: H) -> Self {
        Self {
            extension_id: extension_id.into(),
            db_host,
            action_context: None,
            opened_views: Vec::new(),
            wasi_ctx: WasiCtxBuilder::new().build(),
            table: ResourceTable::new(),
        }
    }

    pub fn extension_id(&self) -> &str {
        &self.extension_id
    }

    pub fn db_host(&self) -> &H {
        &self.db_host
    }

    pub fn db_host_mut(&mut self) -> &mut H {
        &mut self.db_host
    }

    pub fn set_action_context(&mut self, context: ActionContext) {
        self.action_context = Some(context);
    }

    pub fn action_context(&self) -> Option<&ActionContext> {
        self.action_context.as_ref()
    }

    pub fn opened_views(&self) -> &[ViewSpec] {
        &self.opened_views
    }

    pub(crate) fn push_opened_view(&mut self, view: ViewSpec) {
        self.opened_views.push(view);
    }

    pub fn table(&self) -> &ResourceTable {
        &self.table
    }

    pub fn table_mut(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl<H> WasiView for ComponentHostState<H>
where
    H: ExtensionDbHost + Send,
{
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

impl<H> ComponentHostState<H>
where
    H: ExtensionDbHost,
{
    pub(crate) fn session(
        &self,
        resource: &wasmtime::component::Resource<DbSessionResource>,
    ) -> wasmtime::Result<&DbSessionResource> {
        self.table.get(resource).map_err(table_error)
    }

    pub(crate) fn session_mut(
        &mut self,
        resource: &wasmtime::component::Resource<DbSessionResource>,
    ) -> wasmtime::Result<&mut DbSessionResource> {
        self.table.get_mut(resource).map_err(table_error)
    }
}

pub(crate) fn table_error(error: impl ToString) -> wasmtime::Error {
    wasmtime::Error::msg(error.to_string())
}

fn component_engine() -> WasmResult<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(true);
    config.consume_fuel(true);
    Engine::new(&config).map_err(|error| WasmError::ComponentLoad(error.to_string()))
}

fn wit_view_action_event(event: ViewActionEvent) -> Ui::ViewActionEvent {
    Ui::ViewActionEvent {
        view_id: event.view_id,
        action_id: event.action_id,
        fields: event.fields.into_iter().map(wit_field_value).collect(),
    }
}

fn wit_field_value(value: FieldValue) -> Ui::FieldValue {
    Ui::FieldValue {
        id: value.id,
        value: value.value,
    }
}
