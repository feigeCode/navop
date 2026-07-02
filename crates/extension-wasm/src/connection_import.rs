use extension_component::{CandidateFileAccess, ExtensionConnectionImportHost, PermissionSet};
use std::path::Path;
use wasmtime::{
    Config, Engine, Store,
    component::{Component, HasSelf, Linker},
};

use crate::{
    WasmError, WasmResult,
    connection_import_bindings::{
        self,
        onet::extension::{connection_import as Wit, connection_import_host as Host},
    },
};

pub struct ConnectionImportComponentRuntime {
    id: String,
    engine: Engine,
    component: Component,
}

impl ConnectionImportComponentRuntime {
    pub fn from_file(id: impl Into<String>, component_path: &Path) -> WasmResult<Self> {
        if !component_path.exists() {
            return Err(WasmError::ComponentNotFound(
                component_path.display().to_string(),
            ));
        }
        let engine = connection_import_engine()?;
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
        let engine = connection_import_engine()?;
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
        let engine = connection_import_engine()?;
        let component = Component::from_binary(&engine, bytes)
            .map_err(|error| WasmError::ComponentLoad(format!("{error:?}")))?;
        Ok(Self {
            id: id.into(),
            engine,
            component,
        })
    }

    pub async fn descriptor<H>(
        &self,
        state: ConnectionImportHostState<H>,
    ) -> WasmResult<connection_import_protocol::ImporterDescriptor>
    where
        H: ExtensionConnectionImportHost + Send + Sync + 'static,
    {
        let (mut store, importer) = self.instantiate(state).await?;
        let json = importer
            .call_descriptor(&mut store)
            .await
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        decode_json(&json)
    }

    pub async fn scan<H>(
        &self,
        state: ConnectionImportHostState<H>,
    ) -> WasmResult<connection_import_protocol::ImportScanReport>
    where
        H: ExtensionConnectionImportHost + Send + Sync + 'static,
    {
        let (mut store, importer) = self.instantiate(state).await?;
        let json = importer
            .call_scan(&mut store)
            .await
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        decode_json(&json)
    }

    pub async fn preview<H>(
        &self,
        state: ConnectionImportHostState<H>,
        include_passwords: bool,
    ) -> WasmResult<Vec<connection_import_protocol::ImportRecord>>
    where
        H: ExtensionConnectionImportHost + Send + Sync + 'static,
    {
        let (mut store, importer) = self.instantiate(state).await?;
        let options = Wit::ImportOptions { include_passwords };
        let json = importer
            .call_preview(&mut store, options)
            .await
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        decode_json(&json)
    }

    async fn instantiate<H>(
        &self,
        state: ConnectionImportHostState<H>,
    ) -> WasmResult<(
        Store<ConnectionImportHostState<H>>,
        connection_import_bindings::ConnectionImporter,
    )>
    where
        H: ExtensionConnectionImportHost + Send + Sync + 'static,
    {
        let mut linker = Linker::new(&self.engine);
        Host::add_to_linker::<_, HasSelf<_>>(&mut linker, |state| state)
            .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        let mut store = Store::new(&self.engine, state);
        let importer = connection_import_bindings::ConnectionImporter::instantiate_async(
            &mut store,
            &self.component,
            &linker,
        )
        .await
        .map_err(|error| WasmError::ComponentLoad(error.to_string()))?;
        Ok((store, importer))
    }
}

pub struct ConnectionImportHostState<H>
where
    H: ExtensionConnectionImportHost,
{
    extension_id: String,
    importer_id: String,
    host: H,
    permissions: PermissionSet,
}

fn connection_import_engine() -> WasmResult<Engine> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(true);
    Engine::new(&config).map_err(|error| WasmError::ComponentLoad(error.to_string()))
}

fn decode_json<T>(json: &str) -> WasmResult<T>
where
    T: serde::de::DeserializeOwned,
{
    serde_json::from_str(json).map_err(|error| WasmError::ProtocolDecode(error.to_string()))
}

impl<H> ConnectionImportHostState<H>
where
    H: ExtensionConnectionImportHost,
{
    pub fn new(
        extension_id: impl Into<String>,
        importer_id: impl Into<String>,
        host: H,
        permissions: PermissionSet,
    ) -> Self {
        Self {
            extension_id: extension_id.into(),
            importer_id: importer_id.into(),
            host,
            permissions,
        }
    }

    pub fn extension_id(&self) -> &str {
        &self.extension_id
    }

    pub fn importer_id(&self) -> &str {
        &self.importer_id
    }

    fn candidate_access(&self) -> CandidateFileAccess {
        CandidateFileAccess::new(
            self.host.list_candidate_files(&self.importer_id),
            self.permissions.clone(),
        )
    }
}

impl<H> Host::Host for ConnectionImportHostState<H>
where
    H: ExtensionConnectionImportHost + Send + Sync,
{
    async fn current_platform(&mut self) -> wasmtime::Result<Wit::Platform> {
        Ok(wit_platform(self.host.current_platform()))
    }

    async fn list_candidate_files(
        &mut self,
        importer_id: String,
    ) -> wasmtime::Result<Vec<Wit::CandidateFile>> {
        Ok(self
            .host
            .list_candidate_files(&importer_id)
            .into_iter()
            .map(wit_candidate_file)
            .collect())
    }

    async fn read_file(
        &mut self,
        candidate_id: String,
    ) -> wasmtime::Result<Result<Vec<u8>, Wit::HostError>> {
        if let Err(error) = self.candidate_access().candidate(&candidate_id) {
            return Ok(Err(wit_host_error(error)));
        }
        Ok(self.host.read_file(&candidate_id).map_err(wit_host_error))
    }

    async fn read_directory(
        &mut self,
        candidate_id: String,
    ) -> wasmtime::Result<Result<Vec<Wit::DirectoryEntry>, Wit::HostError>> {
        if let Err(error) = self.candidate_access().candidate(&candidate_id) {
            return Ok(Err(wit_host_error(error)));
        }
        Ok(self
            .host
            .read_directory(&candidate_id)
            .map(|entries| entries.into_iter().map(wit_directory_entry).collect())
            .map_err(wit_host_error))
    }

    async fn read_secret(
        &mut self,
        query: Wit::SecretQuery,
    ) -> wasmtime::Result<Wit::SecretResult> {
        Ok(wit_secret_result(self.host.read_secret(
            connection_import_protocol::SecretQuery {
                service: query.service,
                account: query.account,
            },
        )))
    }

    async fn log(&mut self, level: String, message: String) -> wasmtime::Result<()> {
        self.host.log(&level, &message);
        Ok(())
    }
}

fn wit_platform(platform: connection_import_protocol::Platform) -> Wit::Platform {
    match platform {
        connection_import_protocol::Platform::Macos => Wit::Platform::Macos,
        connection_import_protocol::Platform::Windows => Wit::Platform::Windows,
        connection_import_protocol::Platform::Linux => Wit::Platform::Linux,
    }
}

fn wit_candidate_file(file: connection_import_protocol::CandidateFile) -> Wit::CandidateFile {
    Wit::CandidateFile {
        id: file.id,
        platform: file.platform.map(wit_platform),
        path: file.path,
    }
}

fn wit_directory_entry(entry: connection_import_protocol::DirectoryEntry) -> Wit::DirectoryEntry {
    Wit::DirectoryEntry {
        candidate_id: entry.candidate_id,
        name: entry.name,
        is_dir: entry.is_dir,
    }
}

fn wit_secret_result(result: connection_import_protocol::SecretResult) -> Wit::SecretResult {
    match result {
        connection_import_protocol::SecretResult::Included { value } => {
            Wit::SecretResult::Included(value)
        }
        connection_import_protocol::SecretResult::Missing => Wit::SecretResult::Missing,
        connection_import_protocol::SecretResult::PermissionDenied => {
            Wit::SecretResult::PermissionDenied
        }
        connection_import_protocol::SecretResult::Unsupported => Wit::SecretResult::Unsupported,
    }
}

fn wit_host_error(error: connection_import_protocol::HostAccessError) -> Wit::HostError {
    let code = match error {
        connection_import_protocol::HostAccessError::UndeclaredCandidate(_) => {
            "undeclared_candidate"
        }
        connection_import_protocol::HostAccessError::PermissionDenied(_) => "permission_denied",
        connection_import_protocol::HostAccessError::NotFound(_) => "not_found",
        connection_import_protocol::HostAccessError::Io(_) => "io",
    };
    Wit::HostError {
        code: code.to_string(),
        message: error.to_string(),
    }
}
